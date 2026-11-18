//! Apple-Health-Werte aus den Tagesdateien der Notch (`Dokumente\Notch\Health\JJJJ-MM-TT.jsonl`) in helio.db.
//! Die Notch hängt nur an. Helio merkt sich je Datei die gelesene Länge und liest nur Neues; eine unfertige
//! letzte Zeile bleibt für den nächsten Abgleich liegen. Helio schreibt nie in den Ordner der Notch.
//! Format: notch/docs/health-export.md.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::{daily, store};

/// Ein Wert je Zeile. Sonderfelder (Schlafphase `vl`, Sportart `wt`, `kcal`, `dist`, Metadaten `m`) liegen als JSON in `extra`.
pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sample (
    id    TEXT PRIMARY KEY,   -- UUID aus Apple Health bzw. Inhalts-Hash beim Export
    type  TEXT NOT NULL,
    start INTEGER NOT NULL,   -- ms seit 1970 (UTC)
    end   INTEGER NOT NULL,
    lstart INTEGER NOT NULL,  -- start in Ortszeit des iPhones (ms, als wäre sie UTC): für Tage, Stunden, Uhrzeiten
    day   TEXT NOT NULL,      -- Tag der Ablage (Schlaf: Aufwachtag)
    value REAL NOT NULL,
    unit  TEXT,
    src   TEXT,
    dev   TEXT,
    extra TEXT
);
CREATE INDEX IF NOT EXISTS sample_type_lstart ON sample(type, lstart);
CREATE INDEX IF NOT EXISTS sample_day_type ON sample(day, type);
CREATE TABLE IF NOT EXISTS import_state (file TEXT PRIMARY KEY, bytes INTEGER NOT NULL);
-- eigene Markierungen (Seite Tage), z. B. kind = 'krank'; nie aus Health, bleibt beim Neueinlesen
CREATE TABLE IF NOT EXISTS mark (day TEXT PRIMARY KEY, kind TEXT NOT NULL);
";

/// Tabellen anlegen. Hat `sample` noch kein `lstart` (Block 1), wird der Health-Teil neu eingelesen;
/// die Puls-Tabelle `hr` bleibt unberührt.
pub fn migrate(c: &Connection) -> rusqlite::Result<()> {
    let cols: Vec<String> = c.prepare("PRAGMA table_info(sample)")?.query_map([], |r| r.get(1))?.collect::<Result<_, _>>()?;
    if !cols.is_empty() && !cols.iter().any(|x| x == "lstart") {
        c.execute_batch("DROP TABLE IF EXISTS sample; DROP TABLE IF EXISTS import_state; DROP TABLE IF EXISTS daily; PRAGMA user_version = 0;")?;
    }
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS hr (
             ts  INTEGER PRIMARY KEY,   -- Strap-Puls, ms seit 1970 (store.rs)
             bpm INTEGER NOT NULL
         );",
    )?;
    c.execute_batch(SCHEMA)?;
    c.execute_batch(daily::SCHEMA)
}

static RUNNING: AtomicBool = AtomicBool::new(false);
/// Stand nach dem letzten Abgleich: Zählen über Millionen Zeilen dauert, die Seite soll sofort antworten.
static STATUS: Mutex<Option<Status>> = Mutex::new(None);
/// Fehler des letzten Abgleichs, für die Seite Quellen (die fertige App hat keine Konsole).
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn dir(app: &AppHandle) -> Option<PathBuf> {
    // HELIO_HEALTH_DIR: Testordner statt der echten Ablage der Notch
    std::env::var_os("HELIO_HEALTH_DIR")
        .map(PathBuf::from)
        .or_else(|| app.path().document_dir().ok().map(|d| d.join("Notch").join("Health")))
}

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// "2026-11-18T08:00:00+01:00" (so schreiben App und Notch) → (ms seit 1970, Ortszeit in ms).
/// Sekundenbruchteile werden ignoriert.
pub fn parse_time(s: &str) -> Option<(i64, i64)> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let zone = s.get(19..)?.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let off = match zone.as_bytes() {
        [b'Z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let m = zone.get(1..3)?.parse::<i64>().ok()? * 60 + zone.get(4..6)?.parse::<i64>().ok()?;
            if *sign == b'-' { -m } else { m }
        }
        _ => return None,
    };
    let local = ((days_from_civil(y, mo, d) * 24 + h) * 60 + mi) * 60_000 + se * 1000;
    Some((local - off * 60_000, local))
}

/// Eine Zeile prüfen und ablegen. `false` = übersprungen (kaputt oder unvollständig).
fn insert(c: &Connection, v: &Value) -> rusqlite::Result<Option<bool>> {
    let (Some(o), Some(id), Some(t), Some(day), Some(val)) =
        (v.as_object(), v["id"].as_str(), v["t"].as_str(), v["day"].as_str(), v["v"].as_f64())
    else {
        return Ok(None);
    };
    let (Some((s, ls)), Some((e, _))) = (v["s"].as_str().and_then(parse_time), v["e"].as_str().and_then(parse_time)) else {
        return Ok(None);
    };
    if id.is_empty() || t.is_empty() || day.len() != 10 || !val.is_finite() {
        return Ok(None);
    }
    let mut extra = o.clone();
    for k in ["id", "t", "s", "e", "day", "v", "u", "src", "dev"] {
        extra.remove(k);
    }
    let extra = (!extra.is_empty()).then(|| Value::Object(extra).to_string());
    let n = c.execute(
        "INSERT OR IGNORE INTO sample (id, type, start, end, lstart, day, value, unit, src, dev, extra)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![id, t, s, e, ls, day, val, v["u"].as_str(), v["src"].as_str(), v["dev"].as_str(), extra],
    )?;
    Ok(Some(n == 1))
}

#[derive(Serialize, Default, Debug, PartialEq, Clone)]
pub struct ImportResult {
    pub files: usize,
    pub added: usize,
    pub known: usize,
    pub skipped: usize,
}

fn is_day_file(name: &str) -> bool {
    let b = name.as_bytes();
    name.len() == 16
        && name.ends_with(".jsonl")
        && b[..10].iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// Neue Zeilen einer Tagesdatei einlesen. Eine Transaktion je Datei, damit der Live-Puls nie lange warten muss.
pub fn import_file(c: &mut Connection, path: &Path, name: &str, r: &mut ImportResult) -> Result<(), String> {
    let done: u64 = c
        .query_row("SELECT bytes FROM import_state WHERE file = ?1", [name], |x| x.get::<_, i64>(0))
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(0) as u64;
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    // Kürzer als gelesen: Datei wurde ersetzt. Von vorn lesen, die id verhindert Doppelte.
    let from = if len < done { 0 } else { done };
    if len == from {
        return Ok(());
    }
    f.seek(SeekFrom::Start(from)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let Some(end) = buf.iter().rposition(|&b| b == b'\n').map(|i| i + 1) else {
        return Ok(()); // noch keine vollständige Zeile
    };
    let tx = c.transaction().map_err(|e| e.to_string())?;
    for line in buf[..end].split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<Value>(line).ok().map(|v| insert(&tx, &v)) {
            Some(Ok(Some(true))) => r.added += 1,
            Some(Ok(Some(false))) => r.known += 1,
            Some(Err(e)) => return Err(e.to_string()),
            _ => r.skipped += 1,
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO import_state (file, bytes) VALUES (?1, ?2)",
        params![name, (from + end as u64) as i64],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    r.files += 1;
    Ok(())
}

pub fn import_dir(c: &mut Connection, dir: &Path) -> Result<ImportResult, String> {
    let mut r = ImportResult::default();
    let Ok(rd) = std::fs::read_dir(dir) else { return Ok(r) };
    let mut names: Vec<String> = rd.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| is_day_file(n)).collect();
    names.sort();
    let mut days = BTreeSet::new();
    for n in names {
        let before = r.added;
        import_file(c, &dir.join(&n), &n, &mut r)?;
        if r.added > before {
            // Werte von Tag D können auch die Nacht zu D+1 betreffen (Schlaf ab 18 Uhr am Vorabend)
            let d = &n[..10];
            days.insert(d.to_string());
            days.extend(daily::day_num(d).map(|x| daily::day_str(x + 1)));
        }
    }
    daily::rebuild(c, &days).map_err(|e| e.to_string())?;
    Ok(r)
}

#[derive(Serialize, Clone)]
pub struct TypeCount {
    pub t: String,
    pub n: i64,
    pub last: i64,
}

#[derive(Serialize, Clone)]
pub struct Status {
    pub dir: String,
    pub exists: bool,
    pub samples: i64,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
    pub types: Vec<TypeCount>,
    pub sources: Vec<(String, i64)>,
    pub running: bool,
    pub error: Option<String>,
}

pub fn status_of(c: &Connection, dir: &Path) -> rusqlite::Result<Status> {
    let (samples, first_day, last_day) =
        c.query_row("SELECT COUNT(*), MIN(day), MAX(day) FROM sample", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mut st = c.prepare("SELECT type, COUNT(*), MAX(end) FROM sample GROUP BY type ORDER BY COUNT(*) DESC")?;
    let types = st.query_map([], |r| Ok(TypeCount { t: r.get(0)?, n: r.get(1)?, last: r.get(2)? }))?.collect::<Result<_, _>>()?;
    let mut st = c.prepare("SELECT COALESCE(src, '?'), COUNT(*) FROM sample GROUP BY 1 ORDER BY 2 DESC")?;
    let sources = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    Ok(Status {
        dir: dir.display().to_string(),
        exists: dir.is_dir(),
        samples,
        first_day,
        last_day,
        types,
        sources,
        running: RUNNING.load(Ordering::Relaxed),
        error: LAST_ERROR.lock().unwrap().clone(),
    })
}

/// Eigene Verbindung: Abfragen sollen den Live-Puls (gemeinsame Verbindung) nicht aufhalten.
pub fn conn(app: &AppHandle) -> Result<Connection, String> {
    store::connect(&app.path().app_data_dir().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

// async: läuft nicht im Haupt-Thread, das Zählen hält das Fenster nicht an
#[tauri::command]
pub async fn health_status(app: AppHandle) -> Result<Status, String> {
    if let Some(mut s) = STATUS.lock().unwrap().clone() {
        s.running = RUNNING.load(Ordering::Relaxed);
        s.error = LAST_ERROR.lock().unwrap().clone();
        return Ok(s);
    }
    let d = dir(&app).ok_or("Dokumente-Ordner nicht gefunden")?;
    status_of(&conn(&app)?, &d).map_err(|e| e.to_string())
}

/// Abgleich in einem eigenen Thread mit eigener Verbindung (WAL: der Live-Puls schreibt nebenher weiter).
fn sync(app: &AppHandle) {
    if RUNNING.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = app.emit("health", ());
    let res = (|| {
        let d = dir(app).ok_or("Dokumente-Ordner nicht gefunden")?;
        let mut c = conn(app)?;
        daily::ensure_rules(&mut c).map_err(|e| e.to_string())?;
        let r = import_dir(&mut c, &d)?;
        // Zählen nur, wenn sich etwas geändert hat (oder noch nie gezählt)
        if r.added > 0 || STATUS.lock().unwrap().is_none() {
            *STATUS.lock().unwrap() = Some(status_of(&c, &d).map_err(|e| e.to_string())?);
        }
        Ok(r)
    })();
    RUNNING.store(false, Ordering::Relaxed);
    *LAST_ERROR.lock().unwrap() = res.as_ref().err().cloned();
    let _ = app.emit("health", res.ok());
}

#[tauri::command]
pub fn health_sync(app: AppHandle) {
    std::thread::spawn(move || sync(&app));
}

/// Beim Start und danach alle 5 Minuten.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        sync(&app);
        std::thread::sleep(Duration::from_secs(300));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        migrate(&c).unwrap();
        c
    }
    fn line(id: &str, t: &str, v: f64) -> String {
        format!(r#"{{"id":"{id}","t":"{t}","s":"2026-11-18T08:00:00+01:00","e":"2026-11-18T08:01:00+01:00","day":"2026-11-18","v":{v},"u":"count/min","src":"Zepp"}}"#)
    }
    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("helio-health-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn count(c: &Connection) -> i64 {
        c.query_row("SELECT COUNT(*) FROM sample", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn zeit_mit_zeitzone() {
        let parse_time = |s| super::parse_time(s).map(|x| x.0);
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-11-18T08:00:00+01:00"), Some(1_794_985_200_000));
        assert_eq!(parse_time("2026-11-18T07:00:00Z"), Some(1_794_985_200_000));
        assert_eq!(parse_time("2026-03-29T03:00:00+02:00"), parse_time("2026-03-29T01:00:00Z"));
        assert_eq!(parse_time("2026-11-18T03:30:00-04:30"), parse_time("2026-11-18T08:00:00Z"));
        assert_eq!(parse_time("2026-11-18T08:00:00.250+01:00"), Some(1_794_985_200_000));
        assert_eq!(parse_time("2024-02-29T12:00:00Z").map(|t| t % 86_400_000), Some(43_200_000));
        for bad in ["", "2026-11-18", "2026-11-18 08:00:00 +0100", "2026-13-01T00:00:00Z", "2026-11-18T08:00:00+0100", "2026-11-18T08:00:00"] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
        // Ortszeit: 08:00 bleibt 08:00, egal welche Zone
        let local = |s| super::parse_time(s).map(|x| x.1 % 86_400_000 / 3_600_000);
        assert_eq!((local("2026-11-18T08:00:00+01:00"), local("2026-11-18T08:00:00-07:00")), (Some(8), Some(8)));
    }

    #[test]
    fn nur_tagesdateien() {
        assert!(is_day_file("2026-11-18.jsonl"));
        for n in ["_status.json", "2026-11-18.json", "2026-1-18.jsonl", "x026-11-18.jsonl", "2026-11-18.jsonl.tmp"] {
            assert!(!is_day_file(n), "{n}");
        }
    }

    #[test]
    fn nur_neues_ohne_doppelte() {
        let d = tmp("neu");
        let f = d.join("2026-11-18.jsonl");
        std::fs::write(&f, format!("{}\n{}\n", line("a", "heartRate", 72.0), line("b", "heartRate", 75.0))).unwrap();
        std::fs::write(d.join("_status.json"), "{}").unwrap();
        let mut c = db();
        assert_eq!(import_dir(&mut c, &d).unwrap(), ImportResult { files: 1, added: 2, known: 0, skipped: 0 });
        // zweiter Lauf ohne Änderung: nichts gelesen
        assert_eq!(import_dir(&mut c, &d).unwrap(), ImportResult::default());
        // angehängt: nur die neue Zeile
        let mut a = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        writeln!(a, "{}", line("c", "stepCount", 120.0)).unwrap();
        assert_eq!(import_dir(&mut c, &d).unwrap().added, 1);
        assert_eq!(count(&c), 3);
        // ersetzt (kürzer): von vorn, bekannte ids werden erkannt
        std::fs::write(&f, format!("{}\n", line("a", "heartRate", 72.0))).unwrap();
        assert_eq!(import_dir(&mut c, &d).unwrap(), ImportResult { files: 1, added: 0, known: 1, skipped: 0 });
        assert_eq!(count(&c), 3);
    }

    #[test]
    fn unfertige_zeile_wartet() {
        let d = tmp("halb");
        let f = d.join("2026-11-18.jsonl");
        let full = line("a", "heartRate", 72.0);
        std::fs::write(&f, format!("{}\n{}", full, &full[..30])).unwrap();
        let mut c = db();
        assert_eq!(import_dir(&mut c, &d).unwrap().added, 1);
        // Rest der Zeile kommt dazu
        std::fs::write(&f, format!("{}\n{}\n", full, line("b", "heartRate", 80.0))).unwrap();
        assert_eq!(import_dir(&mut c, &d).unwrap(), ImportResult { files: 1, added: 1, known: 0, skipped: 0 });
    }

    #[test]
    fn kaputte_zeilen_und_sonderfelder() {
        let d = tmp("sonder");
        let sleep = r#"{"id":"s1","t":"sleep","s":"2026-11-17T23:30:00+01:00","e":"2026-11-18T06:30:00+01:00","day":"2026-11-18","v":4,"vl":"deep","src":"Zepp"}"#;
        let body = [
            sleep,
            "kein json",
            r#"{"id":"x","t":"heartRate","s":"gestern","e":"2026-11-18T08:00:00+01:00","day":"2026-11-18","v":1}"#,
            r#"{"id":"y","t":"heartRate","s":"2026-11-18T08:00:00+01:00","e":"2026-11-18T08:00:00+01:00","day":"2026-11-18","v":"72"}"#,
            "",
        ]
        .join("\r\n");
        std::fs::write(d.join("2026-11-18.jsonl"), body).unwrap();
        let mut c = db();
        assert_eq!(import_dir(&mut c, &d).unwrap(), ImportResult { files: 1, added: 1, known: 0, skipped: 3 });
        let (extra, unit): (Option<String>, Option<String>) =
            c.query_row("SELECT extra, unit FROM sample WHERE id='s1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(extra.as_deref(), Some(r#"{"vl":"deep"}"#));
        assert_eq!(unit, None);
        let st = status_of(&c, &d).unwrap();
        assert_eq!((st.samples, st.first_day.as_deref(), st.types[0].t.as_str()), (1, Some("2026-11-18"), "sleep"));
    }

    #[test]
    fn alte_tabelle_wird_neu_eingelesen() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE hr (ts INTEGER PRIMARY KEY, bpm INTEGER); INSERT INTO hr VALUES (1, 60);
             CREATE TABLE sample (id TEXT PRIMARY KEY, type TEXT, start INTEGER, end INTEGER, day TEXT, value REAL, unit TEXT, src TEXT, dev TEXT, extra TEXT);
             INSERT INTO sample VALUES ('a','heartRate',0,0,'2026-11-18',60,NULL,NULL,NULL,NULL);
             CREATE TABLE import_state (file TEXT PRIMARY KEY, bytes INTEGER); INSERT INTO import_state VALUES ('2026-11-18.jsonl', 99);",
        )
        .unwrap();
        migrate(&c).unwrap();
        migrate(&c).unwrap(); // zweiter Start: nichts mehr löschen
        let n = |t: &str| c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!((n("hr"), n("sample"), n("import_state"), n("daily")), (1, 0, 0, 0));
    }

    #[test]
    fn abgleich_rechnet_tage() {
        let d = tmp("tage");
        let step = |id: &str, h: u8, v: f64| format!(
            r#"{{"id":"{id}","t":"stepCount","s":"2026-11-18T{h:02}:00:00+01:00","e":"2026-11-18T{h:02}:30:00+01:00","day":"2026-11-18","v":{v},"src":"iPhone Nojo"}}"#
        );
        std::fs::write(d.join("2026-11-18.jsonl"), format!("{}
{}
", step("a", 8, 100.0), step("b", 23, 50.0))).unwrap();
        let mut c = db();
        import_dir(&mut c, &d).unwrap();
        let m = daily::day_of(&c, "2026-11-18").unwrap();
        // 23 Uhr Ortszeit ist UTC schon 22 Uhr, aber beides zählt zum 18.
        assert_eq!(m.iter().find(|x| x.key == "steps").map(|x| x.value), Some(150.0));
        assert_eq!(daily::range_of(&c).unwrap(), Some(("2026-11-18".into(), "2026-11-18".into())));
    }

    #[test]
    fn fehlender_ordner_ist_kein_fehler() {
        let mut c = db();
        assert_eq!(import_dir(&mut c, Path::new("Z:/gibt/es/nicht")).unwrap(), ImportResult::default());
        assert!(!status_of(&c, Path::new("Z:/gibt/es/nicht")).unwrap().exists);
    }
}

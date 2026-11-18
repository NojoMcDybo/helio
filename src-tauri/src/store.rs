//! Lokale Datenbank: jede Messung (≈ 1/s) landet in SQLite unter %APPDATA%\de.nojo.helio\helio.db.
//! Abfragen fassen die Rohwerte in Zeitfenster zusammen, damit das Frontend nie 86 400 Punkte bekommt.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::Serialize;

static DB: Mutex<Option<Connection>> = Mutex::new(None);

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

/// Eigene Verbindung, z. B. für den Health-Abgleich im Hintergrund (WAL: Lesen und Schreiben nebeneinander).
pub fn connect(dir: &Path) -> rusqlite::Result<Connection> {
    let _ = std::fs::create_dir_all(dir);
    let c = Connection::open(dir.join("helio.db"))?;
    c.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;",
    )?;
    crate::health::migrate(&c)?;
    Ok(c)
}

pub fn open(dir: &Path) -> rusqlite::Result<()> {
    *DB.lock().unwrap() = Some(connect(dir)?);
    Ok(())
}

pub fn insert(ts: i64, bpm: u16) {
    if let Some(c) = DB.lock().unwrap().as_ref() {
        let _ = c.execute("INSERT OR REPLACE INTO hr (ts, bpm) VALUES (?1, ?2)", params![ts, bpm]);
    }
}

#[derive(Serialize)]
pub struct Bucket {
    /// Beginn des Zeitfensters (ms)
    pub t: i64,
    pub avg: f64,
    pub min: i64,
    pub max: i64,
}

/// Werte zwischen `from` und `to` (ms), zusammengefasst in Fenster von `bucket` ms.
#[tauri::command]
pub fn series(from: i64, to: i64, bucket: i64) -> Result<Vec<Bucket>, String> {
    let g = DB.lock().unwrap();
    let c = g.as_ref().ok_or("Datenbank nicht offen")?;
    let b = bucket.max(1000);
    let mut st = c
        .prepare_cached(
            "SELECT (ts / ?3) * ?3 AS t, AVG(bpm), MIN(bpm), MAX(bpm)
             FROM hr WHERE ts >= ?1 AND ts < ?2 GROUP BY t ORDER BY t",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map(params![from, to, b], |r| {
            Ok(Bucket { t: r.get(0)?, avg: r.get(1)?, min: r.get(2)?, max: r.get(3)? })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[derive(Serialize, Default)]
pub struct Stats {
    /// Anzahl Messungen (≈ Sekunden mit Daten)
    pub count: i64,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub avg: Option<f64>,
    /// Ruhepuls: niedrigster 5-Minuten-Schnitt (mind. 4 min Daten im Fenster)
    pub rest: Option<f64>,
    /// Sekunden je Zone: <50 %, 50–60, 60–70, 70–80, 80–90, ≥90 % der HFmax
    pub zones: [i64; 6],
}

#[tauri::command]
pub fn stats(from: i64, to: i64, hrmax: i64) -> Result<Stats, String> {
    let g = DB.lock().unwrap();
    let c = g.as_ref().ok_or("Datenbank nicht offen")?;
    let e = |e: rusqlite::Error| e.to_string();
    let mut s = Stats::default();
    c.query_row(
        "SELECT COUNT(*), MIN(bpm), MAX(bpm), AVG(bpm) FROM hr WHERE ts >= ?1 AND ts < ?2",
        params![from, to],
        |r| {
            s.count = r.get(0)?;
            s.min = r.get(1)?;
            s.max = r.get(2)?;
            s.avg = r.get(3)?;
            Ok(())
        },
    )
    .map_err(e)?;
    s.rest = c
        .query_row(
            "SELECT MIN(a) FROM (SELECT AVG(bpm) a, COUNT(*) n FROM hr
             WHERE ts >= ?1 AND ts < ?2 GROUP BY ts / 300000 HAVING n >= 240)",
            params![from, to],
            |r| r.get(0),
        )
        .map_err(e)?;
    let h = hrmax.max(100) as f64;
    let mut st = c
        .prepare_cached(
            "SELECT CASE
                 WHEN bpm < ?3 * 0.5 THEN 0 WHEN bpm < ?3 * 0.6 THEN 1 WHEN bpm < ?3 * 0.7 THEN 2
                 WHEN bpm < ?3 * 0.8 THEN 3 WHEN bpm < ?3 * 0.9 THEN 4 ELSE 5 END AS z, COUNT(*)
             FROM hr WHERE ts >= ?1 AND ts < ?2 GROUP BY z",
        )
        .map_err(e)?;
    let rows = st
        .query_map(params![from, to, h], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .map_err(e)?;
    for (z, n) in rows.flatten() {
        if (0..6).contains(&z) {
            s.zones[z as usize] = n;
        }
    }
    Ok(s)
}

/// Erster und letzter Tag mit Daten (ms), fuer die Tagesnavigation.
#[tauri::command]
pub fn range() -> Option<(i64, i64)> {
    let g = DB.lock().unwrap();
    let c = g.as_ref()?;
    c.query_row("SELECT MIN(ts), MAX(ts) FROM hr", [], |r| {
        Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?))
    })
    .ok()
    .and_then(|(a, b)| Some((a?, b?)))
}

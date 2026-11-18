//! Tagesmatrix: je Tag und Kennzahl genau ein Wert, ohne Doppelzählung über Quellen, mit Datenqualität.
//! Grundlage für alle Auswertungen (docs/BAUPLAN-HEALTH.md, Block 2). Wird nach jedem Import für die
//! betroffenen Tage neu berechnet; Zeiten sind Ortszeit des iPhones (`lstart`).
//!
//! Regeln:
//! - Summen (Schritte, Energie, Essen …): je Stunde und Quelle summieren, je Stunde die größte Quelle nehmen.
//!   So zählen iPhone, zweites iPhone, Zepp und Nike nicht doppelt, und Lücken einer Quelle füllt die andere.
//! - Mittel/Min/Max/Letzter: nur eine Quelle, nach Vorrang Zepp > WHOOP > iPhone > Rest.
//! - Schlaf: Abschnitte, die zwischen 18 Uhr am Vortag und 18 Uhr beginnen, aus der besten Quelle mit Schlafphasen.
//! - Qualität: 2 gut, 1 gering, 0 zu wenig.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::AppHandle;

use crate::health::{self, days_from_civil};

/// Bei geänderten Regeln erhöhen: der nächste Abgleich rechnet dann alle Tage neu.
pub const VERSION: i64 = 5;

/// Höchstpuls für Zonen und Trainingslast, wie `HRMAX` in src/pulse-draw.ts (Faustformel 220 − Alter).
pub const HRMAX: f64 = 200.0;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS daily (
    day     TEXT NOT NULL,
    key     TEXT NOT NULL,
    value   REAL NOT NULL,
    src     TEXT,
    quality INTEGER NOT NULL,
    PRIMARY KEY (day, key)
) WITHOUT ROWID;
-- Stundenwerte je Tag (Ortszeit) für den Tagesrhythmus: steps = Schritte (größte Quelle je Stunde), hr = Puls (Zeitmittel)
CREATE TABLE IF NOT EXISTS hourly (
    day   TEXT NOT NULL,
    key   TEXT NOT NULL,
    h     INTEGER NOT NULL,
    value REAL NOT NULL,
    PRIMARY KEY (day, key, h)
) WITHOUT ROWID;
";

const MIN: i64 = 60_000;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 86_400_000;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Metric {
    pub key: String,
    pub value: f64,
    pub src: Option<String>,
    pub quality: u8,
}

pub(crate) struct Row {
    pub t: String,
    pub start: i64,
    pub end: i64,
    pub ls: i64,
    pub v: f64,
    pub src: String,
    pub extra: Option<String>,
}

pub fn day_num(day: &str) -> Option<i64> {
    let n = |a: usize, z: usize| day.get(a..z)?.parse::<i64>().ok();
    (day.len() == 10).then_some(())?;
    Some(days_from_civil(n(0, 4)?, n(5, 7)?, n(8, 10)?))
}

pub fn day_str(z: i64) -> String {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Vorrang, wenn nur eine Quelle zählen darf (kleiner = besser).
fn rank(src: &str) -> u8 {
    match src {
        "Zepp" => 0,
        "WHOOP" => 1,
        s if s.starts_with("iPhone") || s.contains("Handy") => 2,
        _ => 3,
    }
}

#[derive(Clone, Copy)]
enum Agg {
    Sum,
    Mean,
    Min,
    Max,
    Last,
}
#[derive(Clone, Copy)]
enum Q {
    /// Stunden mit Daten: (gut ab, gering ab)
    Hours(usize, usize),
    /// Anzahl Werte, ab der es „gut“ ist (darunter „gering“)
    Count(usize),
    Always,
}
use Agg::*;
use Q::*;

const RULES: &[(&str, &str, Agg, Q)] = &[
    ("steps", "stepCount", Sum, Hours(12, 4)),
    ("distance", "distanceWalkingRunning", Sum, Hours(12, 4)),
    ("flights", "flightsClimbed", Sum, Hours(12, 4)),
    ("active_kcal", "activeEnergyBurned", Sum, Hours(12, 4)),
    ("basal_kcal", "basalEnergyBurned", Sum, Hours(18, 8)),
    ("hr_avg", "heartRate", Mean, Hours(18, 8)),
    ("hr_min", "heartRate", Min, Hours(18, 8)),
    ("hr_max", "heartRate", Max, Hours(18, 8)),
    ("rhr", "restingHeartRate", Mean, Always),
    ("hrv", "heartRateVariabilitySDNN", Mean, Count(2)),
    ("resp", "respiratoryRate", Mean, Count(10)),
    ("spo2", "oxygenSaturation", Mean, Count(5)),
    ("spo2_min", "oxygenSaturation", Min, Count(5)),
    ("walk_hr", "walkingHeartRateAverage", Mean, Always),
    ("vo2max", "vo2Max", Last, Always),
    ("energy", "dietaryEnergy", Sum, Count(3)),
    ("protein", "dietaryProtein", Sum, Count(2)),
    ("carbs", "dietaryCarbohydrates", Sum, Count(2)),
    ("fat", "dietaryFat", Sum, Count(2)),
    ("sugar", "dietarySugar", Sum, Count(2)),
    ("fiber", "dietaryFiber", Sum, Count(2)),
    ("caffeine", "dietaryCaffeine", Sum, Always),
    ("water", "dietaryWater", Sum, Always),
    ("alcohol", "alcoholicBeverages", Sum, Always),
    ("weight", "bodyMass", Last, Always),
    ("bodyfat", "bodyFatPercentage", Last, Always),
    ("bmi", "bodyMassIndex", Last, Always),
    ("walk_speed", "walkingSpeed", Mean, Count(3)),
    ("step_length", "walkingStepLength", Mean, Count(3)),
    ("asymmetry", "walkingAsymmetry", Mean, Count(3)),
    ("double_support", "walkingDoubleSupport", Mean, Count(3)),
    ("steadiness", "walkingSteadiness", Last, Always),
    ("headphone_db", "headphoneAudioExposure", Mean, Count(3)),
    ("mindful_min", "mindfulSession", Sum, Always),
];

fn quality(q: Q, rows: &[&Row]) -> u8 {
    match q {
        Always => 2,
        Count(g) => if rows.len() >= g { 2 } else { 1 },
        Hours(g, l) => {
            let h = rows.iter().map(|r| r.ls.div_euclid(HOUR)).collect::<BTreeSet<_>>().len();
            if h >= g { 2 } else if h >= l { 1 } else { 0 }
        }
    }
}

/// Je Stunde und Quelle summieren; je Stunde (lokale Stundennummer) die Summe der größten Quelle.
fn per_hour<'a>(rows: &[&'a Row]) -> HashMap<i64, (f64, &'a str)> {
    let mut per: HashMap<(i64, &str), f64> = HashMap::new();
    for r in rows {
        *per.entry((r.ls.div_euclid(HOUR), &r.src)).or_default() += r.v;
    }
    let mut best: HashMap<i64, (f64, &str)> = HashMap::new();
    for ((h, s), v) in per {
        let b = best.entry(h).or_insert((v, s));
        if v > b.0 || (v == b.0 && rank(s) < rank(b.1)) {
            *b = (v, s);
        }
    }
    best
}

/// Tagessumme aus `per_hour` und die Quelle mit dem größten Anteil.
fn hourly_max(rows: &[&Row]) -> (f64, String) {
    let best = per_hour(rows);
    let mut share: HashMap<&str, f64> = HashMap::new();
    let total = best.values().map(|(v, s)| { *share.entry(s).or_default() += v; v }).sum();
    let src = share.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(rank(b.0).cmp(&rank(a.0)))).map(|x| x.0.to_string());
    (total, src.unwrap_or_default())
}

/// Mittel über 5-Minuten-Fenster: Zepp misst im Training jede Sekunde, sonst selten; ein einfaches Mittel wäre fast nur Training.
fn time_mean(rows: &[&Row]) -> f64 {
    let mut b: HashMap<i64, (f64, f64)> = HashMap::new();
    for r in rows {
        let e = b.entry(r.ls.div_euclid(5 * MIN)).or_default();
        e.0 += r.v;
        e.1 += 1.0;
    }
    b.values().map(|(s, n)| s / n).sum::<f64>() / b.len() as f64
}

/// Beste Quelle nach Vorrang, bei Gleichstand die mit mehr Werten.
pub(crate) fn best_source<'a>(rows: &[&'a Row]) -> Option<&'a str> {
    let mut n: HashMap<&str, usize> = HashMap::new();
    for r in rows {
        *n.entry(&r.src).or_default() += 1;
    }
    n.into_iter().min_by(|a, b| rank(a.0).cmp(&rank(b.0)).then(b.1.cmp(&a.1))).map(|x| x.0)
}

fn metric(key: &str, value: f64, src: Option<&str>, quality: u8) -> Metric {
    Metric { key: key.into(), value, src: src.map(str::to_string), quality }
}

pub(crate) fn rows(c: &Connection, sql: &str, p: impl rusqlite::Params) -> rusqlite::Result<Vec<Row>> {
    let mut st = c.prepare_cached(sql)?;
    let it = st.query_map(p, |r| {
        Ok(Row { t: r.get(0)?, start: r.get(1)?, end: r.get(2)?, ls: r.get(3)?, v: r.get(4)?, src: r.get(5)?, extra: r.get(6)? })
    })?;
    it.collect()
}
pub(crate) const COLS: &str = "SELECT type, start, end, lstart, value, COALESCE(src, '?'), extra FROM sample";

#[cfg(test)]
pub fn compute_day(c: &Connection, day: &str) -> rusqlite::Result<Vec<Metric>> {
    Ok(compute(c, day)?.0)
}

/// Stundenwert: (Stunde 0–23 Ortszeit, Schlüssel, Wert)
pub type Hour = (i64, &'static str, f64);

/// Stundenwerte für Schritte (größte Quelle je Stunde) und Puls (Zeitmittel der besten Quelle).
fn hours(by_type: &HashMap<&str, Vec<&Row>>, dn: i64) -> Vec<Hour> {
    let mut out = Vec::new();
    let local = |abs: i64| Some(abs - dn * 24).filter(|h| (0..24).contains(h));
    if let Some(steps) = by_type.get("stepCount") {
        out.extend(per_hour(steps).into_iter().filter_map(|(h, (v, _))| Some((local(h)?, "steps", v))));
    }
    if let Some(hr) = by_type.get("heartRate") {
        if let Some(src) = best_source(hr) {
            let mut by_h: HashMap<i64, Vec<&Row>> = HashMap::new();
            for r in hr.iter().filter(|r| r.src == src) {
                by_h.entry(r.ls.div_euclid(HOUR)).or_default().push(r);
            }
            out.extend(by_h.into_iter().filter_map(|(h, v)| Some((local(h)?, "hr", time_mean(&v)))));
        }
    }
    out
}

fn compute(c: &Connection, day: &str) -> rusqlite::Result<(Vec<Metric>, Vec<Hour>)> {
    let Some(dn) = day_num(day) else { return Ok((vec![], vec![])) };
    let d0 = dn * DAY; // Mitternacht Ortszeit als „lokale ms“
    let all = rows(c, &format!("{COLS} WHERE day = ?1 AND type != 'sleep'"), [day])?;
    let mut out = Vec::new();
    let mut by_type: HashMap<&str, Vec<&Row>> = HashMap::new();
    for r in &all {
        by_type.entry(&r.t).or_default().push(r);
    }

    for &(key, ty, agg, q) in RULES {
        let Some(list) = by_type.get(ty) else { continue };
        let qual = quality(q, list);
        if let Sum = agg {
            let (v, src) = hourly_max(list);
            let v = if key == "mindful_min" { v / 60.0 } else { v };
            out.push(metric(key, v, Some(&src), qual));
            continue;
        }
        let Some(src) = best_source(list) else { continue };
        let mine: Vec<&Row> = list.iter().copied().filter(|r| r.src == src).collect();
        let vals = mine.iter().map(|r| r.v);
        let v = match agg {
            Mean => time_mean(&mine),
            Min => vals.fold(f64::INFINITY, f64::min),
            Max => vals.fold(f64::NEG_INFINITY, f64::max),
            Last => mine.iter().max_by_key(|r| r.end).map(|r| r.v).unwrap_or(f64::NAN),
            Sum => unreachable!(),
        };
        if v.is_finite() {
            out.push(metric(key, v, Some(src), qual));
        }
    }

    if let Some(meals) = by_type.get("dietaryEnergy") {
        let last = meals.iter().map(|r| r.ls).max().unwrap_or(d0);
        out.push(metric("last_meal", ((last - d0) / MIN) as f64, best_source(meals), 2));
    }
    // Koffein ab 14 Uhr (Frage „Koffein am Nachmittag → Schlaf“); 0, wenn an dem Tag Koffein erfasst ist, aber nur früher
    if let Some(caf) = by_type.get("dietaryCaffeine") {
        let late: Vec<&Row> = caf.iter().copied().filter(|r| r.ls - d0 >= 14 * HOUR).collect();
        out.push(metric("caffeine_late", per_hour(&late).values().map(|x| x.0).sum(), best_source(caf), 2));
    }

    if let Some(w) = by_type.get("workout") {
        let groups = workout_groups(w);
        out.extend(workouts(w, &groups));
        let hr = by_type.get("heartRate").map(Vec::as_slice).unwrap_or_default();
        out.extend(trimp(c, &groups, hr)?);
    }

    let sleep = rows(c, &format!("{COLS} WHERE type = 'sleep' AND lstart >= ?1 AND lstart < ?2"), params![d0 - 6 * HOUR, d0 + 18 * HOUR])?;
    if let Some((m, onset, wake)) = night(&sleep, d0) {
        out.extend(m);
        let hr = rows(c, &format!("{COLS} WHERE type = 'heartRate' AND lstart >= ?1 AND lstart < ?2"), params![onset, wake])?;
        if let Some(src) = best_source(&hr.iter().collect::<Vec<_>>()) {
            let v: Vec<&Row> = hr.iter().filter(|r| r.src == src).collect();
            let q = if v.len() >= 60 { 2 } else { 1 };
            out.push(metric("hr_night", time_mean(&v), Some(src), q));
            out.push(metric("hr_night_min", v.iter().map(|r| r.v).fold(f64::INFINITY, f64::min), Some(src), q));
        }
    }
    let h = hours(&by_type, dn);
    Ok((out, h))
}

/// Trainings mehrerer Apps (Zepp, Nike …) überlappen: zeitlich zusammenfassen (UTC-ms), je Gruppe die größte kcal-Angabe.
pub(crate) fn workout_groups(w: &[&Row]) -> Vec<(i64, i64, f64)> {
    let mut iv: Vec<(i64, i64, f64)> = w
        .iter()
        .map(|r| {
            let kcal = r.extra.as_deref().and_then(|x| serde_json::from_str::<serde_json::Value>(x).ok()).and_then(|x| x["kcal"].as_f64());
            (r.start, r.end.max(r.start), kcal.unwrap_or(0.0))
        })
        .collect();
    iv.sort_by_key(|x| x.0);
    let mut groups: Vec<(i64, i64, f64)> = Vec::new();
    for (s, e, k) in iv {
        match groups.last_mut() {
            Some(g) if s < g.1 => {
                g.1 = g.1.max(e);
                g.2 = g.2.max(k);
            }
            _ => groups.push((s, e, k)),
        }
    }
    groups
}

fn workouts(w: &[&Row], groups: &[(i64, i64, f64)]) -> Vec<Metric> {
    let src = best_source(w);
    vec![
        metric("workout_n", groups.len() as f64, src, 2),
        metric("workout_min", groups.iter().map(|g| (g.1 - g.0) as f64).sum::<f64>() / MIN as f64, src, 2),
        metric("workout_kcal", groups.iter().map(|g| g.2).sum(), src, 2),
    ]
}

/// Edwards-Gewicht einer Minute nach % HFmax: unter 50 % → 0, 50–60 % → 1 … ab 90 % → 5.
pub fn edwards(bpm: f64) -> f64 {
    ((bpm * 10.0 / HRMAX).floor() - 4.0).clamp(0.0, 5.0)
}

/// Trainingslast (Edwards-TRIMP) der Trainings des Tages: je Minute der mittlere Puls, vom Helio Strap (Tabelle `hr`),
/// sonst aus Health (beste Quelle). Minuten ohne Puls zählen nicht; Qualität = Anteil der Minuten mit Puls.
fn trimp(c: &Connection, groups: &[(i64, i64, f64)], hr: &[&Row]) -> rusqlite::Result<Option<Metric>> {
    let src = best_source(hr);
    let mut st = c.prepare_cached("SELECT ts, bpm FROM hr WHERE ts >= ?1 AND ts < ?2")?;
    let (mut load, mut covered, mut total, mut by_strap) = (0.0, 0usize, 0usize, 0usize);
    for &(s, e, _) in groups {
        let mut strap: HashMap<i64, (f64, f64)> = HashMap::new();
        for x in st.query_map(params![s, e], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))? {
            let (ts, bpm) = x?;
            let m = strap.entry(ts.div_euclid(MIN)).or_default();
            *m = (m.0 + bpm, m.1 + 1.0);
        }
        let mut health: HashMap<i64, (f64, f64)> = HashMap::new();
        for r in hr.iter().filter(|r| Some(r.src.as_str()) == src && r.start >= s && r.start < e) {
            let m = health.entry(r.start.div_euclid(MIN)).or_default();
            *m = (m.0 + r.v, m.1 + 1.0);
        }
        for m in s.div_euclid(MIN)..e.div_euclid(MIN).max(s.div_euclid(MIN) + 1) {
            total += 1;
            let v = match (strap.get(&m), health.get(&m)) {
                (Some(x), _) => {
                    by_strap += 1;
                    x
                }
                (None, Some(x)) => x,
                _ => continue,
            };
            covered += 1;
            load += edwards(v.0 / v.1);
        }
    }
    if covered == 0 {
        return Ok(None);
    }
    let share = covered as f64 / total as f64;
    let q = if share >= 0.8 { 2 } else if share >= 0.4 { 1 } else { 0 };
    let src = if by_strap * 2 > covered { Some("Helio Strap") } else { src };
    Ok(Some(metric("trimp", load, src, q)))
}

/// Schlafphasen (v): 0 im Bett, 1 schlafend, 2 wach, 3 Kern, 4 Tief, 5 REM.
/// Die Nacht ist der längste zusammenhängende Schlafblock (Lücken bis 90 min); alles andere zählt als Nickerchen.
/// Ergebnis: Kennzahlen sowie Einschlafen und Aufwachen (lokale ms) für den nächtlichen Puls.
fn night(sleep: &[Row], d0: i64) -> Option<(Vec<Metric>, i64, i64)> {
    let asleep = |r: &Row| matches!(r.v as i64, 1 | 3 | 4 | 5);
    let with_stages: Vec<&Row> = sleep.iter().filter(|r| asleep(r)).collect();
    let (src, has_sleep) = match best_source(&with_stages) {
        Some(s) => (s, true),
        None => (best_source(&sleep.iter().collect::<Vec<_>>())?, false),
    };
    let lend = |r: &Row| r.ls + (r.end - r.start);
    let min = |v: i64| v as f64 / MIN as f64;
    let all: Vec<&Row> = sleep.iter().filter(|r| r.src == src).collect();
    if !has_sleep {
        let from = all.iter().map(|r| r.ls).min()?;
        let to = all.iter().map(|r| lend(r)).max()?;
        return Some((vec![metric("inbed_min", min(to - from), Some(src), 1)], from, to));
    }
    // Schlafblöcke: (Beginn, Ende, geschlafen)
    let mut parts: Vec<&Row> = all.iter().copied().filter(|r| asleep(r)).collect();
    parts.sort_by_key(|r| r.ls);
    let mut blocks: Vec<(i64, i64, i64)> = Vec::new();
    for r in parts {
        match blocks.last_mut() {
            Some(b) if r.ls - b.1 <= 90 * MIN => {
                b.1 = b.1.max(lend(r));
                b.2 += r.end - r.start;
            }
            _ => blocks.push((r.ls, lend(r), r.end - r.start)),
        }
    }
    let &(onset, wake, slept) = blocks.iter().max_by_key(|b| b.2)?;
    let naps: i64 = blocks.iter().map(|b| b.2).sum::<i64>() - slept;
    // Abschnitte der Nacht, „im Bett“ darf bis 2 h vor dem Einschlafen beginnen
    let mine: Vec<&Row> = all.iter().copied().filter(|r| lend(r) > onset - 2 * HOUR && r.ls < wake + 30 * MIN).collect();
    let dur = |f: &dyn Fn(i64) -> bool| mine.iter().filter(|r| f(r.v as i64)).map(|r| r.end - r.start).sum::<i64>();
    let span_from = mine.iter().map(|r| r.ls).min()?;
    let span_to = mine.iter().map(|r| lend(r)).max()?;
    let q = if slept >= 3 * HOUR { 2 } else { 1 };
    let mut out = vec![metric("inbed_min", min(span_to - span_from), Some(src), 2)];
    out.push(metric("sleep_min", min(slept), Some(src), q));
    if naps > 0 {
        out.push(metric("nap_min", min(naps), Some(src), 2));
    }
    out.push(metric("awake_min", min(dur(&|v| v == 2)), Some(src), q));
    if mine.iter().any(|r| matches!(r.v as i64, 3..=5)) {
        out.push(metric("deep_min", min(dur(&|v| v == 4)), Some(src), q));
        out.push(metric("rem_min", min(dur(&|v| v == 5)), Some(src), q));
        out.push(metric("core_min", min(dur(&|v| v == 3)), Some(src), q));
    }
    if wake > onset {
        out.push(metric("efficiency", 100.0 * slept as f64 / (wake - onset) as f64, Some(src), q));
    }
    // Uhrzeiten als Minuten ab Mitternacht dieses Tages (negativ = am Vorabend)
    out.push(metric("onset", min(onset - d0), Some(src), q));
    out.push(metric("wake", min(wake - d0), Some(src), q));
    out.push(metric("mid", min((onset + wake) / 2 - d0), Some(src), q));
    Some((out, onset, wake))
}

/// Tage neu berechnen, je 100 Tage eine Transaktion (der Live-Puls soll nie lange warten).
pub fn rebuild(c: &mut Connection, days: &BTreeSet<String>) -> rusqlite::Result<()> {
    let days: Vec<&String> = days.iter().collect();
    for chunk in days.chunks(100) {
        let tx = c.transaction()?;
        for day in chunk {
            let (m, hours) = compute(&tx, day)?;
            tx.execute("DELETE FROM daily WHERE day = ?1", [day])?;
            tx.execute("DELETE FROM hourly WHERE day = ?1", [day])?;
            for x in m {
                tx.prepare_cached("INSERT INTO daily (day, key, value, src, quality) VALUES (?1, ?2, ?3, ?4, ?5)")?
                    .execute(params![day, x.key, x.value, x.src, x.quality])?;
            }
            for (h, key, v) in hours {
                tx.prepare_cached("INSERT INTO hourly (day, key, h, value) VALUES (?1, ?2, ?3, ?4)")?.execute(params![day, key, h, v])?;
            }
        }
        tx.commit()?;
    }
    Ok(())
}

/// Wurden die Regeln geändert (VERSION), alle Tage neu berechnen.
pub fn ensure_rules(c: &mut Connection) -> rusqlite::Result<()> {
    if c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? == VERSION {
        return Ok(());
    }
    let mut days = BTreeSet::new();
    for d in c.prepare("SELECT DISTINCT day FROM sample")?.query_map([], |r| r.get::<_, String>(0))? {
        let d = d?;
        days.extend(day_num(&d).map(|x| day_str(x + 1)));
        days.insert(d);
    }
    c.execute_batch("DELETE FROM daily; DELETE FROM hourly;")?;
    rebuild(c, &days)?;
    c.pragma_update(None, "user_version", VERSION)
}

pub fn day_of(c: &Connection, day: &str) -> rusqlite::Result<Vec<Metric>> {
    let mut st = c.prepare("SELECT key, value, src, quality FROM daily WHERE day = ?1")?;
    let it = st.query_map([day], |r| Ok(Metric { key: r.get(0)?, value: r.get(1)?, src: r.get(2)?, quality: r.get(3)? }))?;
    it.collect()
}

pub fn range_of(c: &Connection) -> rusqlite::Result<Option<(String, String)>> {
    c.query_row("SELECT MIN(day), MAX(day) FROM daily", [], |r| {
        Ok(r.get::<_, Option<String>>(0)?.zip(r.get::<_, Option<String>>(1)?))
    })
}

#[tauri::command]
pub async fn daily_day(app: AppHandle, day: String) -> Result<Vec<Metric>, String> {
    day_of(&health::conn(&app)?, &day).map_err(|e| e.to_string())
}

/// Erster und letzter Tag mit Kennzahlen.
#[tauri::command]
pub async fn daily_range(app: AppHandle) -> Result<Option<(String, String)>, String> {
    range_of(&health::conn(&app)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        health::migrate(&c).unwrap();
        c
    }
    /// Wert mit Ortszeit „TT HH:MM“ im November 2026, Zeitzone +01:00.
    #[allow(clippy::too_many_arguments)]
    fn add(c: &Connection, id: &str, t: &str, day: &str, from: &str, mins: i64, v: f64, src: &str, extra: Option<&str>) {
        let (d, hm) = from.split_once(' ').unwrap();
        let (h, m) = hm.split_once(':').unwrap();
        let ls = day_num(&format!("2026-11-{d}")).unwrap() * DAY + h.parse::<i64>().unwrap() * HOUR + m.parse::<i64>().unwrap() * MIN;
        let s = ls - HOUR;
        c.execute(
            "INSERT INTO sample (id, type, start, end, lstart, day, value, unit, src, dev, extra) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL,?8,NULL,?9)",
            params![id, t, s, s + mins * MIN, ls, day, v, src, extra],
        )
        .unwrap();
    }
    fn get<'a>(m: &'a [Metric], k: &str) -> Option<&'a Metric> {
        m.iter().find(|x| x.key == k)
    }

    #[test]
    fn tage_hin_und_zurueck() {
        for d in ["1970-01-01", "2019-03-07", "2024-02-29", "2026-11-18", "2100-12-31"] {
            assert_eq!(day_str(day_num(d).unwrap()), d);
        }
        assert_eq!(day_str(day_num("2026-12-31").unwrap() + 1), "2027-01-01");
        assert_eq!(day_num("2026-1-1"), None);
    }

    #[test]
    fn schritte_ohne_doppelzaehlung() {
        let c = db();
        let d = "2026-11-17";
        // 9 Uhr: iPhone und Zepp zählen dieselben Schritte → größere Quelle zählt, nicht die Summe
        add(&c, "a", "stepCount", d, "17 09:05", 10, 400.0, "iPhone Nojo", None);
        add(&c, "b", "stepCount", d, "17 09:40", 10, 300.0, "iPhone Nojo", None);
        add(&c, "c", "stepCount", d, "17 09:10", 50, 650.0, "Zepp", None);
        // 10 Uhr: nur Zepp (iPhone lag herum)
        add(&c, "d", "stepCount", d, "17 10:00", 60, 1000.0, "Zepp", None);
        let m = compute_day(&c, d).unwrap();
        let s = get(&m, "steps").unwrap();
        assert_eq!((s.value, s.src.as_deref()), (1700.0, Some("Zepp"))); // 700 (iPhone) + 1000 (Zepp)
        assert_eq!(s.quality, 0); // nur 2 Stunden mit Daten
    }

    #[test]
    fn mittel_aus_einer_quelle_nach_vorrang() {
        let c = db();
        let d = "2026-11-17";
        add(&c, "a", "restingHeartRate", d, "17 00:00", 0, 53.0, "Zepp", None);
        add(&c, "b", "restingHeartRate", d, "17 07:00", 0, 60.0, "WHOOP", None);
        add(&c, "c", "heartRateVariabilitySDNN", d, "17 02:00", 0, 40.0, "Zepp", None);
        add(&c, "e", "heartRateVariabilitySDNN", d, "17 04:00", 0, 80.0, "Zepp", None);
        add(&c, "f", "bodyMass", d, "17 07:00", 0, 80.0, "RENPHO Health", None);
        add(&c, "g", "bodyMass", d, "17 08:00", 0, 79.5, "RENPHO Health", None);
        let m = compute_day(&c, d).unwrap();
        assert_eq!(get(&m, "rhr").map(|x| (x.value, x.src.as_deref())), Some((53.0, Some("Zepp"))));
        assert_eq!(get(&m, "hrv").map(|x| (x.value, x.quality)), Some((60.0, 2)));
        assert_eq!(get(&m, "weight").map(|x| x.value), Some(79.5));
        assert!(get(&m, "hr_avg").is_none()); // kein Puls → keine Zeile statt 0
    }

    #[test]
    fn nacht_ueber_mitternacht_und_naechtlicher_puls() {
        let c = db();
        // Die Notch legt Abschnitte nach ihrem Ende ab: vor Mitternacht am 16., danach am 17.
        add(&c, "s1", "sleep", "2026-11-16", "16 23:00", 60, 3.0, "Zepp", None); // Kern 23–24
        add(&c, "s2", "sleep", "2026-11-17", "17 00:00", 120, 4.0, "Zepp", None); // Tief 0–2
        add(&c, "s3", "sleep", "2026-11-17", "17 02:00", 30, 2.0, "Zepp", None); // wach
        add(&c, "s4", "sleep", "2026-11-17", "17 02:30", 270, 5.0, "Zepp", None); // REM bis 7:00
        add(&c, "w1", "sleep", "2026-11-17", "16 22:30", 540, 0.0, "iPhone Nojo", None); // nur im Bett
        add(&c, "h1", "heartRate", "2026-11-17", "17 03:00", 0, 50.0, "Zepp", None);
        add(&c, "h2", "heartRate", "2026-11-17", "17 04:00", 0, 46.0, "Zepp", None);
        add(&c, "h3", "heartRate", "2026-11-17", "17 12:00", 0, 90.0, "Zepp", None); // tagsüber, zählt nicht
        // Nickerchen am Nachmittag des 17. gehört zum 17., der Abend ab 18 Uhr zum 18.
        add(&c, "n1", "sleep", "2026-11-17", "17 19:00", 20, 3.0, "Zepp", None);
        let m = compute_day(&c, "2026-11-17").unwrap();
        let v = |k| get(&m, k).map(|x| x.value);
        assert_eq!(v("sleep_min"), Some(450.0)); // 60 + 120 + 270
        assert_eq!(v("awake_min"), Some(30.0));
        assert_eq!(v("deep_min"), Some(120.0));
        assert_eq!(v("onset"), Some(-60.0)); // 23:00 am Vorabend
        assert_eq!(v("wake"), Some(420.0)); // 7:00
        assert_eq!(v("efficiency"), Some(93.75)); // 450 / 480
        assert_eq!(get(&m, "sleep_min").and_then(|x| x.src.as_deref()), Some("Zepp"));
        assert_eq!(v("hr_night"), Some(48.0));
        assert_eq!(v("hr_night_min"), Some(46.0));
        let next = compute_day(&c, "2026-11-18").unwrap();
        assert_eq!(get(&next, "sleep_min").map(|x| x.value), Some(20.0));
        // Nickerchen am Nachmittag verschiebt das Aufwachen nicht
        add(&c, "n2", "sleep", "2026-11-17", "17 15:00", 40, 1.0, "Zepp", None);
        let m = compute_day(&c, "2026-11-17").unwrap();
        assert_eq!(get(&m, "wake").map(|x| x.value), Some(420.0));
        assert_eq!(get(&m, "sleep_min").map(|x| x.value), Some(450.0));
        assert_eq!(get(&m, "nap_min").map(|x| x.value), Some(40.0));
    }

    #[test]
    fn puls_mittel_ueber_zeit() {
        let c = db();
        let d = "2026-11-17";
        // 10 Werte im Training binnen einer Minute, je ein Wert in drei ruhigen Stunden
        for i in 0..10 {
            add(&c, &format!("t{i}"), "heartRate", d, "17 18:00", 0, 160.0, "Zepp", None);
        }
        for (i, h) in ["17 08:00", "17 10:00", "17 12:00"].iter().enumerate() {
            add(&c, &format!("r{i}"), "heartRate", d, h, 0, 60.0, "Zepp", None);
        }
        let m = compute_day(&c, d).unwrap();
        assert_eq!(get(&m, "hr_avg").map(|x| x.value), Some(85.0)); // (160 + 3 × 60) / 4, nicht 136
    }

    #[test]
    fn trainings_mehrerer_apps_einmal() {
        let c = db();
        let d = "2026-11-17";
        add(&c, "a", "workout", d, "17 17:00", 30, 1800.0, "Zepp", Some(r#"{"wt":"running","kcal":300}"#));
        add(&c, "b", "workout", d, "17 17:02", 30, 1800.0, "Nike Run Club", Some(r#"{"wt":"running","kcal":320}"#));
        add(&c, "e", "workout", d, "17 20:00", 10, 600.0, "Zepp", None);
        let m = compute_day(&c, d).unwrap();
        let v = |k| get(&m, k).map(|x| x.value);
        assert_eq!((v("workout_n"), v("workout_min"), v("workout_kcal")), (Some(2.0), Some(42.0), Some(320.0)));
    }

    #[test]
    fn trainingslast_strap_vor_health() {
        assert_eq!([99.0, 100.0, 119.0, 120.0, 150.0, 180.0, 200.0, 230.0].map(edwards), [0.0, 1.0, 1.0, 2.0, 3.0, 5.0, 5.0, 5.0]);
        let c = db();
        let d = "2026-11-17";
        add(&c, "w", "workout", d, "17 17:00", 10, 600.0, "Zepp", None);
        // Health: alle 30 s Puls 150 (75 % → 3 je Minute)
        for i in 0..20 {
            add(&c, &format!("h{i}"), "heartRate", d, &format!("17 17:{:02}", i / 2), 0, 150.0, "Zepp", None);
        }
        // Strap in den ersten 5 Minuten: 190 (95 % → 5); Zeitstempel UTC = Ortszeit − 1 h
        let t0 = day_num(d).unwrap() * DAY + 16 * HOUR;
        for sec in (0..300).step_by(5) {
            c.execute("INSERT INTO hr VALUES (?1, 190)", [t0 + sec * 1000]).unwrap();
        }
        let m = compute_day(&c, d).unwrap();
        let t = get(&m, "trimp").unwrap();
        assert_eq!((t.value, t.src.as_deref(), t.quality), (40.0, Some("Zepp"), 2)); // 5 × 5 (Strap) + 5 × 3 (Health); Quelle „Helio Strap“ erst, wenn er die Mehrheit der Minuten liefert
    }

    #[test]
    fn stundenwerte() {
        let c = db();
        let d = "2026-11-17";
        add(&c, "a", "stepCount", d, "17 09:05", 10, 400.0, "iPhone Nojo", None);
        add(&c, "b", "stepCount", d, "17 09:10", 50, 650.0, "Zepp", None);
        add(&c, "e", "stepCount", d, "17 23:30", 10, 80.0, "Zepp", None);
        add(&c, "h1", "heartRate", d, "17 09:00", 0, 60.0, "Zepp", None);
        add(&c, "h2", "heartRate", d, "17 09:30", 0, 80.0, "Zepp", None);
        add(&c, "h3", "heartRate", d, "17 09:31", 0, 99.0, "iPhone Nojo", None); // schlechtere Quelle
        let mut h = compute(&c, d).unwrap().1;
        h.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));
        assert_eq!(h, vec![(9, "hr", 70.0), (9, "steps", 650.0), (23, "steps", 80.0)]);
    }

    #[test]
    fn essen_und_letzte_mahlzeit() {
        let c = db();
        let d = "2026-11-17";
        add(&c, "a", "dietaryEnergy", d, "17 08:00", 0, 500.0, "MacroFactor", None);
        add(&c, "b", "dietaryEnergy", d, "17 13:00", 0, 800.0, "MacroFactor", None);
        add(&c, "e", "dietaryEnergy", d, "17 21:30", 0, 400.0, "MacroFactor", None);
        let m = compute_day(&c, d).unwrap();
        assert_eq!(get(&m, "energy").map(|x| (x.value, x.quality)), Some((1700.0, 2)));
        assert_eq!(get(&m, "last_meal").map(|x| x.value), Some(1290.0)); // 21:30
        assert_eq!(get(&m, "caffeine_late"), None); // kein Koffein erfasst
        add(&c, "k1", "dietaryCaffeine", d, "17 09:00", 0, 80.0, "MacroFactor", None);
        assert_eq!(get(&compute_day(&c, d).unwrap(), "caffeine_late").map(|x| x.value), Some(0.0));
        add(&c, "k2", "dietaryCaffeine", d, "17 15:30", 0, 60.0, "MacroFactor", None);
        add(&c, "k3", "dietaryCaffeine", d, "17 13:59", 0, 40.0, "MacroFactor", None);
        assert_eq!(get(&compute_day(&c, d).unwrap(), "caffeine_late").map(|x| x.value), Some(60.0));
    }

    #[test]
    fn neu_berechnen_ersetzt() {
        let mut c = db();
        let d = "2026-11-17";
        add(&c, "a", "restingHeartRate", d, "17 00:00", 0, 53.0, "Zepp", None);
        let days: BTreeSet<String> = [d.to_string()].into();
        rebuild(&mut c, &days).unwrap();
        c.execute("UPDATE sample SET value = 55", []).unwrap();
        rebuild(&mut c, &days).unwrap();
        assert_eq!(day_of(&c, d).unwrap(), vec![metric("rhr", 55.0, Some("Zepp"), 2)]);
        assert_eq!(range_of(&c).unwrap(), Some((d.into(), d.into())));
        // neue Regeln: alles neu, auch der Folgetag (Nacht)
        c.execute("INSERT INTO daily VALUES ('1999-01-01', 'alt', 1, NULL, 2)", []).unwrap();
        ensure_rules(&mut c).unwrap();
        assert_eq!(range_of(&c).unwrap(), Some((d.into(), d.into())));
        assert_eq!(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), VERSION);
    }
}


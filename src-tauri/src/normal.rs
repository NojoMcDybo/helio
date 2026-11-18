//! Persönliche Normalbereiche und Schlafschuld für die Seite „Heute“ (docs/BAUPLAN-HEALTH.md, Block 3).
//! Grundlage ist die Tagesmatrix (daily.rs). Nur beschreibend: „höher/niedriger als üblich“, kein Gesamtwert.
//!
//! - Üblich = mittlere 80 % (10. bis 90. Perzentil) der 60 Tage davor, ab 14 Tagen mit Werten: „8 von 10 deiner Tage
//!   lagen hier“. Perzentile statt Median ± MAD (Bauplan), weil Schritte und Training schief verteilt sind und
//!   ± 2 MAD dort bis unter null reichte und kaum etwas fand (geprüft an den echten Daten, 18.11.2026).
//! - Schlafschuld: Bedarf = Median der Schlafdauer der 60 Nächte davor. Schuld = Schuld des Vortags × Abklingfaktor plus
//!   (Bedarf − Schlaf). Die Halbwertszeit wird am eigenen Verlauf gewählt: die, bei der hohe Schuld am stärksten
//!   mit längerem Schlaf in der Folgenacht einhergeht (Spearman, nur wenn deutlich). Ohne erkennbaren Ausgleich: Summe der letzten 7 Nächte.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::AppHandle;

use crate::daily::{day_num, day_str};
use crate::health;
use crate::stats::{median, quantile, spearman};

const WINDOW: i64 = 60;
const MIN_DAYS: usize = 14;
const SPARK: i64 = 28;

/// Werte der letzten Nacht (Aufwachtag) und des letzten vollständigen Tages.
const NIGHT: &[&str] = &["sleep_min", "onset", "wake", "efficiency", "deep_min", "rem_min", "hr_night", "hrv", "rhr", "resp", "spo2"];
const DAY: &[&str] = &["steps", "active_kcal", "workout_min", "energy", "protein", "carbs", "weight"];

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Usual {
    pub median: f64,
    pub lo: f64,
    pub hi: f64,
    pub n: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct Item {
    pub key: String,
    pub day: String,
    pub value: Option<f64>,
    pub src: Option<String>,
    pub quality: u8,
    pub usual: Option<Usual>,
    /// "höher" | "niedriger" | "üblich" | null (kein Wert oder kein Normalbereich)
    pub status: Option<&'static str>,
    /// Verlauf der letzten 28 Tage bis einschließlich `day`
    pub series: Vec<Option<f64>>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Debt {
    /// Minuten; positiv = weniger geschlafen als üblich
    pub minutes: f64,
    pub need: f64,
    /// Gewählte Halbwertszeit in Tagen, `None` = kein Ausgleich erkennbar, dann Summe der letzten 7 Nächte
    pub half_life: Option<f64>,
    pub rho: Option<f64>,
    pub nights: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct Today {
    pub night_day: Option<String>,
    pub day_day: Option<String>,
    pub items: Vec<Item>,
    pub debt: Option<Debt>,
    /// Frühzeichen der letzten Nacht (signal.rs)
    pub signal: Option<crate::signal::Signal>,
}

pub fn usual(vals: &[f64]) -> Option<Usual> {
    if vals.len() < MIN_DAYS {
        return None;
    }
    let mut v = vals.to_vec();
    let m = median(&mut v);
    Some(Usual { median: m, lo: quantile(&v, 0.1), hi: quantile(&v, 0.9), n: v.len() })
}

/// Tageswerte einer Kennzahl (Qualität ≥ 1) im Bereich [from, to].
fn values(c: &Connection, key: &str, from: &str, to: &str) -> rusqlite::Result<HashMap<String, f64>> {
    let mut st = c.prepare_cached("SELECT day, value FROM daily WHERE key = ?1 AND day >= ?2 AND day <= ?3 AND quality >= 1")?;
    let it = st.query_map(params![key, from, to], |r| Ok((r.get(0)?, r.get(1)?)))?;
    it.collect()
}

fn item(c: &Connection, key: &str, day: &str) -> rusqlite::Result<Item> {
    let dn = day_num(day).unwrap_or(0);
    let hist = values(c, key, &day_str(dn - WINDOW), &day_str(dn - 1))?;
    let prev: Vec<f64> = hist.values().copied().collect();
    let u = usual(&prev);
    let row: Option<(f64, Option<String>, u8)> = c
        .query_row("SELECT value, src, quality FROM daily WHERE day = ?1 AND key = ?2", params![day, key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let (value, src, quality) = row.map_or((None, None, 0), |(v, s, q)| (Some(v), s, q));
    let status = match (value, &u) {
        (Some(v), Some(u)) if quality >= 1 => {
            // knapp außerhalb (Rundung, z. B. SpO2 in ganzen Prozent) zählt nicht
            // knapp = höchstens 5 % der Bereichsbreite und höchstens 1 % des Medians (sonst verschluckt ein breiter Bereich klare Fälle)
            let m = (0.05 * (u.hi - u.lo)).min(0.01 * u.median.abs());
            Some(if v > u.hi + m { "höher" } else if v < u.lo - m { "niedriger" } else { "üblich" })
        }
        _ => None,
    };
    let spark = values(c, key, &day_str(dn - SPARK + 1), day)?;
    let series = (dn - SPARK + 1..=dn).map(|d| spark.get(&day_str(d)).copied()).collect();
    Ok(Item { key: key.into(), day: day.into(), value, src, quality, usual: u, status, series })
}

/// Schlafschuld bis einschließlich `day`. `sleep`: Schlafminuten je Tagnummer (Aufwachtag).
pub fn debt(sleep: &HashMap<i64, f64>, day: i64) -> Option<Debt> {
    let need_of = |d: i64| {
        let mut v: Vec<f64> = (d - WINDOW..d).filter_map(|x| sleep.get(&x).copied()).collect();
        (v.len() >= MIN_DAYS).then(|| median(&mut v))
    };
    let need = need_of(day)?;
    let first = *sleep.keys().min()?;
    // Fehlbetrag je Nacht mit eigenem Bedarf (gleitend), fehlende Nächte zählen nicht
    let deficit: Vec<(i64, f64)> = (first..=day).filter_map(|d| Some((d, need_of(d)? - sleep.get(&d)?))).collect();
    let mut best: Option<(f64, f64, usize)> = None; // (rho, half_life, n)
    for h in [1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0] {
        let r = 0.5f64.powf(1.0 / h);
        let (mut s, mut last, mut xs, mut ys) = (0.0, None::<i64>, vec![], vec![]);
        for &(d, x) in &deficit {
            if let Some(l) = last {
                s *= r.powi((d - l) as i32);
                if d - l == 1 {
                    // Schuld bis gestern gegen heutigen Mehrschlaf (= −Fehlbetrag)
                    xs.push(s);
                    ys.push(-x);
                }
            }
            s += x;
            last = Some(d);
        }
        if xs.len() >= 60 {
            if let Some(rho) = spearman(&xs, &ys) {
                if best.is_none_or(|b| rho > b.0) {
                    best = Some((rho, h, xs.len()));
                }
            }
        }
    }
    let nights = deficit.iter().filter(|(d, _)| *d > day - 7).count();
    match best {
        // nur, wenn der Ausgleich nicht Zufall sein kann (ρ·√(n−1) ≥ 2, etwa p < 0,05)
        Some((rho, h, n)) if rho * ((n - 1) as f64).sqrt() >= 2.0 => {
            let r = 0.5f64.powf(1.0 / h);
            let m = deficit.iter().map(|&(d, x)| x * r.powi((day - d) as i32)).sum();
            Some(Debt { minutes: m, need, half_life: Some(h), rho: Some(rho), nights })
        }
        _ => {
            let m = deficit.iter().filter(|(d, _)| *d > day - 7).map(|(_, x)| x).sum();
            Some(Debt { minutes: m, need, half_life: None, rho: best.map(|b| b.0), nights })
        }
    }
}

pub fn today_of(c: &Connection, today: &str) -> rusqlite::Result<Today> {
    // Letzte Nacht: jüngster Tag mit Schlaf- oder Erholungswerten (Ruhepuls kommt auch ohne getragene Nacht)
    let keys = NIGHT.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(",");
    let night_day: Option<String> =
        c.query_row(&format!("SELECT MAX(day) FROM daily WHERE key IN ({keys}) AND day <= ?1"), [today], |r| r.get(0))?;
    // Der heutige Tag läuft noch: Bewegung und Essen vom letzten abgeschlossenen Tag
    let yesterday = day_num(today).map(|d| day_str(d - 1)).unwrap_or_default();
    let day_day = c.query_row("SELECT MAX(day) FROM daily WHERE day <= ?1", [&yesterday], |r| r.get::<_, Option<String>>(0))?;
    let mut items = Vec::new();
    for (keys, day) in [(NIGHT, &night_day), (DAY, &day_day)] {
        let Some(day) = day else { continue };
        for k in keys {
            let it = item(c, k, day)?;
            if it.value.is_some() || it.usual.is_some() {
                items.push(it);
            }
        }
    }
    let debt = match night_day.as_deref().and_then(day_num) {
        Some(dn) => {
            let mut st = c.prepare("SELECT day, value FROM daily WHERE key = 'sleep_min' AND quality = 2")?;
            let sleep: HashMap<i64, f64> = st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?
                .filter_map(|x| x.ok().and_then(|(d, v)| Some((day_num(&d)?, v))))
                .collect();
            debt(&sleep, dn)
        }
        None => None,
    };
    let signal = match &night_day {
        Some(d) => crate::signal::signal_of(c, d)?,
        None => None,
    };
    Ok(Today { night_day, day_day, items, debt, signal })
}

/// `today`: heutiges Datum in Ortszeit (JJJJ-MM-TT), kommt aus der Oberfläche.
#[tauri::command]
pub async fn today_view(app: AppHandle, today: String) -> Result<Today, String> {
    today_of(&health::conn(&app)?, &today).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        health::migrate(&c).unwrap();
        c
    }
    fn put(c: &Connection, day: i64, key: &str, v: f64) {
        c.execute("INSERT OR REPLACE INTO daily VALUES (?1, ?2, ?3, 'Zepp', 2)", params![day_str(day), key, v]).unwrap();
    }
    /// gleichmäßig gestreute Werte um `m` (±3) ohne Zufall
    fn wobble(i: i64, m: f64) -> f64 {
        m + [-3.0, -1.0, 0.0, 1.0, 3.0, 2.0, -2.0][(i % 7) as usize]
    }

    #[test]
    fn normalbereich_robust() {
        assert_eq!(usual(&[60.0; 13]), None); // zu wenig Tage
        let mut v: Vec<f64> = (0..30).map(|i| wobble(i, 55.0)).collect();
        v.push(120.0); // ein Ausreißer verschiebt den Bereich kaum
        let u = usual(&v).unwrap();
        assert_eq!(u.median, 55.0);
        assert_eq!((u.lo, u.hi), (52.0, 58.0));
        // schief verteilt: der Bereich bleibt im Wertebereich, nicht unter null
        let u = usual(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 30.0, 45.0, 60.0, 90.0, 120.0, 0.0]).unwrap();
        assert_eq!((u.lo, u.median), (0.0, 0.0));
        assert!(u.hi > 60.0 && u.hi < 120.0);
    }

    #[test]
    fn abweichung_wird_gefunden_normales_nicht() {
        let c = db();
        let today = day_num("2026-11-18").unwrap();
        for d in today - 70..today {
            put(&c, d, "rhr", wobble(d, 55.0));
            put(&c, d, "hrv", wobble(d, 70.0));
            put(&c, d, "sleep_min", wobble(d, 450.0));
            put(&c, d, "steps", wobble(d, 8000.0) * 1.0);
        }
        put(&c, today, "rhr", 64.0); // deutlich höher
        put(&c, today, "hrv", 69.0); // normal
        put(&c, today, "sleep_min", 452.0);
        let t = today_of(&c, "2026-11-18").unwrap();
        assert_eq!(t.night_day.as_deref(), Some("2026-11-18"));
        assert_eq!(t.day_day.as_deref(), Some("2026-11-17")); // heute läuft noch
        let st = |k: &str| t.items.iter().find(|i| i.key == k).and_then(|i| i.status);
        assert_eq!((st("rhr"), st("hrv"), st("sleep_min"), st("steps")), (Some("höher"), Some("üblich"), Some("üblich"), Some("üblich")));
        assert_eq!(t.items.iter().find(|i| i.key == "rhr").unwrap().series.len(), 28);
        // knapp unter dem Bereich (Rundung) zählt nicht, deutlich darunter schon
        put(&c, today, "spo2", 0.0);
        for d in today - 30..today {
            put(&c, d, "spo2", 98.0 + (d % 2) as f64);
            put(&c, d, "active_kcal", [100.0, 300.0, 900.0, 1600.0][(d % 4) as usize]);
        }
        put(&c, today, "spo2", 97.99);
        put(&c, today - 1, "active_kcal", 34.0);
        let t = today_of(&c, "2026-11-18").unwrap();
        let st = |k: &str| t.items.iter().find(|i| i.key == k).and_then(|i| i.status);
        assert_eq!((st("spo2"), st("active_kcal")), (Some("üblich"), Some("niedriger")));
    }

    #[test]
    fn zu_wenig_verlauf() {
        let c = db();
        let today = day_num("2026-11-18").unwrap();
        for d in today - 5..=today {
            put(&c, d, "rhr", 55.0);
        }
        let t = today_of(&c, "2026-11-18").unwrap();
        let rhr = t.items.iter().find(|i| i.key == "rhr").unwrap();
        assert_eq!((rhr.usual.as_ref(), rhr.status, rhr.value), (None, None, Some(55.0)));
        assert_eq!(t.night_day.as_deref(), Some("2026-11-18"));
        assert_eq!(t.debt, None); // keine Schlafwerte
    }

    #[test]
    fn schlafschuld_mit_ausgleich() {
        // Nach kurzen Nächten wird länger geschlafen: der Ausgleich ist erkennbar, die Schuld wird gewählt und abgeklungen
        let mut s = HashMap::new();
        let mut debt_now = 0.0f64;
        for d in 0..200i64 {
            let short = d % 5 == 0 || d % 7 == 0;
            let v = if short { 360.0 } else { 450.0 + (debt_now * 0.4).min(90.0) };
            debt_now = debt_now * 0.8 + (450.0 - v);
            s.insert(d, v);
        }
        let x = debt(&s, 199).unwrap();
        assert!((440.0..470.0).contains(&x.need), "{x:?}");
        assert!(x.half_life.is_some() && x.rho.unwrap() > 0.2, "{x:?}");
        // Ohne Ausgleich (Schlaf unabhängig): Summe der letzten 7 Nächte
        let mut z = 88172645463325252u64;
        let mut rnd = || {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            (z % 1000) as f64 / 1000.0
        };
        let flat: HashMap<i64, f64> = (0..200).map(|d| (d, 380.0 + 120.0 * rnd())).collect();
        let y = debt(&flat, 199).unwrap();
        assert_eq!(y.half_life, None);
        assert_eq!(y.nights, 7);
        // zu wenig Nächte
        assert_eq!(debt(&(0..10).map(|d| (d, 400.0)).collect(), 9), None);
    }
}



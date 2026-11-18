//! Belastung und Erholung (docs/BAUPLAN-HEALTH.md, Block 4). Grundlage: Trainingslast `trimp` der Tagesmatrix (daily.rs),
//! Trainings und Puls aus Health, Strap-Puls aus `hr`. Nur beschreibend, keine Trainingsempfehlung.
//!
//! - Fitness und Ermüdung nach Banister: gleitende Last über 42 bzw. 7 Tage (exponentiell), Form = Fitness − Ermüdung.
//!   Tage ohne Training zählen als 0.
//! - Pulserholung: Abfall 60 und 120 s nach dem Ende eines Trainings ab 10 min, wenn der Puls am Ende ≥ 60 % HFmax lag.
//!   Strap-Puls, wenn vorhanden, sonst Health (Zepp misst nach dem Training weiter jede Sekunde).
//! - Nach harten Tagen: Ruhepuls und HRV 1 und 2 Tage nach Tagen mit Trainingslast ab dem 80. Perzentil, als Abstand zum
//!   eigenen Median der 60 Tage davor, gegen dieselbe Größe nach allen übrigen Tagen. Mit 95-%-Unsicherheit.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::AppHandle;

use crate::daily::{self, day_num, day_str, Row, COLS, HRMAX};
use crate::health;
use crate::normal::usual;
use crate::stats::t975;

const FIT_DAYS: f64 = 42.0;
const FAT_DAYS: f64 = 7.0;
const SHOW_DAYS: i64 = 90;
const RECOVERY_DAYS: i64 = 180;
const SEC: i64 = 1000;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct LoadDay {
    pub day: String,
    pub trimp: f64,
    pub fitness: f64,
    pub fatigue: f64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Recovery {
    pub day: String,
    /// Ende des Trainings, ms seit 1970 (UTC)
    pub end: i64,
    pub minutes: f64,
    pub hr_end: f64,
    pub drop60: f64,
    pub drop120: Option<f64>,
    pub src: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct After {
    pub key: String,
    /// Tage nach dem harten Tag
    pub lag: i64,
    pub n: usize,
    pub mean: f64,
    /// halbe Breite des 95-%-Bereichs
    pub ci: f64,
    pub other_n: usize,
    pub other_mean: f64,
}

#[derive(Serialize, Debug, Clone)]
pub struct LoadView {
    pub days: Vec<LoadDay>,
    pub recoveries: Vec<Recovery>,
    pub after: Vec<After>,
    /// Schwelle „harter Tag“ (Trainingslast)
    pub hard: Option<f64>,
}

fn by_day(c: &Connection, key: &str) -> rusqlite::Result<HashMap<i64, f64>> {
    let mut st = c.prepare_cached("SELECT day, value FROM daily WHERE key = ?1 AND quality >= 1")?;
    let it = st.query_map([key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?;
    let mut m = HashMap::new();
    for x in it {
        let (d, v) = x?;
        if let Some(n) = day_num(&d) {
            m.insert(n, v);
        }
    }
    Ok(m)
}

/// Fitness (42 Tage) und Ermüdung (7 Tage) für jeden Tag ab der ersten Trainingslast bis `to`.
pub fn banister(trimp: &HashMap<i64, f64>, to: i64) -> Vec<LoadDay> {
    let Some(&first) = trimp.keys().min() else { return vec![] };
    let (kf, ka) = (1.0 - (-1.0 / FIT_DAYS).exp(), 1.0 - (-1.0 / FAT_DAYS).exp());
    let (mut f, mut a) = (0.0, 0.0);
    (first..=to)
        .map(|d| {
            let t = trimp.get(&d).copied().unwrap_or(0.0);
            f += (t - f) * kf;
            a += (t - a) * ka;
            LoadDay { day: day_str(d), trimp: t, fitness: f, fatigue: a }
        })
        .collect()
}

fn mean_in(v: &[(i64, f64)], from: i64, to: i64) -> Option<f64> {
    let x: Vec<f64> = v.iter().filter(|p| p.0 >= from && p.0 <= to).map(|p| p.1).collect();
    (x.len() >= 2).then(|| x.iter().sum::<f64>() / x.len() as f64)
}

/// Pulserholung aus Pulswerten (Zeit relativ zum Trainingsende in ms, bpm).
pub fn recovery_of(v: &[(i64, f64)]) -> Option<(f64, f64, Option<f64>)> {
    let end = mean_in(v, -15 * SEC, 0)?;
    let at60 = mean_in(v, 50 * SEC, 70 * SEC)?;
    (end >= 0.6 * HRMAX).then(|| (end, end - at60, mean_in(v, 110 * SEC, 130 * SEC).map(|x| end - x)))
}

fn recoveries(c: &Connection, from: i64, to: i64) -> rusqlite::Result<Vec<Recovery>> {
    let w = daily::rows(c, &format!("{COLS} WHERE type = 'workout' AND day >= ?1 AND day <= ?2"), params![day_str(from), day_str(to)])?;
    let refs: Vec<&Row> = w.iter().collect();
    // Ortszeit-Versatz je Trainingsbeginn, um Health-Puls über den Index (type, lstart) zu finden
    let offset: HashMap<i64, i64> = w.iter().map(|r| (r.start, r.ls - r.start)).collect();
    let mut strap_st = c.prepare_cached("SELECT ts, bpm FROM hr WHERE ts >= ?1 AND ts <= ?2")?;
    let mut out = Vec::new();
    for (s, e, _) in daily::workout_groups(&refs) {
        if e - s < 10 * 60 * SEC {
            continue;
        }
        let off = offset.get(&s).copied().unwrap_or(0);
        let strap: Vec<(i64, f64)> = strap_st
            .query_map(params![e - 30 * SEC, e + 150 * SEC], |r| Ok((r.get::<_, i64>(0)? - e, r.get::<_, f64>(1)?)))?
            .collect::<Result<_, _>>()?;
        let (pts, src) = if strap.len() >= 30 {
            (strap, "Helio Strap".to_string())
        } else {
            let hr = daily::rows(
                c,
                &format!("{COLS} WHERE type = 'heartRate' AND lstart >= ?1 AND lstart <= ?2"),
                params![e + off - 30 * SEC, e + off + 150 * SEC],
            )?;
            let hr_refs: Vec<&Row> = hr.iter().collect();
            let Some(src) = daily::best_source(&hr_refs) else { continue };
            (hr.iter().filter(|r| r.src == src).map(|r| (r.start - e, r.v)).collect(), src.to_string())
        };
        if let Some((hr_end, drop60, drop120)) = recovery_of(&pts) {
            let day = day_str((e + off).div_euclid(86_400_000));
            out.push(Recovery { day, end: e, minutes: (e - s) as f64 / 60_000.0, hr_end, drop60, drop120, src });
        }
    }
    Ok(out)
}

fn mean_ci(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    if v.len() < 2 {
        return (m, f64::NAN);
    }
    let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    (m, t975(n - 1.0) * sd / n.sqrt())
}

/// Ruhepuls/HRV nach harten Tagen. Gibt die Schwelle und je Kennzahl und Abstand die Mittelwerte zurück.
pub fn after_hard(trimp: &HashMap<i64, f64>, series: &[(&str, HashMap<i64, f64>)], from: i64, to: i64) -> (Option<f64>, Vec<After>) {
    let mut loads: Vec<f64> = (from..=to).filter_map(|d| trimp.get(&d).copied()).filter(|t| *t > 0.0).collect();
    if loads.len() < 10 {
        return (None, vec![]);
    }
    loads.sort_by(f64::total_cmp);
    let hard = loads[((loads.len() - 1) as f64 * 0.8).round() as usize];
    let mut out = Vec::new();
    for (key, vals) in series {
        for lag in 1..=2 {
            let (mut h, mut o) = (vec![], vec![]);
            for d in from..=to {
                let Some(v) = vals.get(&(d + lag)) else { continue };
                let prev: Vec<f64> = (d - 60..d).filter_map(|x| vals.get(&x).copied()).collect();
                let Some(u) = usual(&prev) else { continue };
                let dev = v - u.median;
                if trimp.get(&d).copied().unwrap_or(0.0) >= hard { h.push(dev) } else { o.push(dev) }
            }
            if h.len() >= 3 && !o.is_empty() {
                let (mean, ci) = mean_ci(&h);
                out.push(After { key: key.to_string(), lag, n: h.len(), mean, ci, other_n: o.len(), other_mean: mean_ci(&o).0 });
            }
        }
    }
    (Some(hard), out)
}

pub fn load_of(c: &Connection, today: &str) -> rusqlite::Result<LoadView> {
    let Some(to) = day_num(today) else { return Ok(LoadView { days: vec![], recoveries: vec![], after: vec![], hard: None }) };
    let trimp = by_day(c, "trimp")?;
    let all = banister(&trimp, to);
    let days = all.into_iter().filter(|d| day_num(&d.day).is_some_and(|n| n > to - SHOW_DAYS)).collect();
    let recoveries = recoveries(c, to - RECOVERY_DAYS, to)?;
    let series = [("rhr", by_day(c, "rhr")?), ("hrv", by_day(c, "hrv")?)];
    let (hard, after) = after_hard(&trimp, &series, to - 365, to);
    Ok(LoadView { days, recoveries, after, hard })
}

/// `today`: heutiges Datum in Ortszeit (JJJJ-MM-TT), kommt aus der Oberfläche.
#[tauri::command]
pub async fn load_view(app: AppHandle, today: String) -> Result<LoadView, String> {
    load_of(&health::conn(&app)?, &today).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitness_und_ermuedung() {
        // 30 Tage jeden Tag 100: Ermüdung (7 Tage) holt schneller auf als Fitness (42 Tage)
        let t: HashMap<i64, f64> = (0..30).map(|d| (d, 100.0)).collect();
        let b = banister(&t, 39);
        assert_eq!(b.len(), 40);
        let d29 = &b[29];
        assert!(d29.fatigue > 98.0 && d29.fitness > 50.0 && d29.fitness < 55.0, "{d29:?}");
        // 10 Tage Pause: Ermüdung fällt deutlich, Fitness kaum
        let d39 = &b[39];
        assert!(d39.fatigue < 25.0 && d39.fitness > 40.0, "{d39:?}");
        assert!(banister(&HashMap::new(), 5).is_empty());
    }

    #[test]
    fn pulserholung() {
        // 1 Wert je Sekunde: 160 am Ende, fällt linear um 0,5/s
        let v: Vec<(i64, f64)> = (-30..=150).map(|s| (s * SEC, if s <= 0 { 160.0 } else { 160.0 - 0.5 * s as f64 })).collect();
        let (end, d60, d120) = recovery_of(&v).unwrap();
        assert_eq!((end, d60, d120), (160.0, 30.0, Some(60.0)));
        // zu locker (unter 60 % HFmax) oder ohne Werte nach 60 s: keine Messung
        let easy: Vec<(i64, f64)> = v.iter().map(|&(t, b)| (t, b - 60.0)).collect();
        assert_eq!(recovery_of(&easy), None);
        assert_eq!(recovery_of(&v[..60]), None);
    }

    #[test]
    fn nach_harten_tagen() {
        // Alle 5 Tage hart (300), sonst 50; Ruhepuls am Tag danach +4, sonst ±1
        let trimp: HashMap<i64, f64> = (0..200).map(|d| (d, if d % 5 == 0 { 300.0 } else { 50.0 })).collect();
        let rhr: HashMap<i64, f64> = (0..200).map(|d| (d, if d % 5 == 1 { 59.0 } else { 55.0 + (d % 3) as f64 - 1.0 })).collect();
        let (hard, after) = after_hard(&trimp, &[("rhr", rhr)], 100, 190);
        assert_eq!(hard, Some(300.0));
        let lag1 = after.iter().find(|a| a.lag == 1).unwrap();
        assert!((lag1.mean - 4.0).abs() < 0.6 && lag1.ci < 1.0 && lag1.n >= 15, "{lag1:?}");
        assert!(lag1.other_mean.abs() < 1.0);
        // zu wenig Trainingstage
        assert_eq!(after_hard(&HashMap::new(), &[], 0, 10), (None, vec![]));
    }
}



//! Prognose-Experiment (docs/BAUPLAN-HEALTH.md, Block 8): Ruhepuls und HRV der nächsten 7 Tage. NICHT BELASTBAR.
//!
//! Vorgehen wie Haze `shared/analysis/forecast.mjs` (Haze 1.11, eigener Code), auf Tageswerte übertragen: drei einfache
//! Modelle, Rückwärtsprüfung auf den letzten 90 Tagen, die das gelernte Modell nicht kennt; gezeigt wird das beste nur,
//! wenn es die Fortschreibung um mindestens 5 % schlägt, sonst ehrlich die Fortschreibung. Das 80-%-Band stammt aus den
//! echten Fehlern der Prüfung. Statt AR auf Differenzen (Minutenwerte) lernt Helio die Rückkehr zum Üblichen je Abstand.
//! TTM aus Haze 1.12 ist nicht übernommen: Es braucht 360 Werte am Stück, Helio hat höchstens rund 60 Tage am Stück.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::AppHandle;

use crate::daily::{day_num, day_str};
use crate::health;
use crate::stats::{mean, median, quantile};

const KEYS: &[&str] = &["rhr", "hrv"];
/// Tage voraus
const H: usize = 7;
const TEST_DAYS: i64 = 90;
/// Bezug für „üblich“: Median der 28 Tage bis zum Starttag
const BASE_DAYS: i64 = 28;
const SHOW_DAYS: i64 = 28;

type Series = HashMap<i64, f64>;

#[derive(Serialize, Debug, Clone)]
pub struct Score {
    pub key: &'static str,
    pub label: &'static str,
    /// mittlerer quadratischer Fehler je Horizont (1–7 Tage)
    pub rmse: Vec<Option<f64>>,
    /// Mittel der RMSE über die Horizonte
    pub score: Option<f64>,
}

#[derive(Serialize, Debug, Clone)]
pub struct Point {
    pub day: String,
    pub value: f64,
    pub lo: f64,
    pub hi: f64,
}

#[derive(Serialize, Debug, Clone)]
pub struct Forecast {
    pub key: &'static str,
    /// Startpunkte der Rückwärtsprüfung (Tage mit Wert in den letzten 90)
    pub origins: usize,
    pub enough: bool,
    pub best: &'static str,
    /// um so viel besser als die Fortschreibung (0 = Fortschreibung gezeigt)
    pub gain: f64,
    pub models: Vec<Score>,
    /// die letzten 28 Tage bis `last`
    pub from: String,
    pub last: Option<String>,
    pub series: Vec<Option<f64>>,
    pub points: Vec<Point>,
}

fn base(s: &Series, t: i64) -> Option<f64> {
    let mut v: Vec<f64> = (t - BASE_DAYS + 1..=t).filter_map(|d| s.get(&d).copied()).collect();
    (v.len() >= 10).then(|| median(&mut v))
}
fn mean_of(s: &Series, t: i64, days: i64, need: usize) -> Option<f64> {
    let v: Vec<f64> = (t - days + 1..=t).filter_map(|d| s.get(&d).copied()).collect();
    (v.len() >= need).then(|| mean(&v))
}

/// Gelernt: Rückkehr zum Üblichen. y(t+h) − üblich = a_h · (Mittel der letzten 3 Tage − üblich), a_h per kleinster Quadrate.
#[derive(Debug, Clone)]
struct Fit {
    a: [f64; H],
}

fn fit(s: &Series, until: i64) -> Option<Fit> {
    let mut a = [0.0; H];
    for (h, a) in a.iter_mut().enumerate() {
        let (mut sxy, mut sxx, mut n) = (0.0, 0.0, 0);
        // feste Reihenfolge: gleiche Daten, gleiche Summe
        let mut ts: Vec<i64> = s.keys().copied().filter(|&t| t + h as i64 + 1 < until).collect();
        ts.sort_unstable();
        for t in ts {
            let (Some(m), Some(r), Some(y)) = (base(s, t), mean_of(s, t, 3, 1), s.get(&(t + h as i64 + 1))) else { continue };
            sxy += (r - m) * (y - m);
            sxx += (r - m) * (r - m);
            n += 1;
        }
        if n < 60 || sxx <= 0.0 {
            return None;
        }
        *a = sxy / sxx;
    }
    Some(Fit { a })
}

const MODELS: &[(&str, &str)] = &[("naive", "Wie zuletzt"), ("mean7", "Mittel der letzten 7 Tage"), ("learned", "Gelernt: Rückkehr zum Üblichen")];

/// Vorhersage von Modell `k` am Starttag `t` für `h` Tage voraus (1-basiert).
fn run(k: &str, f: Option<&Fit>, s: &Series, t: i64, h: usize) -> Option<f64> {
    let last = *s.get(&t)?;
    match k {
        "naive" => Some(last),
        "mean7" => mean_of(s, t, 7, 4),
        _ => {
            let (f, m, r) = (f?, base(s, t)?, mean_of(s, t, 3, 1)?);
            Some(m + f.a[h - 1] * (r - m))
        }
    }
}

#[derive(Debug)]
struct Eval {
    origins: usize,
    /// Fehler (Vorhersage − Wahrheit) je Modell und Horizont
    err: Vec<[Vec<f64>; H]>,
    fit: Option<Fit>,
}

/// Rückwärtsprüfung: Starttage in den letzten `TEST_DAYS` bis `to`; das gelernte Modell sieht nur Tage davor.
/// Alle Modelle auf denselben Startpunkten (nur wo jedes eine Vorhersage hat).
fn evaluate(s: &Series, to: i64) -> Eval {
    let test_from = to - TEST_DAYS + 1;
    let fit = fit(s, test_from);
    let mut err: Vec<[Vec<f64>; H]> = MODELS.iter().map(|_| Default::default()).collect();
    let mut origins = 0;
    for t in test_from..to {
        if !s.contains_key(&t) {
            continue;
        }
        origins += 1;
        for h in 1..=H {
            let Some(&truth) = s.get(&(t + h as i64)) else { continue };
            if t + h as i64 > to {
                continue;
            }
            let preds: Vec<Option<f64>> = MODELS.iter().map(|m| run(m.0, fit.as_ref(), s, t, h)).collect();
            // gelernt fehlt ganz, wenn zu wenig Lerndaten: dann vergleichen die übrigen untereinander
            if preds.iter().zip(MODELS).any(|(p, m)| p.is_none() && !(m.0 == "learned" && fit.is_none())) {
                continue;
            }
            for (i, p) in preds.iter().enumerate() {
                if let Some(p) = p {
                    err[i][h - 1].push(p - truth);
                }
            }
        }
    }
    Eval { origins, err, fit }
}

fn rmse(e: &[f64]) -> Option<f64> {
    (e.len() >= 10).then(|| (e.iter().map(|x| x * x).sum::<f64>() / e.len() as f64).sqrt())
}

fn sorted(e: &[f64]) -> Vec<f64> {
    let mut v = e.to_vec();
    v.sort_by(f64::total_cmp);
    v
}

pub fn forecast_of(s: &Series, key: &'static str, to: i64) -> Forecast {
    let ev = evaluate(s, to);
    let models: Vec<Score> = MODELS
        .iter()
        .zip(&ev.err)
        .map(|(&(k, label), e)| {
            let rmse: Vec<Option<f64>> = e.iter().map(|x| rmse(x)).collect();
            let score = rmse.iter().all(|r| r.is_some()).then(|| mean(&rmse.iter().flatten().copied().collect::<Vec<_>>()));
            Score { key: k, label, rmse, score }
        })
        .collect();
    let naive = models[0].score;
    let mut best = 0;
    for (i, m) in models.iter().enumerate().skip(1) {
        if let (Some(sc), Some(nv)) = (m.score, naive) {
            if sc <= nv * 0.95 && sc < models[best].score.unwrap_or(f64::INFINITY) {
                best = i;
            }
        }
    }
    let enough = naive.is_some() && ev.err[0][0].len() >= 30;
    let gain = match (models[best].score, naive) {
        (Some(b), Some(n)) if best > 0 => 1.0 - b / n,
        _ => 0.0,
    };

    // Vorhersage ab dem letzten Tag mit Wert, wenn er höchstens 2 Tage zurückliegt
    let last = s.keys().filter(|&&d| d <= to).max().copied();
    let mut points = Vec::new();
    if let Some(l) = last.filter(|&l| enough && to - l <= 2) {
        for h in 1..=H {
            let e = sorted(&ev.err[best][h - 1]);
            let Some(v) = run(MODELS[best].0, ev.fit.as_ref(), s, l, h) else { continue };
            if e.len() < 10 {
                continue;
            }
            // Fehler = Vorhersage − Wahrheit, also Wahrheit = Vorhersage − Fehler
            points.push(Point { day: day_str(l + h as i64), value: v, lo: v - quantile(&e, 0.9), hi: v - quantile(&e, 0.1) });
        }
    }
    let end = last.unwrap_or(to);
    let from = end - SHOW_DAYS + 1;
    Forecast {
        key,
        origins: ev.origins,
        enough,
        best: MODELS[best].0,
        gain,
        models,
        from: day_str(from),
        last: last.map(day_str),
        series: (from..=end).map(|d| s.get(&d).copied()).collect(),
        points,
    }
}

pub fn forecasts(c: &Connection, today: &str) -> rusqlite::Result<Vec<Forecast>> {
    let Some(to) = day_num(today) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for &k in KEYS {
        let mut st = c.prepare("SELECT day, value FROM daily WHERE key = ?1 AND quality >= 1 AND day <= ?2")?;
        let s: Series = st
            .query_map(params![k, today], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?
            .filter_map(|x| x.ok().and_then(|(d, v)| Some((day_num(&d)?, v))))
            .collect();
        if !s.is_empty() {
            out.push(forecast_of(&s, k, to));
        }
    }
    Ok(out)
}

/// `today`: heutiges Datum in Ortszeit (JJJJ-MM-TT), kommt aus der Oberfläche. Nur aufgerufen, wenn das Experiment an ist.
#[tauri::command]
pub async fn forecast_view(app: AppHandle, today: String) -> Result<Vec<Forecast>, String> {
    forecasts(&health::conn(&app)?, &today).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Rng;

    /// annähernd normalverteilt, Mittel 0, Varianz 0,5
    fn gauss(r: &mut Rng) -> f64 {
        (0..6).map(|_| r.next()).sum::<f64>() - 3.0
    }
    /// um 60 schwankend, kehrt mit φ zurück (φ = 1: Irrfahrt)
    fn series(n: i64, phi: f64, seed: u32) -> Series {
        let mut r = Rng::new(seed);
        let mut x = 0.0;
        (0..n)
            .map(|d| {
                x = phi * x + 3.0 * gauss(&mut r);
                (d, 60.0 + x)
            })
            .collect()
    }

    #[test]
    fn gelerntes_modell_schlaegt_fortschreibung_bei_rueckkehr() {
        let s = series(400, 0.5, 1);
        let f = forecast_of(&s, "rhr", 399);
        assert!(f.enough);
        assert_ne!(f.best, "naive", "{:?}", f.models);
        assert!(f.gain >= 0.05);
        assert_eq!(f.points.len(), 7);
        assert!(f.points.iter().all(|p| p.lo <= p.value && p.value <= p.hi));
    }

    #[test]
    fn irrfahrt_zeigt_fortschreibung() {
        let f = forecast_of(&series(400, 1.0, 2), "rhr", 399);
        assert_eq!(f.best, "naive", "{:?}", f.models);
        assert_eq!(f.gain, 0.0);
    }

    #[test]
    fn band_trifft_rund_80_prozent_in_der_folgezeit() {
        let s = series(700, 0.6, 3);
        let ev = evaluate(&s, 399);
        let f = forecast_of(&s, "rhr", 399);
        let k = MODELS.iter().position(|m| m.0 == f.best).unwrap();
        let (mut hit, mut n) = (0, 0);
        for t in 400..690 {
            for h in 1..=H {
                let e = sorted(&ev.err[k][h - 1]);
                let v = run(f.best, ev.fit.as_ref(), &s, t, h).unwrap();
                let y = s[&(t + h as i64)];
                hit += usize::from(v - quantile(&e, 0.9) <= y && y <= v - quantile(&e, 0.1));
                n += 1;
            }
        }
        let share = hit as f64 / n as f64;
        assert!((0.7..=0.9).contains(&share), "{share}");
    }

    #[test]
    fn zu_wenig_oder_alte_werte_ohne_vorhersage() {
        let few: Series = (0..20).map(|d| (d, 60.0)).collect();
        let f = forecast_of(&few, "rhr", 19);
        assert!(!f.enough);
        assert!(f.points.is_empty());
        // letzter Wert vor 5 Tagen: keine Vorhersage, Prüfung bleibt
        let s = series(400, 0.5, 4);
        let f = forecast_of(&s, "rhr", 404);
        assert!(f.points.is_empty());
        assert_eq!(f.series.len(), SHOW_DAYS as usize);
    }

    #[test]
    fn lernt_nur_vor_dem_pruefzeitraum() {
        // Nach dem Prüfbeginn ändert sich alles; das gelernte a darf davon nichts wissen
        let mut s = series(400, 0.5, 5);
        let a = fit(&s, 400 - TEST_DAYS + 1).unwrap().a;
        for d in 311..400 {
            s.insert(d, 200.0 - d as f64 * 0.1);
        }
        assert_eq!(fit(&s, 400 - TEST_DAYS + 1).unwrap().a, a);
    }
}

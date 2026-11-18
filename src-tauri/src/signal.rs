//! Frühzeichen: Nachtwerte weichen über mehrere Nächte gemeinsam ab, wie oft vor einem Infekt. Beschreibung, kein Befund.
//!
//! Vorgehen nach Studien mit Smartwatch-Daten (Mishra et al. 2020, Nat. Biomed. Eng.; Alavi et al. 2022, Nat. Med. „NightSignal“;
//! Quer et al. 2021, Nat. Med.): Ruhe- und Nachtpuls und Atemfrequenz steigen, HRV sinkt, oft 1–3 Tage vor Symptomen. Welche
//! Krankheit, lässt sich daraus nicht sagen; dasselbe Muster entsteht nach Alkohol, hartem Training oder kurzem Schlaf.
//! Je Nacht: Abstand jedes Werts zu deinem Üblichen (Median der Tage 8–60 davor, robuste Streuung), gedeckelt bei 4, gemittelt.
//! Auffällig ist eine Nacht, wenn der Mittelwert ≥ `THRESHOLD` ist und mindestens zwei Werte, darunter ein Puls, je ≥ 1 abweichen;
//! Hinweis nach zwei auffälligen Nächten hintereinander. Die Regel ist fest und wird nicht an deinen Krank-Tagen eingestellt
//! (nur gegen zu viele Hinweise in Nojos Daten gewählt: erste Fassung 18, diese 8 in 11 Monaten); der Rückblick zeigt, wie oft sie traf.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::daily::{day_num, day_str};
use crate::health;
use crate::stats::{median, quantile};

/// Kennzahl, Richtung (+1: höher fällt auf), kleinste Streuung
const PARTS: &[(&str, f64, f64)] = &[("hr_night", 1.0, 1.0), ("rhr", 1.0, 1.0), ("resp", 1.0, 0.3), ("hrv", -1.0, 3.0)];
const THRESHOLD: f64 = 1.2;
/// Bezug: Tage `d − BASE_FROM` bis `d − BASE_TO`; die letzte Woche zählt nicht, sonst verschluckt ein beginnender Infekt sich selbst
const BASE_FROM: i64 = 60;
const BASE_TO: i64 = 8;
const MIN_BASE: usize = 14;
/// Krank-Phase gilt als vorher erkannt, wenn ein Hinweis bis zu 3 Tage vor ihrem Beginn oder am ersten Tag kam
const LEAD: i64 = 3;
const HISTORY: i64 = 28;

type Series = HashMap<i64, f64>;

#[derive(Serialize, Debug, Clone)]
pub struct Part {
    pub key: &'static str,
    pub value: f64,
    pub usual: f64,
    /// Abstand in Streuungen, in Richtung „auffällig“ positiv
    pub z: f64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Check {
    /// markierte Krank-Phasen mit Nachtwerten davor
    pub episodes: usize,
    /// davon mit Hinweis bis 3 Tage vorher oder am ersten Tag
    pub caught: usize,
    /// Hinweis-Phasen insgesamt
    pub runs: usize,
    /// davon ohne markierte Krankheit in der Nähe
    pub unmarked: usize,
    /// Jahre mit Nachtwerten
    pub years: f64,
}

#[derive(Serialize, Debug, Clone)]
pub struct Signal {
    pub day: String,
    pub active: bool,
    /// Nächte in Folge über der Schwelle
    pub nights: usize,
    pub score: Option<f64>,
    /// Schwelle für den Mittelwert (für die Linie im Verlauf)
    pub threshold: f64,
    pub parts: Vec<Part>,
    /// was dasselbe Muster auch erklären könnte: "alcohol", "training", "short_sleep"
    pub maybe: Vec<&'static str>,
    /// Mittelwert je Nacht, die letzten 28 bis `day`
    pub history: Vec<Option<f64>>,
    pub check: Check,
}

fn spread(v: &mut [f64], m: f64) -> f64 {
    let mut dev: Vec<f64> = v.iter().map(|x| (x - m).abs()).collect();
    median(&mut dev) * 1.4826
}

/// Abweichungen einer Nacht; `None`, wenn kein Pulswert oder weniger als zwei Kennzahlen mit Bezug.
fn parts(data: &HashMap<&str, Series>, d: i64) -> Option<Vec<Part>> {
    let mut out = Vec::new();
    for &(key, dir, floor) in PARTS {
        let Some(s) = data.get(key) else { continue };
        let Some(&value) = s.get(&d) else { continue };
        let mut base: Vec<f64> = (d - BASE_FROM..=d - BASE_TO).filter_map(|x| s.get(&x).copied()).collect();
        if base.len() < MIN_BASE {
            continue;
        }
        let usual = median(&mut base);
        let sd = spread(&mut base, usual).max(floor);
        out.push(Part { key, value, usual, z: dir * (value - usual) / sd });
    }
    (out.len() >= 2 && out.iter().any(|p| p.key == "hr_night" || p.key == "rhr")).then_some(out)
}

/// Mittelwert der Abstände (je Wert gedeckelt, ein Ausreißer allein trägt nicht) und ob die Nacht auffällig ist
fn night(p: &[Part]) -> (f64, bool) {
    let score = p.iter().map(|x| x.z.clamp(-4.0, 4.0)).sum::<f64>() / p.len() as f64;
    let up: Vec<&Part> = p.iter().filter(|x| x.z >= 1.0).collect();
    (score, score >= THRESHOLD && up.len() >= 2 && up.iter().any(|x| x.key == "hr_night" || x.key == "rhr"))
}

type Nights = HashMap<i64, (f64, bool)>;

/// Hinweis-Tage: zwei auffällige Nächte hintereinander (fehlende Nacht unterbricht)
fn alerts(n: &Nights) -> BTreeSet<i64> {
    let high = |d: i64| n.get(&d).is_some_and(|x| x.1);
    n.keys().copied().filter(|&d| high(d) && high(d - 1)).collect()
}

/// Zusammenhängende Tage (Lücke ≤ `gap`) als (Anfang, Ende)
fn runs(days: &BTreeSet<i64>, gap: i64) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = Vec::new();
    for &d in days {
        match out.last_mut() {
            Some(r) if d - r.1 <= gap => r.1 = d,
            _ => out.push((d, d)),
        }
    }
    out
}

fn check(scores: &Nights, alert: &BTreeSet<i64>, sick: &BTreeSet<i64>) -> Check {
    let hits = runs(alert, 1);
    // Krank-Tage mit bis zu 2 Tagen Lücke sind eine Phase; erkannt zählt nur, wo davor Nachtwerte da waren
    let sick_runs = runs(sick, 3);
    let episodes: Vec<&(i64, i64)> = sick_runs.iter().filter(|e| (e.0 - LEAD..=e.0).any(|d| scores.contains_key(&d))).collect();
    let caught = episodes.iter().filter(|e| alert.range(e.0 - LEAD..=e.0).next().is_some()).count();
    let unmarked = hits.iter().filter(|r| !sick_runs.iter().any(|e| r.0 >= e.0 - LEAD && r.0 <= e.1)).count();
    Check { episodes: episodes.len(), caught, runs: hits.len(), unmarked, years: scores.len() as f64 / 365.0 }
}

fn load(c: &Connection, to: &str) -> rusqlite::Result<HashMap<&'static str, Series>> {
    let mut out = HashMap::new();
    let mut st = c.prepare_cached("SELECT day, value FROM daily WHERE key = ?1 AND quality >= 1 AND day <= ?2")?;
    for key in PARTS.iter().map(|p| p.0).chain(["alcohol", "trimp", "sleep_min"]) {
        let s: Series = st
            .query_map(params![key, to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?
            .filter_map(|x| x.ok().and_then(|(d, v)| Some((day_num(&d)?, v))))
            .collect();
        out.insert(key, s);
    }
    Ok(out)
}

pub fn sick_days(c: &Connection) -> rusqlite::Result<BTreeSet<i64>> {
    let mut st = c.prepare("SELECT day FROM mark WHERE kind = 'krank'")?;
    let days = st.query_map([], |r| r.get::<_, String>(0))?.filter_map(|d| d.ok().and_then(|d| day_num(&d))).collect();
    Ok(days)
}

/// Was die Nacht `d` auch erklären könnte: Alkohol am Vorabend, Training am Vortag über deinem P90, Nacht unter deinem P10.
fn maybe(data: &HashMap<&str, Series>, d: i64) -> Vec<&'static str> {
    let prior = |key: &str, q: f64| {
        let s = data.get(key)?;
        // vor dem fraglichen Vortag, damit er sich nicht selbst die Schwelle setzt
        let mut v: Vec<f64> = (d - 1 - BASE_FROM..d - 1).filter_map(|x| s.get(&x).copied()).filter(|&x| x > 0.0).collect();
        v.sort_by(f64::total_cmp);
        (v.len() >= MIN_BASE).then(|| quantile(&v, q))
    };
    let at = |key: &str, x: i64| data.get(key).and_then(|s| s.get(&x).copied());
    let mut out = Vec::new();
    if at("alcohol", d - 1).is_some_and(|a| a > 0.0) {
        out.push("alcohol");
    }
    if let (Some(t), Some(p90)) = (at("trimp", d - 1), prior("trimp", 0.9)) {
        if t >= p90 {
            out.push("training");
        }
    }
    if let (Some(s), Some(p10)) = (at("sleep_min", d), prior("sleep_min", 0.1)) {
        if s < p10 {
            out.push("short_sleep");
        }
    }
    out
}

pub fn signal_of(c: &Connection, night_day: &str) -> rusqlite::Result<Option<Signal>> {
    let Some(day) = day_num(night_day) else { return Ok(None) };
    let data = load(c, night_day)?;
    let nights: BTreeSet<i64> = data.get("hr_night").into_iter().chain(data.get("rhr")).flat_map(|s| s.keys().copied()).collect();
    let scores: Nights = nights.iter().filter_map(|&d| Some((d, night(&parts(&data, d)?)))).collect();
    let alert = alerts(&scores);
    let p = parts(&data, day).unwrap_or_default();
    let in_run = (0..).take_while(|i| scores.get(&(day - i)).is_some_and(|x| x.1)).count();
    let active = alert.contains(&day);
    let mut why: Vec<&'static str> = if active { maybe(&data, day).into_iter().chain(maybe(&data, day - 1)).collect() } else { Vec::new() };
    why.sort_unstable();
    why.dedup();
    Ok(Some(Signal {
        day: night_day.into(),
        active,
        nights: in_run,
        score: scores.get(&day).map(|x| x.0),
        threshold: THRESHOLD,
        parts: p,
        maybe: why,
        history: (day - HISTORY + 1..=day).map(|d| scores.get(&d).map(|x| x.0)).collect(),
        check: check(&scores, &alert, &sick_days(c)?),
    }))
}

/// Krank-Tage für die Seite Tage
#[tauri::command]
pub async fn marks(app: AppHandle) -> Result<Vec<String>, String> {
    let days = sick_days(&health::conn(&app)?).map_err(|e| e.to_string())?;
    Ok(days.into_iter().map(day_str).collect())
}

/// Tag als krank markieren oder Markierung entfernen. Nur Helios eigene Datenbank.
#[tauri::command]
pub async fn mark_sick(app: AppHandle, day: String, sick: bool) -> Result<(), String> {
    day_num(&day).ok_or("Ungültiger Tag")?;
    let c = health::conn(&app)?;
    let r = if sick {
        c.execute("INSERT OR REPLACE INTO mark (day, kind) VALUES (?1, 'krank')", [&day])
    } else {
        c.execute("DELETE FROM mark WHERE day = ?1", [&day])
    };
    r.map_err(|e| e.to_string())?;
    let _ = app.emit("marks", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Rng;

    fn noise(r: &mut Rng) -> f64 {
        (0..6).map(|_| r.next()).sum::<f64>() - 3.0
    }
    /// 300 ruhige Nächte; an `ill` (Tage) Infekt-Muster
    fn data(ill: &[i64], seed: u32) -> HashMap<&'static str, Series> {
        let mut r = Rng::new(seed);
        let mut d: HashMap<&'static str, Series> = HashMap::new();
        for day in 0..300 {
            let k = if ill.contains(&day) { 1.0 } else { 0.0 };
            d.entry("hr_night").or_default().insert(day, 58.0 + 2.0 * noise(&mut r) + 6.0 * k);
            d.entry("rhr").or_default().insert(day, 55.0 + 2.0 * noise(&mut r) + 5.0 * k);
            d.entry("resp").or_default().insert(day, 14.0 + 0.4 * noise(&mut r) + 1.2 * k);
            d.entry("hrv").or_default().insert(day, 70.0 + 10.0 * noise(&mut r) - 20.0 * k);
        }
        d
    }
    fn scores(d: &HashMap<&str, Series>) -> Nights {
        (0..300).filter_map(|x| Some((x, night(&parts(d, x)?)))).collect()
    }

    #[test]
    fn infekt_muster_loest_hinweis_aus_ruhige_naechte_kaum() {
        let ill = [200, 201, 202, 203];
        let s = scores(&data(&ill, 1));
        let a = alerts(&s);
        assert!(a.contains(&201) && a.contains(&203), "{a:?}");
        // ohne Muster: wenige Fehlalarme (zwei Nächte über 1 Streuung im Mittel sind selten)
        let quiet = alerts(&scores(&data(&[], 2)));
        assert!(runs(&quiet, 1).len() <= 3, "{quiet:?}");
    }

    #[test]
    fn eine_nacht_reicht_nicht_und_luecke_unterbricht() {
        let s: Nights = [(10, (2.0, true)), (11, (0.2, false)), (12, (2.0, true)), (14, (2.0, true))].into();
        assert!(alerts(&s).is_empty());
        // nur ein Wert auffällig (Atemfrequenz-Ausreißer): Nacht zählt nicht
        let p = |key, z| Part { key, value: 0.0, usual: 0.0, z };
        assert!(!night(&[p("rhr", 0.5), p("resp", 9.0)]).1);
        assert!(!night(&[p("resp", 2.0), p("hrv", 2.0), p("rhr", 0.0)]).1, "ohne Puls");
        assert!(night(&[p("rhr", 1.5), p("resp", 1.5)]).1);
    }

    #[test]
    fn ohne_puls_kein_wert() {
        let mut d = data(&[], 3);
        d.remove("hr_night");
        d.remove("rhr");
        assert!(parts(&d, 100).is_none());
        let d = data(&[], 3);
        assert!(parts(&d, 5).is_none(), "zu wenig Bezugstage");
        assert_eq!(parts(&d, 100).unwrap().len(), 4);
    }

    #[test]
    fn rueckblick_zaehlt_erkannte_und_unmarkierte() {
        let scores: Nights = (0..400).map(|d| (d, (0.0, false))).collect();
        let alert: BTreeSet<i64> = [99, 100, 250, 251].into();
        // krank ab 101 (Hinweis 2 Tage vorher), krank ab 300 ohne Hinweis
        let sick: BTreeSet<i64> = [101, 102, 103, 300, 301].into();
        assert_eq!(check(&scores, &alert, &sick), Check { episodes: 2, caught: 1, runs: 2, unmarked: 1, years: 400.0 / 365.0 });
    }

    #[test]
    fn moegliche_andere_gruende() {
        let mut d = data(&[], 4);
        d.entry("alcohol").or_default().insert(149, 2.0);
        d.insert("trimp", (80..150).map(|x| (x, 50.0 + (x % 10) as f64)).collect());
        d.get_mut("trimp").unwrap().insert(149, 200.0);
        d.insert("sleep_min", (80..151).map(|x| (x, 420.0 + (x % 7) as f64 * 10.0)).collect());
        d.get_mut("sleep_min").unwrap().insert(150, 300.0);
        assert_eq!(maybe(&d, 150), vec!["alcohol", "training", "short_sleep"]);
        assert!(maybe(&d, 115).is_empty());
    }
}

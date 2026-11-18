//! Langzeit und Zusammenhänge (docs/BAUPLAN-HEALTH.md, Block 6). Nur beschreibend: was zusammen auftritt, nicht was es verursacht.
//!
//! Vorgehen wie Haze `shared/analysis/longterm.mjs` und `insights.mjs` (Haze 1.9): Wochen- und Monatsmittel, Trend über die
//! letzten 52 Wochen (Theil-Sen, Mann-Kendall, FDR, nur spürbare Änderungen), Wendepunkte im letzten Jahr (binäre Segmentierung
//! mit Langzeitvarianz), feste Fragen mit Spearman, Permutationstest, Bootstrap-Intervall und FDR. Statt der Bayes-Umbrüche aus
//! dem Bauplan die Haze-Wendepunkte (getestet, gleiche Aussage „ab wann dauerhaft verschoben“). Zusätzlich eine Bestätigung im
//! neuesten Drittel der Tage. Granger-Tests sind nicht gebaut.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::AppHandle;

use crate::daily::{day_num, day_str};
use crate::health;
use crate::rhythm::weekday;
use crate::stats::{self, median};

/// Kennzahl und kleinster spürbarer Unterschied (in ihrer Einheit; Uhrzeiten und Dauern in Minuten).
pub const METRICS: &[(&str, f64)] = &[
    ("sleep_min", 20.0),
    ("onset", 20.0),
    ("wake", 20.0),
    ("deep_min", 10.0),
    ("efficiency", 2.0),
    ("hr_night", 2.0),
    ("rhr", 2.0),
    ("hrv", 5.0),
    ("resp", 0.5),
    ("spo2", 1.0),
    ("steps", 1000.0),
    ("active_kcal", 50.0),
    ("workout_min", 10.0),
    ("trimp", 20.0),
    ("vo2max", 1.0),
    ("energy", 150.0),
    ("weight", 1.0),
];

/// Feste Fragen: x am Tag d, y am Tag d + lag. Nachtwerte gehören zum Aufwachtag, „Nacht danach“ ist also lag 1.
pub const QUESTIONS: &[(&str, &str, i64)] = &[
    ("caffeine_late", "onset", 1),
    ("caffeine_late", "deep_min", 1),
    ("alcohol", "hrv", 1),
    ("alcohol", "hr_night", 1),
    ("last_meal", "deep_min", 1),
    ("last_meal", "hr_night", 1),
    ("trimp", "deep_min", 1),
    ("trimp", "hrv", 1),
    ("sleep_min", "rhr", 0),
    ("sleep_min", "hrv", 0),
    ("onset_dev", "rhr", 0),
    ("onset_dev", "hrv", 0),
];

const TREND_WEEKS: i64 = 52;
const MIN_WEEKS_TREND: usize = 8;
const VALID_WEEK: usize = 4;
const VALID_MONTH: usize = 10;
const MIN_PAIRS: usize = 14;
const PERMUTATIONS: usize = 999;

#[derive(Serialize, Debug, Clone)]
pub struct Trend {
    pub n: usize,
    pub per_month: f64,
    /// Änderung über die gefundenen Wochen (Theil-Sen-Steigung × Spanne)
    pub change: f64,
    pub p: f64,
    pub q: f64,
    /// „steigend“, „fallend“ oder „kein klarer Trend“
    pub status: &'static str,
    pub slope: f64,
    pub intercept: f64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Change {
    pub day: String,
    pub before: f64,
    pub after: f64,
}

#[derive(Serialize, Debug, Clone)]
pub struct Metric {
    pub key: &'static str,
    /// Mittel der letzten 28 Tage
    pub now: Option<f64>,
    /// Wochenmittel der letzten 52 Wochen (ab 4 Tagen mit Wert)
    pub weeks: Vec<Option<f64>>,
    /// Monatsmittel über alle Jahre (ab 10 Tagen mit Wert): (JJJJ-MM, Mittel, Tage)
    pub months: Vec<(String, f64, usize)>,
    pub trend: Option<Trend>,
    pub changes: Vec<Change>,
}

#[derive(Serialize, Debug, Clone)]
pub struct Link {
    pub x: &'static str,
    pub y: &'static str,
    pub lag: i64,
    pub n: usize,
    pub rho: Option<f64>,
    pub ci: Option<(f64, f64)>,
    pub p: Option<f64>,
    pub q: Option<f64>,
    /// Gruppen: unteres Drittel von x bis `low_max`, oberes ab `high_min`; Mediane von y in beiden.
    /// `zero`: x ist an vielen Tagen 0 (Alkohol, Koffein am Nachmittag), dann Tage ohne (x = 0) gegen Tage mit (x > 0).
    pub zero: bool,
    pub low_max: Option<f64>,
    pub high_min: Option<f64>,
    pub low_median: Option<f64>,
    pub high_median: Option<f64>,
    /// „zu wenige Daten“, „Zusammenhang“ oder „kein deutlicher Zusammenhang“
    pub status: &'static str,
    /// nur bei Zusammenhang: im neuesten Drittel der Tage gleiche Richtung und p < 0,05
    pub confirmed: Option<bool>,
}

#[derive(Serialize, Debug, Clone)]
pub struct LongtermView {
    pub to: String,
    pub first: String,
    pub week_starts: Vec<String>,
    pub metrics: Vec<Metric>,
    pub links: Vec<Link>,
}

type Series = HashMap<i64, f64>;

fn load(c: &Connection, to: i64) -> rusqlite::Result<HashMap<String, Series>> {
    // nur die gebrauchten Schlüssel: Kennzahlen, Fragen und die Tage, an denen fehlende Einträge als 0 gelten
    let mut keys: Vec<&str> = METRICS.iter().map(|m| m.0).chain(QUESTIONS.iter().flat_map(|q| [q.0, q.1])).chain(["energy", "hr_avg", "workout_n"]).collect();
    keys.sort_unstable();
    keys.dedup();
    let list = keys.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
    let mut out: HashMap<String, Series> = HashMap::new();
    let mut st = c.prepare(&format!("SELECT day, key, value FROM daily WHERE quality >= 1 AND day <= ?1 AND key IN ({list})"))?;
    for x in st.query_map(params![day_str(to)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?)))? {
        let (d, k, v) = x?;
        if let Some(dn) = day_num(&d) {
            out.entry(k).or_default().insert(dn, v);
        }
    }
    Ok(out)
}

/// Abgeleitete Werte für die Fragen: fehlende Einträge als 0, wo der Tag sonst erfasst ist; Abweichung der Einschlafzeit.
fn derive(data: &mut HashMap<String, Series>) {
    let fill = |data: &mut HashMap<String, Series>, key: &str, when: &str| {
        let days: Vec<i64> = data.get(when).map(|s| s.keys().copied().collect()).unwrap_or_default();
        let s = data.entry(key.into()).or_default();
        for d in days {
            s.entry(d).or_insert(0.0);
        }
    };
    // Essen erfasst, aber kein Alkohol/Koffein eingetragen = keiner; Puls erfasst, aber kein Training = Last 0
    fill(data, "alcohol", "energy");
    fill(data, "caffeine_late", "energy");
    // Trainingstage ohne verwertbare Last (zu wenig Puls im Training) bleiben offen statt 0
    let open: Vec<i64> = data.get("workout_n").map(|w| w.keys().filter(|d| !data.get("trimp").is_some_and(|t| t.contains_key(d))).copied().collect()).unwrap_or_default();
    fill(data, "trimp", "hr_avg");
    if let Some(t) = data.get_mut("trimp") {
        for d in open {
            t.remove(&d);
        }
    }
    let onset = data.get("onset").cloned().unwrap_or_default();
    let dev = onset
        .iter()
        .filter_map(|(&d, &v)| {
            let mut prev: Vec<f64> = (d - 7..d).filter_map(|x| onset.get(&x).copied()).collect();
            (prev.len() >= 4).then(|| (d, (v - median(&mut prev)).abs()))
        })
        .collect();
    data.insert("onset_dev".into(), dev);
}

/// Trend über die Wochen; q und Einordnung folgen nach der FDR-Korrektur über alle Kennzahlen.
fn trend(weeks: &[Option<f64>]) -> Option<Trend> {
    let (x, y): (Vec<f64>, Vec<f64>) = weeks.iter().enumerate().filter_map(|(i, v)| v.map(|v| (i as f64, v))).unzip();
    if x.len() < MIN_WEEKS_TREND {
        return None;
    }
    let (slope, intercept) = stats::theil_sen(&x, &y)?;
    let change = slope * (x[x.len() - 1] - x[0]);
    Some(Trend { n: x.len(), per_month: slope * 30.44 / 7.0, change, p: stats::mann_kendall(&y), q: 1.0, status: "kein klarer Trend", slope, intercept })
}

fn changes(s: &Series, from: i64, to: i64, meaningful: f64) -> Vec<Change> {
    let days: Vec<i64> = (from..=to).filter(|d| s.contains_key(d)).collect();
    let v: Vec<f64> = days.iter().map(|d| s[d]).collect();
    let cuts = stats::change_points(&v, 14, 4.0, 3);
    let bounds: Vec<usize> = std::iter::once(0).chain(cuts.iter().copied()).chain([v.len()]).collect();
    cuts.iter()
        .enumerate()
        .filter_map(|(i, &c)| {
            let before = median(&mut v[bounds[i]..c].to_vec());
            let after = median(&mut v[c..bounds[i + 2]].to_vec());
            ((after - before).abs() >= meaningful).then(|| Change { day: day_str(days[c]), before, after })
        })
        .collect()
}

fn metric(key: &'static str, s: &Series, week_starts: &[i64], to: i64) -> Metric {
    let mean_of = |from: i64, upto: i64, min: usize| {
        let v: Vec<f64> = (from..=upto).filter_map(|d| s.get(&d).copied()).collect();
        (v.len() >= min).then(|| stats::mean(&v))
    };
    let weeks: Vec<Option<f64>> = week_starts.iter().map(|&w| mean_of(w, (w + 6).min(to), VALID_WEEK)).collect();
    let mut by_month: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (&d, &v) in s.iter().filter(|(&d, _)| d <= to) {
        by_month.entry(day_str(d)[..7].to_string()).or_default().push(v);
    }
    let months = by_month.into_iter().filter(|(_, v)| v.len() >= VALID_MONTH).map(|(m, v)| (m, stats::mean(&v), v.len())).collect();
    let meaningful = METRICS.iter().find(|m| m.0 == key).map_or(0.0, |m| m.1);
    Metric { key, now: mean_of(to - 27, to, 7), trend: trend(&weeks), weeks, months, changes: changes(s, to - 364, to, meaningful) }
}

fn link(data: &HashMap<String, Series>, (x, y, lag): (&'static str, &'static str, i64), seed: u32) -> Link {
    let empty = Series::new();
    let (sx, sy) = (data.get(x).unwrap_or(&empty), data.get(y).unwrap_or(&empty));
    let mut pairs: Vec<(i64, f64, f64)> = sx.iter().filter_map(|(&d, &a)| sy.get(&(d + lag)).map(|&b| (d, a, b))).collect();
    pairs.sort_by_key(|p| p.0);
    let mut l = Link {
        x, y, lag, n: pairs.len(), rho: None, ci: None, p: None, q: None, zero: false, low_max: None, high_min: None, low_median: None, high_median: None,
        status: "zu wenige Daten", confirmed: None,
    };
    let distinct = |f: fn(&(i64, f64, f64)) -> f64| {
        let mut v: Vec<f64> = pairs.iter().map(f).collect();
        v.sort_by(f64::total_cmp);
        v.dedup();
        v.len()
    };
    let zeros = pairs.iter().filter(|p| p.1 == 0.0).count();
    l.zero = zeros * 3 >= pairs.len();
    let mut by_x: Vec<(f64, f64)> = pairs.iter().map(|p| (p.1, p.2)).collect();
    by_x.sort_by(|a, b| a.0.total_cmp(&b.0));
    let k = by_x.len() / 3;
    let (lo, hi) = if l.zero { by_x.split_at(zeros) } else { (&by_x[..k], &by_x[by_x.len() - k..]) };
    if pairs.len() < MIN_PAIRS || distinct(|p| p.1) < if l.zero { 2 } else { 4 } || distinct(|p| p.2) < 3 || lo.len() < 5 || hi.len() < 5 {
        return l;
    }
    let (xs, ys): (Vec<f64>, Vec<f64>) = pairs.iter().map(|p| (p.1, p.2)).unzip();
    let t = stats::spearman_test(&xs, &ys, PERMUTATIONS, PERMUTATIONS, seed);
    l.low_max = lo.last().map(|p| p.0);
    l.high_min = hi.first().map(|p| p.0);
    l.low_median = Some(median(&mut lo.iter().map(|p| p.1).collect::<Vec<_>>()));
    l.high_median = Some(median(&mut hi.iter().map(|p| p.1).collect::<Vec<_>>()));
    (l.rho, l.ci, l.p, l.status) = (t.rho, t.ci, Some(t.p), "geprüft");
    // Bestätigung: neuestes Drittel der Tage für sich
    let recent = &pairs[pairs.len() - pairs.len() / 3..];
    if recent.len() >= MIN_PAIRS {
        let (rx, ry): (Vec<f64>, Vec<f64>) = recent.iter().map(|p| (p.1, p.2)).unzip();
        let r = stats::spearman_test(&rx, &ry, PERMUTATIONS, 0, seed);
        l.confirmed = Some(r.p < 0.05 && r.rho.zip(t.rho).is_some_and(|(a, b)| a.signum() == b.signum()));
    }
    l
}

pub fn longterm_of(c: &Connection, today: &str) -> rusqlite::Result<Option<LongtermView>> {
    let Some(t) = day_num(today) else { return Ok(None) };
    let to = t - 1;
    let mut data = load(c, to)?;
    let Some(first) = data.values().flat_map(|s| s.keys()).min().copied() else { return Ok(None) };
    let week_starts: Vec<i64> = (0..TREND_WEEKS).map(|i| to - weekday(to) - (TREND_WEEKS - 1 - i) * 7).collect();
    let empty = Series::new();
    let mut metrics: Vec<Metric> = METRICS.iter().map(|&(k, _)| metric(k, data.get(k).unwrap_or(&empty), &week_starts, to)).collect();
    // Trends: eine FDR-Korrektur über alle Kennzahlen
    let q = stats::fdr(&metrics.iter().filter_map(|m| m.trend.as_ref().map(|t| t.p)).collect::<Vec<_>>());
    for ((m, &(_, meaningful)), q) in metrics.iter_mut().zip(METRICS).filter(|(m, _)| m.trend.is_some()).zip(q) {
        let t = m.trend.as_mut().unwrap();
        t.q = q;
        if q <= 0.1 && t.change.abs() >= meaningful {
            t.status = if t.change > 0.0 { "steigend" } else { "fallend" };
        }
    }

    derive(&mut data);
    let mut links: Vec<Link> = QUESTIONS.iter().enumerate().map(|(i, &qn)| link(&data, qn, 7 + i as u32)).collect();
    let tested: Vec<usize> = (0..links.len()).filter(|&i| links[i].status == "geprüft").collect();
    let q = stats::fdr(&tested.iter().map(|&i| links[i].p.unwrap_or(1.0)).collect::<Vec<_>>());
    for (&i, q) in tested.iter().zip(q) {
        let l = &mut links[i];
        l.q = Some(q);
        // wie Haze: |ρ| ≥ 0,3, q ≤ 0,1, Intervall ohne 0, Gruppen getrennt
        let clear = l.rho.is_some_and(|r| r.abs() >= 0.3)
            && q <= 0.1
            && l.ci.is_some_and(|c| c.0 > 0.0 || c.1 < 0.0)
            && l.high_min > l.low_max;
        l.status = if clear { "Zusammenhang" } else { "kein deutlicher Zusammenhang" };
        if !clear {
            l.confirmed = None;
        }
    }
    Ok(Some(LongtermView { to: day_str(to), first: day_str(first), week_starts: week_starts.iter().map(|&d| day_str(d)).collect(), metrics, links }))
}

/// `today`: heutiges Datum in Ortszeit (JJJJ-MM-TT), kommt aus der Oberfläche.
#[tauri::command]
pub async fn longterm_view(app: AppHandle, today: String) -> Result<Option<LongtermView>, String> {
    longterm_of(&health::conn(&app)?, &today).map_err(|e| e.to_string())
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
    fn noise(n: usize, seed: u32) -> Vec<f64> {
        let mut r = stats::Rng::new(seed);
        (0..n).map(|_| (0..6).map(|_| r.next()).sum::<f64>() - 3.0).collect()
    }

    #[test]
    fn trend_und_wendepunkt() {
        let c = db();
        let today = day_num("2026-11-18").unwrap();
        let n = noise(400, 3);
        for i in 0..400 {
            let d = today - 400 + i as i64;
            // Ruhepuls steigt über das Jahr um 8 bpm, HRV springt vor 100 Tagen um −15 ms, Schritte nur Rauschen
            put(&c, d, "rhr", 55.0 + 8.0 * i as f64 / 400.0 + n[i]);
            put(&c, d, "hrv", 50.0 + if i >= 300 { -15.0 } else { 0.0 } + 3.0 * n[(i + 7) % 400]);
            put(&c, d, "steps", 8000.0 + 1500.0 * n[(i + 13) % 400]);
        }
        let v = longterm_of(&c, "2026-11-18").unwrap().unwrap();
        let m = |k: &str| v.metrics.iter().find(|m| m.key == k).unwrap();
        assert_eq!(v.week_starts.len(), 52);
        assert_eq!(weekday(day_num(&v.week_starts[0]).unwrap()), 0);
        let rhr = m("rhr").trend.as_ref().unwrap();
        assert_eq!(rhr.status, "steigend", "{rhr:?}");
        assert!((rhr.per_month - 8.0 / 400.0 * 30.44).abs() < 0.2, "{rhr:?}");
        assert_eq!(m("steps").trend.as_ref().unwrap().status, "kein klarer Trend");
        let hrv = &m("hrv").changes;
        assert_eq!(hrv.len(), 1, "{hrv:?}");
        assert!(day_num(&hrv[0].day).unwrap().abs_diff(today - 100) <= 3 && (hrv[0].after - hrv[0].before + 15.0).abs() < 3.0, "{hrv:?}");
        assert!(m("steps").changes.is_empty());
        assert!(m("rhr").months.len() >= 12 && m("weight").months.is_empty());
    }

    #[test]
    fn eingebauter_zusammenhang_wird_gefunden_rauschen_nicht() {
        let c = db();
        let today = day_num("2026-11-18").unwrap();
        let n = noise(600, 5);
        for i in 0..200 {
            let d = today - 200 + i as i64;
            // Training am Tag d senkt die HRV der Nacht danach; Alkohol nur Rauschen; Essen erfasst, Alkohol nur an manchen Tagen
            let load = (i * 37 % 100) as f64;
            put(&c, d, "trimp", load);
            put(&c, d + 1, "hrv", 60.0 - 0.2 * load + 4.0 * n[i]);
            put(&c, d, "energy", 2000.0);
            if i % 4 == 0 {
                put(&c, d, "alcohol", 1.0 + (i % 3) as f64 + n[200 + i].abs());
            }
            put(&c, d, "sleep_min", 420.0 + 30.0 * n[400 + i]);
            put(&c, d, "rhr", 55.0 + n[(i * 7) % 600]);
        }
        let v = longterm_of(&c, "2026-11-18").unwrap().unwrap();
        let l = |x: &str, y: &str| v.links.iter().find(|l| l.x == x && l.y == y).unwrap();
        let t = l("trimp", "hrv");
        assert_eq!((t.status, t.confirmed), ("Zusammenhang", Some(true)), "{t:?}");
        assert!(t.rho.unwrap() < -0.5 && t.high_median < t.low_median);
        // Alkohol: Tage ohne Eintrag zählen als 0, weil Essen erfasst ist
        let a = l("alcohol", "hrv");
        assert!(a.n > 150 && a.status == "kein deutlicher Zusammenhang", "{a:?}");
        assert!(a.zero && a.low_max == Some(0.0) && a.high_min.unwrap() >= 1.0, "{a:?}"); // ohne gegen mit
        assert!(!t.zero);
        assert_eq!(l("sleep_min", "rhr").status, "kein deutlicher Zusammenhang");
        assert_eq!(l("caffeine_late", "onset").status, "zu wenige Daten");
        // Abweichung der Einschlafzeit braucht 4 Vornächte
        let mut data: HashMap<String, Series> = HashMap::new();
        data.insert("onset".into(), (0..6).map(|d| (d, if d == 5 { 60.0 } else { 0.0 })).collect());
        data.insert("hr_avg".into(), (0..6).map(|d| (d, 60.0)).collect());
        data.insert("workout_n".into(), [(1, 1.0), (2, 1.0)].into());
        data.insert("trimp".into(), [(2, 80.0)].into());
        derive(&mut data);
        // Tag 1: Training ohne Last bleibt offen, Tag 0: kein Training = 0
        assert_eq!((data["trimp"].get(&0), data["trimp"].get(&1), data["trimp"].get(&2)), (Some(&0.0), None, Some(&80.0)));
        assert_eq!(data["onset_dev"].get(&3), None);
        assert_eq!((data["onset_dev"][&4], data["onset_dev"][&5]), (0.0, 60.0));
    }
}


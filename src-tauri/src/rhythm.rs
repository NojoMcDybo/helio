//! Rhythmus und Schlafregelmäßigkeit (docs/BAUPLAN-HEALTH.md, Block 5). Grundlage: Stundenwerte (`hourly`, daily.rs),
//! Schlafabschnitte aus Health und die Tagesmatrix. Nur beschreibend.
//!
//! Verfahren (nach den Originalarbeiten neu geschrieben, kein Code übernommen; Auswahl wie in
//! github.com/labrinyang/apple-health-analysis, MIT):
//! - Kosinor mit einer 24-h-Komponente, kleinste Quadrate: y = M + A·cos(2π(t − φ)/24). Cornelissen 2014, Theor Biol Med Model 11:16.
//! - IS/IV (Stabilität zwischen Tagen, Zerstückelung innerhalb der Tage) auf Schritten je Stunde. Witting et al. 1990, Biol Psychiatry 27:563.
//! - SRI (Wahrscheinlichkeit, 24 h später im selben Zustand schlafend/wach zu sein), 5-min-Abschnitte, −100…100. Phillips et al. 2017, Sci Rep 7:3216.
//! - Sozialer Jetlag: Schlafmitte freie Tage (Aufwachen Sa/So) minus Werktage. Wittmann et al. 2006, Chronobiol Int 23:497.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::AppHandle;

use crate::daily::{self, day_num, day_str, Row, COLS};
use crate::health;
use crate::stats;

const WEEKS: i64 = 26;
const PROFILE_DAYS: i64 = 28;
const EPOCH: i64 = 5 * 60_000;
const EPOCHS: usize = 288;
const DAY_MS: i64 = 86_400_000;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Cosinor {
    pub mesor: f64,
    pub amp: f64,
    /// Uhrzeit des Höchstwerts in Stunden (0–24)
    pub peak: f64,
    pub r2: f64,
    pub n: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct Week {
    pub start: String,
    pub days: usize,
    pub steps: Option<Cosinor>,
    pub hr: Option<Cosinor>,
    pub is: Option<f64>,
    pub iv: Option<f64>,
    pub sri: Option<f64>,
    pub mid: Option<f64>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Jetlag {
    pub work: f64,
    pub free: f64,
    pub n_work: usize,
    pub n_free: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct RhythmView {
    pub to: String,
    /// mittlere Schritte je Stunde über die letzten 4 Wochen
    pub profile: Vec<Option<f64>>,
    /// alle Kennzahlen über die letzten 4 Wochen
    pub now: Week,
    pub sri_before: Option<f64>,
    pub jetlag: Option<Jetlag>,
    pub weeks: Vec<Week>,
}

/// 3×3-Gleichungssystem (Gauß mit Pivotsuche).
fn solve3(mut a: [[f64; 4]; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let p = (col..3).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))?;
        if a[p][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, p);
        let pivot = a[col];
        for (r, row) in a.iter_mut().enumerate() {
            if r != col {
                let f = row[col] / pivot[col];
                for (x, p) in row.iter_mut().zip(pivot).skip(col) {
                    *x -= f * p;
                }
            }
        }
    }
    Some([a[0][3] / a[0][0], a[1][3] / a[1][1], a[2][3] / a[2][2]])
}

/// Kosinor über Punkte (Uhrzeit in Stunden, Wert). Mindestens 72 Punkte aus 18 verschiedenen Stunden.
pub fn cosinor(pts: &[(f64, f64)]) -> Option<Cosinor> {
    let hours = pts.iter().map(|p| p.0.floor() as i64).collect::<std::collections::BTreeSet<_>>().len();
    if pts.len() < 72 || hours < 18 {
        return None;
    }
    let w = std::f64::consts::TAU / 24.0;
    let mut a = [[0.0; 4]; 3];
    for &(t, y) in pts {
        let x = [1.0, (w * t).cos(), (w * t).sin()];
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += x[i] * x[j];
            }
            a[i][3] += x[i] * y;
        }
    }
    let [m, b, g] = solve3(a)?;
    let mean = pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64;
    let (mut ss_res, mut ss_tot) = (0.0, 0.0);
    for &(t, y) in pts {
        let fit = m + b * (w * t).cos() + g * (w * t).sin();
        ss_res += (y - fit).powi(2);
        ss_tot += (y - mean).powi(2);
    }
    let peak = (g.atan2(b) / w).rem_euclid(24.0);
    Some(Cosinor { mesor: m, amp: (b * b + g * g).sqrt(), peak, r2: if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 0.0 }, n: pts.len() })
}

/// IS und IV aus ganzen Tagen à 24 Stundenwerten. Mindestens 4 Tage.
pub fn is_iv(days: &[[f64; 24]]) -> Option<(f64, f64)> {
    if days.len() < 4 {
        return None;
    }
    let x: Vec<f64> = days.iter().flatten().copied().collect();
    let n = x.len() as f64;
    let mean = x.iter().sum::<f64>() / n;
    let total: f64 = x.iter().map(|v| (v - mean).powi(2)).sum();
    if total == 0.0 {
        return None;
    }
    let hourly: f64 = (0..24).map(|h| (days.iter().map(|d| d[h]).sum::<f64>() / days.len() as f64 - mean).powi(2)).sum();
    let diffs: f64 = x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    Some((n * hourly / (24.0 * total), n * diffs / ((n - 1.0) * total)))
}

/// SRI aus Kalendertagen (je 288 Abschnitte schlafend/wach, `None` = Tag nicht erfasst). Mindestens 5 Tagespaare.
pub fn sri(days: &[Option<Vec<bool>>]) -> Option<f64> {
    let (mut same, mut total) = (0usize, 0usize);
    for w in days.windows(2) {
        if let (Some(a), Some(b)) = (&w[0], &w[1]) {
            same += a.iter().zip(b).filter(|(x, y)| x == y).count();
            total += a.len();
        }
    }
    (total >= 5 * EPOCHS).then(|| -100.0 + 200.0 * same as f64 / total as f64)
}

fn median(v: &mut [f64]) -> Option<f64> {
    (!v.is_empty()).then(|| stats::median(v))
}

/// 0 = Montag … 6 = Sonntag (Tag 0 = 1970-01-01 war ein Donnerstag).
pub fn weekday(dn: i64) -> i64 {
    (dn + 3).rem_euclid(7)
}

pub fn jetlag(mid: &HashMap<i64, f64>, from: i64, to: i64) -> Option<Jetlag> {
    let (mut work, mut free) = (vec![], vec![]);
    for d in from..=to {
        if let Some(&m) = mid.get(&d) {
            if weekday(d) >= 5 { free.push(m) } else { work.push(m) }
        }
    }
    if work.len() < 3 || free.len() < 3 {
        return None;
    }
    let (n_work, n_free) = (work.len(), free.len());
    Some(Jetlag { work: median(&mut work)?, free: median(&mut free)?, n_work, n_free })
}

struct Data {
    steps: HashMap<i64, [f64; 24]>,
    hr: HashMap<i64, Vec<(f64, f64)>>,
    sleep: HashMap<i64, Vec<bool>>,
    mid: HashMap<i64, f64>,
}

fn load(c: &Connection, from: i64, to: i64) -> rusqlite::Result<Data> {
    let (f, t) = (day_str(from), day_str(to));
    let mut steps: HashMap<i64, [f64; 24]> = HashMap::new();
    let mut hr: HashMap<i64, Vec<(f64, f64)>> = HashMap::new();
    let mut st = c.prepare("SELECT day, key, h, value FROM hourly WHERE day >= ?1 AND day <= ?2")?;
    for x in st.query_map(params![f, t], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, f64>(3)?)))? {
        let (d, k, h, v) = x?;
        let Some(dn) = day_num(&d) else { continue };
        match k.as_str() {
            "steps" => steps.entry(dn).or_insert([0.0; 24])[h as usize] = v,
            "hr" => hr.entry(dn).or_default().push((h as f64 + 0.5, v)),
            _ => {}
        }
    }
    // Tage ohne nennenswerte Schritte (Telefon nicht dabei) zählen nicht als „ruhiger Tag“
    steps.retain(|_, d| d.iter().sum::<f64>() >= 500.0);

    let mut mid = HashMap::new();
    let mut nights = std::collections::HashSet::new();
    let mut st = c.prepare("SELECT day, key, value FROM daily WHERE key IN ('mid', 'sleep_min') AND day >= ?1 AND day <= ?2 AND quality = 2")?;
    for x in st.query_map(params![day_str(from - 1), day_str(to + 1)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?)))? {
        let (d, k, v) = x?;
        let Some(dn) = day_num(&d) else { continue };
        if k == "mid" { mid.insert(dn, v); } else { nights.insert(dn); }
    }
    // Schlafzustand je 5 min und Kalendertag; erfasst ist ein Tag, wenn die Nacht davor und die Nacht danach erfasst sind
    let rows = daily::rows(c, &format!("{COLS} WHERE type = 'sleep' AND lstart >= ?1 AND lstart < ?2"), params![from * DAY_MS - DAY_MS, (to + 1) * DAY_MS])?;
    // Nacht n = 18:00 am Vortag bis 18:00 (wie daily.rs); Quelle je Nacht, damit ein Wechsel des Schlaf-Trackers nichts leert
    let mut by_night: HashMap<i64, Vec<&Row>> = HashMap::new();
    for r in rows.iter().filter(|r| matches!(r.v as i64, 1 | 3 | 4 | 5) && r.end > r.start) {
        by_night.entry((r.ls + 6 * 3_600_000).div_euclid(DAY_MS)).or_default().push(r);
    }
    let src: HashMap<i64, &str> = by_night.iter().filter_map(|(n, v)| Some((*n, daily::best_source(v)?))).collect();
    // Abschnitt i zählt, wenn seine Mitte i·5 min + 2,5 min in [a, b) liegt
    let first = |x: i64| ((x - EPOCH / 2 + EPOCH - 1).div_euclid(EPOCH)).clamp(0, EPOCHS as i64) as usize;
    let mut sleep = HashMap::new();
    for d in (from..=to).filter(|d| nights.contains(d) && nights.contains(&(d + 1))) {
        let mut e = vec![false; EPOCHS];
        for n in [d, d + 1] {
            let (Some(rs), Some(s)) = (by_night.get(&n), src.get(&n)) else { continue };
            for r in rs.iter().filter(|r| r.src == *s) {
                let a = r.ls - d * DAY_MS;
                e[first(a)..first(a + r.end - r.start)].fill(true);
            }
        }
        sleep.insert(d, e);
    }
    Ok(Data { steps, hr, sleep, mid })
}

fn range_stats(data: &Data, from: i64, to: i64) -> Week {
    let days: Vec<[f64; 24]> = (from..=to).filter_map(|d| data.steps.get(&d).copied()).collect();
    let steps_pts: Vec<(f64, f64)> = days.iter().flat_map(|d| d.iter().enumerate().map(|(h, v)| (h as f64 + 0.5, *v))).collect();
    let hr_pts: Vec<(f64, f64)> = (from..=to).filter_map(|d| data.hr.get(&d)).flatten().copied().collect();
    // bis einschließlich des Folgetags: auch das Paar über die Wochengrenze zählt
    let sleep: Vec<Option<Vec<bool>>> = (from..=to + 1).map(|d| data.sleep.get(&d).cloned()).collect();
    let isiv = is_iv(&days);
    let mut mids: Vec<f64> = (from..=to).filter_map(|d| data.mid.get(&d).copied()).collect();
    Week {
        start: day_str(from),
        days: days.len(),
        steps: cosinor(&steps_pts),
        hr: cosinor(&hr_pts),
        is: isiv.map(|x| x.0),
        iv: isiv.map(|x| x.1),
        sri: sri(&sleep),
        mid: median(&mut mids),
    }
}

pub fn rhythm_of(c: &Connection, today: &str) -> rusqlite::Result<Option<RhythmView>> {
    let Some(t) = day_num(today) else { return Ok(None) };
    let to = t - 1; // der heutige Tag läuft noch
    let first_week = to - weekday(to) - (WEEKS - 1) * 7;
    let data = load(c, first_week.min(to - 2 * PROFILE_DAYS + 1), to)?;
    let from = to - PROFILE_DAYS + 1;
    let now = range_stats(&data, from, to);
    let sri_before = range_stats(&data, from - PROFILE_DAYS, from - 1).sri;
    let days: Vec<&[f64; 24]> = (from..=to).filter_map(|d| data.steps.get(&d)).collect();
    let profile = (0..24).map(|h| (!days.is_empty()).then(|| days.iter().map(|d| d[h]).sum::<f64>() / days.len() as f64)).collect();
    let weeks = (0..WEEKS)
        .map(|i| {
            let s = first_week + i * 7;
            range_stats(&data, s, (s + 6).min(to))
        })
        .collect();
    Ok(Some(RhythmView {
        to: day_str(to),
        profile,
        now,
        sri_before,
        jetlag: jetlag(&data.mid, from, to),
        weeks,
    }))
}

/// `today`: heutiges Datum in Ortszeit (JJJJ-MM-TT), kommt aus der Oberfläche.
#[tauri::command]
pub async fn rhythm_view(app: AppHandle, today: String) -> Result<Option<RhythmView>, String> {
    rhythm_of(&health::conn(&app)?, &today).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kosinor_findet_hoechstwert() {
        // 10 Tage stündlich: Mittel 70, Ausschlag 12, Höchstwert 15:00
        let pts: Vec<(f64, f64)> = (0..240)
            .map(|i| {
                let t = (i % 24) as f64 + 0.5;
                (t, 70.0 + 12.0 * (std::f64::consts::TAU * (t - 15.0) / 24.0).cos())
            })
            .collect();
        let c = cosinor(&pts).unwrap();
        assert!((c.mesor - 70.0).abs() < 1e-6 && (c.amp - 12.0).abs() < 1e-6 && (c.peak - 15.0).abs() < 1e-6 && c.r2 > 0.999, "{c:?}");
        // Höchstwert kurz vor Mitternacht bleibt im Bereich 0–24
        let late: Vec<(f64, f64)> = pts.iter().map(|&(t, _)| (t, (std::f64::consts::TAU * (t - 23.5) / 24.0).cos())).collect();
        assert!((cosinor(&late).unwrap().peak - 23.5).abs() < 1e-6);
        assert_eq!(cosinor(&pts[..60]), None); // zu wenig Punkte
    }

    #[test]
    fn is_iv_grenzfaelle() {
        // jeden Tag gleich, ein Block 8–20 Uhr: IS = 1, IV = 14 Sprünge · 4 / 167
        let day: [f64; 24] = std::array::from_fn(|h| if (8..20).contains(&h) { 500.0 } else { 0.0 });
        let (is, iv) = is_iv(&[day; 7]).unwrap();
        assert!((is - 1.0).abs() < 1e-9 && (iv - 56.0 / 167.0).abs() < 1e-9, "{is} {iv}");
        // stündlich abwechselnd: IV nahe 2
        let alt: [f64; 24] = std::array::from_fn(|h| if h % 2 == 0 { 500.0 } else { 0.0 });
        let (_, iv) = is_iv(&[alt; 7]).unwrap();
        assert!(iv > 1.9, "{iv}");
        // Tage gegeneinander verschoben: IS sinkt
        let shifted: Vec<[f64; 24]> = (0..7).map(|d| std::array::from_fn(|h| if (h + 24 - d * 3) % 24 < 12 { 500.0 } else { 0.0 })).collect();
        assert!(is_iv(&shifted).unwrap().0 < 0.3);
        assert_eq!(is_iv(&[day; 3]), None);
    }

    #[test]
    fn sri_gleich_und_verschoben() {
        let night = |from: usize, to: usize| Some((0..EPOCHS).map(|i| i < to || i >= from).collect::<Vec<bool>>());
        // jeden Tag 23–7 Uhr: 100
        let same = vec![night(276, 84); 8];
        assert_eq!(sri(&same), Some(100.0));
        // abwechselnd 23–7 und 3–11 Uhr: deutlich niedriger
        let alt: Vec<_> = (0..8).map(|i| if i % 2 == 0 { night(276, 84) } else { Some((0..EPOCHS).map(|e| (36..132).contains(&e)).collect()) }).collect();
        let v = sri(&alt).unwrap();
        assert!(v < 40.0, "{v}");
        // Lücken: Paare mit fehlendem Tag zählen nicht, zu wenige Paare → None
        let gaps = vec![night(276, 84), None, night(276, 84), night(276, 84), None];
        assert_eq!(sri(&gaps), None);
    }

    #[test]
    fn sozialer_jetlag() {
        assert_eq!(weekday(day_num("2026-11-18").unwrap()), 2); // Mittwoch
        let start = day_num("2026-11-02").unwrap(); // Montag
        let mid: HashMap<i64, f64> = (0..14).map(|i| (start + i, if weekday(start + i) >= 5 { 330.0 } else { 240.0 })).collect();
        let j = jetlag(&mid, start, start + 13).unwrap();
        assert_eq!((j.work, j.free, j.n_work, j.n_free), (240.0, 330.0, 10, 4));
        assert_eq!(jetlag(&HashMap::new(), start, start + 13), None);
    }

    #[test]
    fn ansicht_aus_der_datenbank() {
        let c = Connection::open_in_memory().unwrap();
        health::migrate(&c).unwrap();
        let today = day_num("2026-11-18").unwrap();
        // 60 Tage: Schritte 9–18 Uhr, Puls mit Höchstwert um 14:30, jede Nacht 23–7 Uhr (Ortszeit = UTC+1)
        for d in today - 60..today {
            for h in 0..24 {
                let steps = if (9..18).contains(&h) { 800.0 } else { 0.0 };
                c.execute("INSERT INTO hourly VALUES (?1, 'steps', ?2, ?3)", params![day_str(d), h, steps]).unwrap();
                let hr = 65.0 + 10.0 * (std::f64::consts::TAU * (h as f64 + 0.5 - 14.5) / 24.0).cos();
                c.execute("INSERT INTO hourly VALUES (?1, 'hr', ?2, ?3)", params![day_str(d), h, hr]).unwrap();
            }
            let mid = if weekday(d) >= 5 { 240.0 + 60.0 } else { 180.0 };
            for (k, v) in [("sleep_min", 480.0), ("mid", mid)] {
                c.execute("INSERT INTO daily VALUES (?1, ?2, ?3, 'Zepp', 2)", params![day_str(d), k, v]).unwrap();
            }
            let ls = d * DAY_MS - 3_600_000; // 23:00 am Vortag
            c.execute(
                "INSERT INTO sample (id, type, start, end, lstart, day, value, src) VALUES (?1, 'sleep', ?2, ?3, ?4, ?5, 3, 'Zepp')",
                params![format!("s{d}"), ls - 3_600_000, ls - 3_600_000 + 8 * 3_600_000, ls, day_str(d)],
            )
            .unwrap();
        }
        let v = rhythm_of(&c, "2026-11-18").unwrap().unwrap();
        assert_eq!(v.to, "2026-11-17");
        assert_eq!(v.profile[10], Some(800.0));
        assert!((v.now.hr.as_ref().unwrap().peak - 14.5).abs() < 1e-6, "{:?}", v.now.hr);
        // die Nacht zum 18. fehlt, darum hat der 17. keinen Schlafzustand – trotzdem genug Paare
        assert_eq!(v.now.sri, Some(100.0));
        assert_eq!(v.sri_before, Some(100.0));
        let j = v.jetlag.unwrap();
        assert_eq!((j.work, j.free), (180.0, 300.0));
        assert_eq!(v.weeks.len(), WEEKS as usize);
        let last = v.weeks.last().unwrap();
        assert_eq!((last.start.as_str(), last.days), ("2026-11-16", 2));
        let full = &v.weeks[WEEKS as usize - 2];
        assert!((full.is.unwrap() - 1.0).abs() < 1e-9 && full.sri == Some(100.0), "{full:?}");
        assert_eq!(rhythm_of(&c, "kein Datum").unwrap().map(|v| v.to), None);
    }
}


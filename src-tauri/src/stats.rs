//! Kleine, deterministische Statistik ohne Abhängigkeiten: gleicher Seed = gleiches Ergebnis.
//!
//! Herkunft: nach Rust übertragen aus Haze `shared/analysis/stats.mjs` (Haze 1.9, Commit 9733e4e, eigener Code),
//! gleiche Verfahren und Konstanten; Tests aus Haze `tests/analysis.test.mjs` und `tests/longterm.test.mjs` übernommen.
//! Angepasst: Ränge 0-basiert (für Korrelationen gleichwertig), `t975` aus load.rs (Cornish-Fisher wie Haze `tQuantile975`).

/// Perzentil einer sortierten Liste, linear interpoliert (Typ 7, wie Haze `quantileSorted`).
pub fn quantile(v: &[f64], q: f64) -> f64 {
    let i = (v.len() - 1) as f64 * q;
    let (lo, hi) = (i.floor() as usize, i.ceil() as usize);
    v[lo] + (v[hi] - v[lo]) * (i - lo as f64)
}

/// Median; sortiert `v`. Leere Liste ergibt NaN.
pub fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    quantile(v, 0.5)
}

pub fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Ränge mit Mittelrang bei Bindungen.
pub fn ranks(v: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut r = vec![0.0; v.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && v[idx[j + 1]] == v[idx[i]] {
            j += 1;
        }
        for k in i..=j {
            r[idx[k]] = (i + j) as f64 / 2.0;
        }
        i = j + 1;
    }
    r
}

pub fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() < 3 {
        return None;
    }
    let (mx, my) = (mean(x), mean(y));
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        let (dx, dy) = (a - mx, b - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    (sxx > 0.0 && syy > 0.0).then(|| sxy / (sxx * syy).sqrt())
}

pub fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    pearson(&ranks(x), &ranks(y))
}

/// Seedbarer Zufall (mulberry32, wie Haze `rng`).
pub struct Rng(u32);
impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed)
    }
    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        (t ^ (t >> 14)) as f64 / 4_294_967_296.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() * n as f64) as usize
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpearmanTest {
    pub n: usize,
    pub rho: Option<f64>,
    /// Permutations-p, zweiseitig
    pub p: f64,
    /// Bootstrap-95-%-Intervall
    pub ci: Option<(f64, f64)>,
}

/// Spearman mit Permutations-p (zweiseitig) und Bootstrap-95-%-Intervall.
pub fn spearman_test(x: &[f64], y: &[f64], permutations: usize, bootstrap: usize, seed: u32) -> SpearmanTest {
    let n = x.len();
    let (rx, ry) = (ranks(x), ranks(y));
    let Some(rho) = pearson(&rx, &ry).filter(|r| r.is_finite()) else { return SpearmanTest { n, rho: None, p: 1.0, ci: None } };
    let mut r = Rng::new(seed);
    let mut perm = ry.clone();
    let mut extreme = 0;
    for _ in 0..permutations {
        for i in (1..perm.len()).rev() {
            let j = r.below(i + 1);
            perm.swap(i, j);
        }
        if pearson(&rx, &perm).is_some_and(|v| v.abs() >= rho.abs() - 1e-12) {
            extreme += 1;
        }
    }
    let mut boots = Vec::with_capacity(bootstrap);
    let (mut bx, mut by) = (vec![0.0; n], vec![0.0; n]);
    for _ in 0..bootstrap {
        for i in 0..n {
            let j = r.below(n);
            bx[i] = x[j];
            by[i] = y[j];
        }
        if let Some(v) = spearman(&bx, &by).filter(|v| v.is_finite()) {
            boots.push(v);
        }
    }
    boots.sort_by(f64::total_cmp);
    let ci = (boots.len() >= 50).then(|| (quantile(&boots, 0.025), quantile(&boots, 0.975)));
    SpearmanTest { n, rho: Some(rho), p: (extreme + 1) as f64 / (permutations + 1) as f64, ci }
}

/// Benjamini-Hochberg: q-Werte zu p-Werten.
pub fn fdr(ps: &[f64]) -> Vec<f64> {
    let m = ps.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| ps[a].total_cmp(&ps[b]));
    let mut q = vec![0.0; m];
    let mut min: f64 = 1.0;
    for k in (0..m).rev() {
        min = min.min(ps[order[k]] * m as f64 / (k + 1) as f64);
        q[order[k]] = min;
    }
    q
}

/// Standardnormalverteilung (Abramowitz/Stegun 7.1.26, Fehler < 1,5e-7).
pub fn normal_cdf(z: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.327_591_1 * z.abs() / std::f64::consts::SQRT_2);
    let e = 1.0 - t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429)))) * (-z * z / 2.0).exp();
    if z >= 0.0 { (1.0 + e) / 2.0 } else { (1.0 - e) / 2.0 }
}

/// 97,5-%-Quantil der t-Verteilung (Cornish-Fisher-Näherung).
pub fn t975(df: f64) -> f64 {
    let z: f64 = 1.959_964;
    let g1 = (z.powi(3) + z) / 4.0;
    let g2 = (5.0 * z.powi(5) + 16.0 * z.powi(3) + 3.0 * z) / 96.0;
    let g3 = (3.0 * z.powi(7) + 19.0 * z.powi(5) + 17.0 * z.powi(3) - 15.0 * z) / 384.0;
    z + g1 / df + g2 / df.powi(2) + g3 / df.powi(3)
}

/// Theil-Sen: Median aller paarweisen Steigungen, robust gegen Ausreißer. Ergebnis (Steigung, Achsenabschnitt).
pub fn theil_sen(x: &[f64], y: &[f64]) -> Option<(f64, f64)> {
    let mut s = Vec::new();
    for i in 0..x.len() {
        for j in i + 1..x.len() {
            if x[j] != x[i] {
                s.push((y[j] - y[i]) / (x[j] - x[i]));
            }
        }
    }
    if s.is_empty() {
        return None;
    }
    let slope = median(&mut s);
    let mut rest: Vec<f64> = y.iter().zip(x).map(|(v, t)| v - slope * t).collect();
    Some((slope, median(&mut rest)))
}

/// Mann-Kendall-Trendtest (zweiseitig, mit Bindungskorrektur); y in zeitlicher Reihenfolge. Ergebnis p.
pub fn mann_kendall(y: &[f64]) -> f64 {
    let n = y.len();
    if n < 4 {
        return 1.0;
    }
    let mut s = 0.0;
    for i in 0..n {
        for j in i + 1..n {
            s += (y[j] - y[i]).signum() * f64::from(y[j] != y[i]);
        }
    }
    let mut sorted = y.to_vec();
    sorted.sort_by(f64::total_cmp);
    let nf = n as f64;
    let mut v = nf * (nf - 1.0) * (2.0 * nf + 5.0);
    for g in sorted.chunk_by(|a, b| a == b) {
        let t = g.len() as f64;
        v -= t * (t - 1.0) * (2.0 * t + 5.0);
    }
    v /= 18.0;
    let z = if v > 0.0 { (s - s.signum() * f64::from(s != 0.0)) / v.sqrt() } else { 0.0 };
    if z == 0.0 { 1.0 } else { (2.0 * (1.0 - normal_cdf(z.abs()))).min(1.0) }
}

/// Autokorrelation mit Verzögerung 1.
pub fn lag1(a: &[f64]) -> f64 {
    let m = mean(a);
    let (mut num, mut den) = (0.0, 0.0);
    for i in 0..a.len() {
        den += (a[i] - m).powi(2);
        if i > 0 {
            num += (a[i] - m) * (a[i - 1] - m);
        }
    }
    if den > 0.0 { num / den } else { 0.0 }
}

/// Wendepunkte im Mittelwert (binäre Segmentierung). Der Strafterm nutzt die Langzeitvarianz der Reste um die gefundenen
/// Abschnitte (Varianz × (1+ρ)/(1−ρ)), damit zusammenhängende Tage keine Scheinbrüche erzeugen. Iteriert, bis die Schnitte
/// stabil sind. Liefert Startindizes neuer Abschnitte.
pub fn change_points(v: &[f64], min_size: usize, beta: f64, max: usize) -> Vec<usize> {
    let n = v.len();
    if n < 2 * min_size {
        return vec![];
    }
    let mut pre = vec![0.0];
    for x in v {
        pre.push(pre.last().unwrap() + x);
    }
    let avg = |i: usize, j: usize| (pre[j] - pre[i]) / (j - i) as f64;
    let best = |i: usize, j: usize| {
        let mut top: Option<(usize, f64)> = None;
        for k in i + min_size..=j.saturating_sub(min_size) {
            let g = ((k - i) * (j - k)) as f64 / (j - i) as f64 * (avg(i, k) - avg(k, j)).powi(2);
            if top.is_none_or(|t| g > t.1) {
                top = Some((k, g));
            }
        }
        top
    };
    let segment = |penalty: f64| {
        let mut cuts: Vec<usize> = vec![];
        while cuts.len() < max {
            let bounds: Vec<usize> = std::iter::once(0).chain(cuts.iter().copied()).chain([n]).collect();
            let mut pick: Option<(usize, f64)> = None;
            for w in bounds.windows(2) {
                if let Some(b) = best(w[0], w[1]) {
                    if pick.is_none_or(|p| b.1 > p.1) {
                        pick = Some(b);
                    }
                }
            }
            match pick {
                Some((k, g)) if g > penalty => {
                    cuts.push(k);
                    cuts.sort_unstable();
                }
                _ => break,
            }
        }
        cuts
    };
    let lrv = |cuts: &[usize]| {
        let bounds: Vec<usize> = std::iter::once(0).chain(cuts.iter().copied()).chain([n]).collect();
        let mut res = Vec::with_capacity(n);
        for w in bounds.windows(2) {
            let m = avg(w[0], w[1]);
            res.extend(v[w[0]..w[1]].iter().map(|x| x - m));
        }
        let rho = lag1(&res).clamp(0.0, 0.9);
        let var = res.iter().map(|x| x * x).sum::<f64>() / (res.len() as f64 - bounds.len() as f64 + 1.0).max(1.0);
        var * (1.0 + rho) / (1.0 - rho)
    };
    // Start optimistisch (Streuung aus Tagesdifferenzen), dann mit der Langzeitvarianz der Reste nachschärfen.
    let mut d: Vec<f64> = v.windows(2).map(|w| w[1] - w[0]).collect();
    let md = median(&mut d.clone());
    let mut dev: Vec<f64> = d.iter_mut().map(|x| (*x - md).abs()).collect();
    let sigma = 1.4826 * median(&mut dev) / std::f64::consts::SQRT_2;
    if sigma == 0.0 {
        return vec![];
    }
    let ln = (n as f64).ln();
    let mut cuts = segment(beta * sigma * sigma * ln);
    for _ in 0..4 {
        let next = segment(beta * lrv(&cuts) * ln);
        if next == cuts {
            break;
        }
        cuts = next;
    }
    cuts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// wie Haze `noise`: Summe von 6 Gleichverteilten, zentriert
    fn noise(n: usize, seed: u32, s: f64) -> Vec<f64> {
        let mut r = Rng::new(seed);
        (0..n).map(|_| ((0..6).map(|_| r.next()).sum::<f64>() - 3.0) * 1.41 * s).collect()
    }

    #[test]
    fn grundlagen_wie_haze() {
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.5);
        assert_eq!(ranks(&[10.0, 20.0, 20.0, 30.0]), vec![0.0, 1.5, 1.5, 3.0]);
        assert_eq!(spearman(&[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 4.0, 8.0, 16.0, 32.0]), Some(1.0));
        assert_eq!(spearman(&[1.0, 2.0, 3.0, 4.0, 5.0], &[5.0, 4.0, 3.0, 2.0, 1.0]), Some(-1.0));
        let p = [0.01, 0.04, 0.03, 0.5];
        let q = fdr(&p);
        assert!(q.iter().zip(p).all(|(q, p)| *q >= p - 1e-12) && q[0] <= q[1]);
        let x: Vec<f64> = (0..30).map(f64::from).collect();
        let y: Vec<f64> = x.iter().map(|v| v * 2.0 + v % 3.0).collect();
        let (a, b) = (spearman_test(&x, &y, 999, 999, 7), spearman_test(&x, &y, 999, 999, 7));
        assert_eq!(a, b);
        assert!(a.p < 0.01 && a.ci.unwrap().0 > 0.8, "{a:?}");
        assert!((normal_cdf(1.96) - 0.975).abs() < 1e-4 && (normal_cdf(0.0) - 0.5).abs() < 1e-9);
        assert!((t975(10.0) - 2.228).abs() < 0.01);
    }

    #[test]
    fn gleiche_zahlen_wie_haze() {
        // Bezugswerte aus Haze stats.mjs (node), gleiche Eingaben und Seeds
        let mut r = Rng::new(1);
        let v: Vec<f64> = (0..3).map(|_| (r.next() * 1e9).round()).collect();
        assert_eq!(v, vec![627073941.0, 2735721.0, 527447040.0]);
        let x: Vec<f64> = (0..30).map(f64::from).collect();
        let y: Vec<f64> = x.iter().map(|v| v * 2.0 + v % 3.0).collect();
        let t = spearman_test(&x, &y, 999, 999, 7);
        let (lo, hi) = t.ci.unwrap();
        assert!((t.rho.unwrap() - 0.998998386037683).abs() < 1e-12 && t.p == 0.001, "{t:?}");
        assert!((lo - 0.9950690581658026).abs() < 1e-12 && (hi - 0.9996649841260412).abs() < 1e-12, "{t:?}");
        assert!((mann_kendall(&noise(40, 2, 5.0)) - 0.18798397147671575).abs() < 1e-12);
        let n = noise(20, 4, 5.0);
        let (s, i) = theil_sen(&(0..20).map(f64::from).collect::<Vec<_>>(), &n).unwrap();
        assert!((s - 0.16940280471841107).abs() < 1e-12 && (i - 0.90701959597427).abs() < 1e-12);
    }

    #[test]
    fn trend() {
        let x: Vec<f64> = (0..10).map(f64::from).collect();
        let mut y: Vec<f64> = x.iter().map(|v| 2.0 * v + 1.0).collect();
        y[4] = 99.0; // ein Ausreißer ändert die Steigung nicht
        assert_eq!(theil_sen(&x, &y).unwrap().0, 2.0);
        assert!(mann_kendall(&x.iter().map(|v| v + (v % 2.0) * 0.5).collect::<Vec<_>>()) < 0.01);
        assert_eq!(mann_kendall(&[5.0; 5]), 1.0);
    }

    #[test]
    fn wendepunkte() {
        let shifted: Vec<f64> = noise(120, 3, 5.0).iter().enumerate().map(|(i, v)| v + if i >= 70 { -15.0 } else { 0.0 }).collect();
        let c = change_points(&shifted, 14, 4.0, 3);
        assert!(c.len() == 1 && c[0].abs_diff(70) <= 3, "{c:?}");
        for seed in [1, 2, 4, 5, 6] {
            assert_eq!(change_points(&noise(120, seed, 5.0), 14, 4.0, 3), Vec::<usize>::new(), "Scheinbruch bei Seed {seed}");
        }
        // zusammenhängende Tage (AR(1)) erzeugen ohne Bruch keine Scheinbrüche
        let mut r = noise(150, 9, 5.0);
        for i in 1..r.len() {
            r[i] += 0.7 * r[i - 1];
        }
        assert_eq!(change_points(&r, 14, 4.0, 3), Vec::<usize>::new());
    }
}

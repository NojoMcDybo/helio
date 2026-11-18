import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { METRIC, clock, dur, fmt } from "./metrics";
import { gapPath, localDay, short, useWidth } from "./chart";
import { ForecastCard } from "./Forecast";

// Seite „Langzeit“: Trends über 52 Wochen, Wendepunkte, Monatsmittel über alle Jahre, feste Fragen zu Zusammenhängen (longterm.rs).
// Diagramme einfarbig in Tinte wie Belastung und Rhythmus; Aussagen wie in Haze Langzeit/Hinweise.
type Trend = { n: number; per_month: number; change: number; p: number; q: number; status: string; slope: number; intercept: number };
type Change = { day: string; before: number; after: number };
type Metric = { key: string; now: number | null; weeks: (number | null)[]; months: [string, number, number][]; trend: Trend | null; changes: Change[] };
type Link = {
  x: string; y: string; lag: number; n: number; rho: number | null; ci: [number, number] | null; p: number | null; q: number | null; zero: boolean;
  low_max: number | null; high_min: number | null; low_median: number | null; high_median: number | null; status: string; confirmed: boolean | null;
};
type LongtermView = { to: string; first: string; week_starts: string[]; metrics: Metric[]; links: Link[] };

const show = (key: string, v: number) => METRIC.get(key)?.show(v) ?? fmt(v, 1);
/// Unterschied in der Einheit der Kennzahl (Uhrzeiten und Dauern in Minuten)
const delta = (key: string, v: number) => {
  const kind = METRIC.get(key)?.kind;
  const s = v > 0 ? "+" : v < 0 ? "−" : "±";
  return kind === "clock" || kind === "dur" ? `${s}${dur(Math.abs(v))}` : `${s}${show(key, Math.abs(v)).replace(/^-/, "")}`;
};
const MONTHS = ["Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"];
const monthLabel = (m: string) => `${MONTHS[+m.slice(5, 7) - 1]} ${m.slice(0, 4)}`;

// Fragen: x am Tag, y am Tag danach (lag 0: Nachtwerte gehören zum Aufwachtag) bzw. in der Nacht danach (lag 1)
const X: Record<string, { title: string; none?: string; show: (v: number) => string }> = {
  caffeine_late: { title: "Koffein nach 14 Uhr", none: "ohne Koffein nach 14 Uhr", show: (v) => `${fmt(v)} mg` },
  alcohol: { title: "Alkohol", none: "ohne Alkohol", show: (v) => `${fmt(v, 1)} Getränke` },
  last_meal: { title: "Späte letzte Mahlzeit", show: clock },
  trimp: { title: "Trainingslast", show: (v) => fmt(v) },
  sleep_min: { title: "Schlafdauer", show: dur },
  onset_dev: { title: "Unregelmäßige Einschlafzeit", show: (v) => `${dur(v)} Abweichung` },
};
const when = (l: Link) => (l.lag === 1 ? "in der Nacht danach" : "am Tag danach");

function Spark({ m }: { m: Metric }) {
  const vals = m.weeks.filter((v): v is number => v != null);
  if (vals.length < 2) return null;
  const W = 140, H = 34, n = m.weeks.length, t = m.trend;
  const line = t ? [0, n - 1].map((i) => t.intercept + t.slope * i) : [];
  const lo = Math.min(...vals, ...line), hi = Math.max(...vals, ...line), pad = (hi - lo) * 0.12 || 1;
  const x = (i: number) => (i / (n - 1)) * W, y = (v: number) => H - ((v - lo + pad) / (hi - lo + 2 * pad)) * H;
  return (
    <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="trend-spark" aria-hidden="true">
      <path d={gapPath(m.weeks, x, y)} className="spark-line" />
      {t && t.status !== "kein klarer Trend" && <path d={`M0 ${y(line[0]).toFixed(1)}L${W} ${y(line[1]).toFixed(1)}`} className="spark-trend" />}
    </svg>
  );
}

function Tile({ m, active, onPick }: { m: Metric; active: boolean; onPick: () => void }) {
  const label = METRIC.get(m.key)?.label ?? m.key, t = m.trend, c = m.changes[m.changes.length - 1];
  const text = !t ? "zu wenige Wochen für einen Trend"
    : t.status === "kein klarer Trend" ? "kein klarer Trend"
    : `${t.status}: ${delta(m.key, t.change)} in ${t.n} Wochen`;
  return (
    <button type="button" className={"trend-tile long-tile" + (t && t.status !== "kein klarer Trend" ? " off" : "") + (active ? " picked" : "")}
      aria-pressed={active} onClick={onPick} data-g={METRIC.get(m.key)?.g}>
      <span className="n-eyebrow">{label}</span>
      <b>{m.now != null ? show(m.key, m.now) : "–"}</b>
      <Spark m={m} />
      <small>{text}{c ? ` · Wendepunkt ${short(c.day)}: ${show(m.key, c.before)} → ${show(m.key, c.after)}` : ""}</small>
    </button>
  );
}

function MonthChart({ m }: { m: Metric }) {
  const [ref, w] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const list = m.months, n = list.length;
  const H = 160, L = 46, R = 6, T = 8, B = 20;
  const vals = list.map((x) => x[1]), lo = Math.min(...vals), hi = Math.max(...vals), pad = (hi - lo) * 0.1 || 1;
  const x = (i: number) => L + ((i + 0.5) / n) * (w - L - R), y = (v: number) => T + ((hi + pad - v) / (hi - lo + 2 * pad)) * (H - T - B);
  // Lücken (Monate ohne genug Tage) unterbrechen die Linie
  const idx = (s: string) => +s.slice(0, 4) * 12 + +s.slice(5, 7);
  const series = list.map((v, i) => (i && idx(v[0]) - idx(list[i - 1][0]) > 1 ? null : v[1]));
  const path = list.reduce((s, v, i) => s + (i && series[i] != null ? "L" : "M") + `${x(i).toFixed(1)} ${y(v[1]).toFixed(1)}`, "");
  const years = list.map((v, i) => [v[0], i] as const).filter(([k], i) => i === 0 || k.endsWith("-01"));
  const pick = (e: React.PointerEvent) => {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    setHover(Math.max(0, Math.min(n - 1, Math.round(((e.clientX - r.left - L) / (w - L - R)) * n - 0.5))));
  };
  const h = hover != null ? list[hover] : null;
  const label = METRIC.get(m.key)?.label ?? m.key;
  return (
    <div ref={ref} className="chart" data-g={METRIC.get(m.key)?.g}>
      <svg width={w} height={H} role="img" aria-label={`${label}, Monatsmittel über ${n} Monate`} onPointerMove={pick} onPointerLeave={() => setHover(null)}>
        {[hi, (hi + lo) / 2, lo].map((v) => (
          <g key={v}>
            <line x1={L} x2={w - R} y1={y(v)} y2={y(v)} className="grid" />
            <text x={L - 6} y={y(v) + 3} className="axis" textAnchor="end">{show(m.key, v)}</text>
          </g>
        ))}
        {years.map(([k, i]) => <text key={k} x={x(i)} y={H - 6} className="axis" textAnchor={i ? "middle" : "start"}>{k.slice(0, 4)}</text>)}
        <path d={path} className="line-fit" />
        {list.map((v, i) => ((i && series[i] != null) || series[i + 1] != null) || <circle key={v[0]} cx={x(i)} cy={y(v[1])} r={3} className="chart-dot" />)}
        {h && hover != null && <line x1={x(hover)} x2={x(hover)} y1={T} y2={H - B} className="crosshair" />}
      </svg>
      {h && hover != null && (
        <div className="chart-tip" style={{ left: Math.min(w - 150, Math.max(0, x(hover) + 10)) }}>
          <b>{monthLabel(h[0])}</b>
          <span>{show(m.key, h[1])} im Mittel</span>
          <span className="muted">{h[2]} Tage mit Wert</span>
        </div>
      )}
    </div>
  );
}

function LinkRow({ l }: { l: Link }) {
  const X0 = X[l.x], y = METRIC.get(l.y)?.label ?? l.y, yv = (v: number | null) => (v == null ? "–" : show(l.y, v));
  const title = `${X0?.title ?? l.x} → ${y} ${when(l)}`;
  if (l.status === "zu wenige Daten") return <tr><td>{title}</td><td className="verdict">zu wenige Tage (n = {l.n})</td></tr>;
  const groups = l.zero
    ? `${X0?.none ?? "ohne"}: ${yv(l.low_median)}, mit (ab ${X0?.show(l.high_min ?? 0)}): ${yv(l.high_median)}`
    : `unteres Drittel (bis ${X0?.show(l.low_max ?? 0)}): ${yv(l.low_median)}, oberes (ab ${X0?.show(l.high_min ?? 0)}): ${yv(l.high_median)}`;
  const found = l.status === "Zusammenhang";
  return (
    <tr className={found ? "found" : ""}>
      <td>
        {title}
        <small>Median {y} {groups} · ρ = {fmt(l.rho ?? 0, 2)}, n = {l.n} Tage</small>
      </td>
      <td className="verdict">
        {found ? (Math.abs(l.rho ?? 0) >= 0.5 ? "deutlicher Zusammenhang" : "mäßiger Zusammenhang") : "kein deutlicher Zusammenhang"}
        {found && <small>{l.confirmed == null ? "zu wenige neue Tage zum Bestätigen" : l.confirmed ? "auch in den neuesten Tagen" : "in den neuesten Tagen nicht bestätigt"}</small>}
      </td>
    </tr>
  );
}

export function Longterm() {
  const [view, setView] = useState<LongtermView | null>(null);
  const [error, setError] = useState("");
  const [pick, setPick] = useState("steps");
  const load = () => invoke<LongtermView | null>("longterm_view", { today: localDay() }).then((v) => { setView(v); setError(""); }).catch((e) => setError(String(e)));
  useEffect(() => {
    void load();
    const un = listen<unknown>("health", (e) => { if (e.payload) void load(); });
    const t = setInterval(() => void load(), 30 * 60_000);
    return () => { void un.then((f) => f()); clearInterval(t); };
  }, []);

  const shown = view?.metrics.filter((m) => m.now != null || m.weeks.some((w) => w != null)) ?? [];
  if (!view || !shown.length) {
    return (
      <section className="days" aria-label="Langzeit">
        <div className="n-eyebrow">Langzeit</div>
        <p className="fine">{error || (view ? "Noch zu wenig Werte. Sobald Apple-Health-Werte über mehrere Wochen da sind, erscheinen hier Trends." : "…")}</p>
      </section>
    );
  }
  const sel = view.metrics.find((m) => m.key === pick && m.months.length > 1) ?? shown.find((m) => m.months.length > 1);
  const moving = shown.filter((m) => m.trend && m.trend.status !== "kein klarer Trend");
  const found = view.links.filter((l) => l.status === "Zusammenhang");

  return (
    <section className="days" aria-label="Langzeit">
      <p className="today-sum" role="status">
        {moving.length
          ? <>Klarer Trend im letzten Jahr: {moving.map((m, i) => <span key={m.key}>{i ? ", " : ""}<b>{METRIC.get(m.key)?.label}</b> {m.trend!.status}</span>)}.</>
          : "Im letzten Jahr kein klarer Trend."}
      </p>
      <article className="day-group n-card">
        <div className="n-eyebrow">Trends · 52 Wochen bis {short(view.to)}</div>
        <small className="fine">Groß: Mittel der letzten 4 Wochen. Linie: Wochenmittel, gestrichelt der Trend, wo er klar ist. Antippen zeigt die Monate.</small>
        <div className="trend-tiles">{shown.map((m) => <Tile key={m.key} m={m} active={m.key === sel?.key} onPick={() => setPick(m.key)} />)}</div>
      </article>

      {sel && (
        <article className="day-group n-card">
          <div className="n-eyebrow">{METRIC.get(sel.key)?.label} · Monate seit {monthLabel(sel.months[0][0])}</div>
          <MonthChart m={sel} />
        </article>
      )}

      <article className="day-group n-card">
        <div className="n-eyebrow">Zusammenhänge · feste Fragen</div>
        <small className="fine">
          {found.length ? `${found.length} von ${view.links.length} Fragen zeigen einen Zusammenhang.` : `Keine der ${view.links.length} Fragen zeigt in deinen Daten einen deutlichen Zusammenhang.`}
        </small>
        <table className="links">
          <tbody>{view.links.map((l) => <LinkRow key={`${l.x}${l.y}${l.lag}`} l={l} />)}</tbody>
        </table>
      </article>
      <ForecastCard />
      {error && <p className="fine warn" role="alert">{error}</p>}
      <details className="more n-card">
        <summary>Wie wird gerechnet?</summary>
        <p className="fine">
          Trend: Steigung über die Wochenmittel der letzten 52 Wochen (Theil-Sen, robust gegen Ausreißer), geprüft mit Mann-Kendall und über alle
          Kennzahlen gemeinsam korrigiert (FDR). „Klar“ heißt: statistisch deutlich und spürbar groß (z. B. 20 min Schlaf, 2 bpm Ruhepuls, 1.000
          Schritte). Wendepunkt: ab diesem Tag lag der Wert im letzten Jahr dauerhaft anders (Median davor → danach). Zusammenhänge: feste Fragen,
          Spearman-Korrelation mit Permutationstest und Bootstrap-Intervall, FDR über alle Fragen; „Zusammenhang“ ab |ρ| ≥ 0,3. Bestätigt heißt: im
          neuesten Drittel der Tage für sich wieder zu sehen. Fehlen Alkohol oder Koffein an einem Tag mit erfasstem Essen, zählt das als keiner.
          Zusammen auftreten heißt nicht verursachen: Beides kann eine gemeinsame Ursache haben. Das beschreibt deine Werte und ist kein medizinischer Befund.
        </p>
      </details>
    </section>
  );
}

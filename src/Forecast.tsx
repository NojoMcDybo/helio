import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { METRIC, fmt } from "./metrics";
import { gapPath, localDay, short, useWidth } from "./chart";

// Prognose-Experiment (forecast.rs): standardmäßig aus, nur hier, nie in der Notch. Darstellung wie Haze `src/Forecast.tsx`:
// blasse gestrichelte Linie mit Band hinter „heute“, dauerhaft beschriftet „Experiment · nicht belastbar“.
type Point = { day: string; value: number; lo: number; hi: number };
type Score = { key: string; label: string; rmse: (number | null)[]; score: number | null };
type Forecast = {
  key: string; origins: number; enough: boolean; best: string; gain: number; models: Score[];
  from: string; last: string | null; series: (number | null)[]; points: Point[];
};

const STORE = "helio-forecast";
const stored = () => { try { return localStorage.getItem(STORE) === "an"; } catch { return false; } };
const show = (key: string, v: number) => METRIC.get(key)?.show(v) ?? fmt(v, 1);
const dayIdx = (from: string, day: string) => Math.round((Date.parse(`${day}T12:00:00`) - Date.parse(`${from}T12:00:00`)) / 864e5);

function Chart({ f }: { f: Forecast }) {
  const [ref, w] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const n = f.series.length + 7, H = 140, L = 46, R = 6, T = 14, B = 20;
  const vals = [...f.series.filter((v): v is number => v != null), ...f.points.flatMap((p) => [p.lo, p.hi])];
  const lo = Math.min(...vals), hi = Math.max(...vals), pad = (hi - lo) * 0.1 || 1;
  const x = (i: number) => L + ((i + 0.5) / n) * (w - L - R), y = (v: number) => T + ((hi + pad - v) / (hi - lo + 2 * pad)) * (H - T - B);
  const lastI = f.last ? dayIdx(f.from, f.last) : f.series.length - 1;
  const pts = f.points.map((p) => ({ ...p, i: dayIdx(f.from, p.day) }));
  // Prognose setzt am letzten Wert an
  const start = f.series[lastI];
  const line = (start != null ? `M${x(lastI)} ${y(start)}` : "") + pts.map((p, k) => `${start != null || k ? "L" : "M"}${x(p.i)} ${y(p.value)}`).join("");
  const band = pts.length ? `M${pts.map((p) => `${x(p.i)} ${y(p.hi)}`).join("L")}L${[...pts].reverse().map((p) => `${x(p.i)} ${y(p.lo)}`).join("L")}Z` : "";
  const pick = (e: React.PointerEvent) => {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    setHover(Math.max(0, Math.min(n - 1, Math.round(((e.clientX - r.left - L) / (w - L - R)) * n - 0.5))));
  };
  const hp = hover != null ? pts.find((p) => p.i === hover) : null, hv = hover != null ? f.series[hover] : null;
  const dayOf = (i: number) => { const d = new Date(`${f.from}T12:00:00`); d.setDate(d.getDate() + i); return localDay(d); };
  const label = METRIC.get(f.key)?.label ?? f.key;
  return (
    <div ref={ref} className="chart" data-g="heart">
      <svg width={w} height={H} role="img" aria-label={`${label}, letzte 4 Wochen und Prognose für 7 Tage, Experiment, nicht belastbar`}
        onPointerMove={pick} onPointerLeave={() => setHover(null)}>
        {[hi, (hi + lo) / 2, lo].map((v) => (
          <g key={v}>
            <line x1={L} x2={w - R} y1={y(v)} y2={y(v)} className="grid" />
            <text x={L - 6} y={y(v) + 3} className="axis" textAnchor="end">{show(f.key, v)}</text>
          </g>
        ))}
        <text x={L} y={H - 6} className="axis">{short(f.from)}</text>
        <text x={x(lastI)} y={H - 6} className="axis" textAnchor="middle">{f.last ? short(f.last) : ""}</text>
        <text x={w - R} y={10} className="axis" textAnchor="end">Experiment · nicht belastbar</text>
        <line x1={x(lastI)} x2={x(lastI)} y1={T} y2={H - B} className="crosshair" />
        {band && <path d={band} className="usual-band" />}
        <path d={gapPath(f.series, x, y)} className="line-fit" />
        {pts.length > 0 && <path d={line} className="line-fat" />}
        {hover != null && (hp || hv != null) && <line x1={x(hover)} x2={x(hover)} y1={T} y2={H - B} className="crosshair" />}
      </svg>
      {hover != null && (hp || hv != null) && (
        <div className="chart-tip" style={{ left: Math.min(w - 150, Math.max(0, x(hover) + 10)) }}>
          <b>{short(dayOf(hover))}</b>
          {hp ? <>
            <span>Prognose {show(f.key, hp.value)}</span>
            <span className="muted">8 von 10 Fehlern: {show(f.key, hp.lo)}–{show(f.key, hp.hi)}</span>
          </> : <span>{show(f.key, hv!)}</span>}
        </div>
      )}
    </div>
  );
}

function Block({ f }: { f: Forecast }) {
  const best = f.models.find((m) => m.key === f.best)!, naive = f.models.find((m) => m.key === "naive")!;
  const label = METRIC.get(f.key)?.label ?? f.key;
  const text = !f.enough
    ? `Noch zu wenig Werte für eine ehrliche Prüfung (${f.origins} Tage mit Wert in den letzten 90, nötig 30).`
    : f.best === "naive"
      ? `Kein Modell lag in der Prüfung deutlich (5 %) näher als „wie zuletzt“ (typischerweise ±${show(f.key, naive.score!)} daneben). Gezeigt wird darum der letzte Wert.`
      : `„${best.label}“ lag in der Prüfung an ${f.origins} Tagen typischerweise ±${show(f.key, best.score!)} daneben, ${fmt(f.gain * 100)} % weniger als „wie zuletzt“ (±${show(f.key, naive.score!)}).`;
  return (
    <div className="forecast">
      <b>{label}</b>
      {f.series.some((v) => v != null) && <Chart f={f} />}
      <small className="fine">
        {text}
        {f.enough && !f.points.length ? " Der letzte Wert ist älter als 2 Tage, darum gibt es gerade keine Prognose." : ""}
      </small>
    </div>
  );
}

export function ForecastCard() {
  const [on, setOn] = useState(stored);
  const [list, setList] = useState<Forecast[] | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!on) return;
    const load = () => invoke<Forecast[]>("forecast_view", { today: localDay() }).then((v) => { setList(v); setError(""); }).catch((e) => setError(String(e)));
    void load();
    const un = listen<unknown>("health", (e) => { if (e.payload) void load(); });
    const t = setInterval(() => void load(), 30 * 60_000);
    return () => { void un.then((u) => u()); clearInterval(t); };
  }, [on]);
  const toggle = (v: boolean) => {
    try { localStorage.setItem(STORE, v ? "an" : "aus"); } catch { /* bleibt bis zum Neustart */ }
    setOn(v);
    if (!v) setList(null);
  };

  return (
    <article className="day-group n-card lg">
      <div className="n-eyebrow">Prognose-Experiment · nicht belastbar</div>
      {!on ? (
        <>
          <small className="fine">
            Ruhepuls und HRV der nächsten 7 Tage aus einfachen Modellen, rückwärts an deinen letzten 90 Tagen geprüft. Die Werte schwanken von
            Tag zu Tag stark, die Prognose ist unsicher und keine Grundlage für Entscheidungen.
          </small>
          <button className="n-btn forecast-toggle" onClick={() => toggle(true)}>Einschalten, nicht belastbar</button>
        </>
      ) : (
        <>
          {list?.map((f) => <Block key={f.key} f={f} />)}
          {list && !list.length && <small className="fine">Noch keine Werte für Ruhepuls oder HRV.</small>}
          {!list && !error && <small className="fine">…</small>}
          {error && <p className="fine warn" role="alert">{error}</p>}
          <small className="fine forecast-note">
            Linie: deine Werte. Gestrichelt ab dem letzten Wert: Prognose, das helle Band umfasst 8 von 10 Fehlern der Prüfung. Geprüft wird an
            jedem Tag der letzten 90 Tage, was die Modelle für die 7 Tage danach gesagt hätten; das gelernte Modell kennt diese Tage nicht. Es
            weiß nichts über Training, Krankheit oder Schlaf der kommenden Tage.
          </small>
          <button className="n-btn forecast-toggle" onClick={() => toggle(false)}>Ausschalten</button>
        </>
      )}
    </article>
  );
}

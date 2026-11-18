import { DivBar } from "./viz";
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { fmt } from "./metrics";
import { localDay, median, short, ticks, useWidth } from "./chart";

// Seite „Belastung“: Trainingslast, Fitness/Ermüdung (Banister), Pulserholung, Ruhepuls/HRV nach harten Tagen (load.rs).
// Diagramme einfarbig in Tinte (Farbe nur für Abweichungen, Haze UI-Leitlinien); Kennung über Linienart und Legende.
type LoadDay = { day: string; trimp: number; fitness: number; fatigue: number };
type Recovery = { day: string; end: number; minutes: number; hr_end: number; drop60: number; drop120: number | null; src: string };
type After = { key: string; lag: number; n: number; mean: number; ci: number; other_n: number; other_mean: number };
type LoadView = { days: LoadDay[]; recoveries: Recovery[]; after: After[]; hard: number | null };

const dayMs = (day: string) => new Date(`${day}T12:00:00`).getTime();
const signed = (v: number, d = 0) => `${v > 0 ? "+" : v < 0 ? "−" : "±"}${fmt(Math.abs(v), d)}`;

function LoadChart({ days }: { days: LoadDay[] }) {
  const [ref, w] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const H = 170, L = 34, R = 6, T = 8, B = 20, n = days.length;
  const max = Math.max(1, ...days.map((d) => Math.max(d.trimp, d.fitness, d.fatigue)));
  const ys = ticks(max), top = ys[ys.length - 1] < max ? max : ys[ys.length - 1];
  const x = (i: number) => L + ((i + 0.5) / n) * (w - L - R), y = (v: number) => T + (1 - v / top) * (H - T - B);
  const bw = Math.min(12, Math.max(1, ((w - L - R) / n) * 0.6));
  const line = (k: "fitness" | "fatigue") => days.map((d, i) => `${i ? "L" : "M"}${x(i).toFixed(1)} ${y(d[k]).toFixed(1)}`).join("");
  const months = days.map((d, i) => [d, i] as const).filter(([d]) => d.day.endsWith("-01"));
  const pick = (e: React.PointerEvent) => {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    setHover(Math.max(0, Math.min(n - 1, Math.round(((e.clientX - r.left - L) / (w - L - R)) * n - 0.5))));
  };
  const h = hover != null ? days[hover] : null;
  return (
    <div ref={ref} className="chart" data-g="move" data-g2="warm">
      <svg width={w} height={H} role="img" aria-label={`Trainingslast der letzten ${n} Tage mit Fitness und Ermüdung`}
        onPointerMove={pick} onPointerLeave={() => setHover(null)}>
        {ys.map((v) => (
          <g key={v}>
            <line x1={L} x2={w - R} y1={y(v)} y2={y(v)} className="grid" />
            <text x={L - 6} y={y(v) + 3} className="axis" textAnchor="end">{fmt(v)}</text>
          </g>
        ))}
        {months.map(([d, i]) => <text key={d.day} x={x(i)} y={H - 6} className="axis" textAnchor="middle">{short(d.day)}</text>)}
        {days.map((d, i) => d.trimp > 0 && (
          <rect key={d.day} x={x(i) - bw / 2} y={y(d.trimp)} width={bw} height={y(0) - y(d.trimp)} rx={Math.min(2, bw / 2)} className="load-bar" />
        ))}
        <path d={line("fitness")} className="line-fit" />
        <path d={line("fatigue")} className="line-fat" />
        {h && hover != null && <line x1={x(hover)} x2={x(hover)} y1={T} y2={H - B} className="crosshair" />}
      </svg>
      {h && hover != null && (
        <div className="chart-tip" style={{ left: Math.min(w - 150, Math.max(0, x(hover) + 10)) }}>
          <b>{new Date(dayMs(h.day)).toLocaleDateString("de-DE", { weekday: "short", day: "2-digit", month: "2-digit" })}</b>
          <span><i className="chart-key bar-key" />{fmt(h.trimp)} Trainingslast</span>
          <span><i className="chart-key fit-key" />{fmt(h.fitness)} Fitness</span>
          <span><i className="chart-key fat-key" />{fmt(h.fatigue)} Ermüdung</span>
        </div>
      )}
      <div className="chart-legend" aria-hidden="true">
        <span><i className="chart-key bar-key" />Trainingslast je Tag</span>
        <span><i className="chart-key fit-key" />Fitness (42 Tage)</span>
        <span><i className="chart-key fat-key" />Ermüdung (7 Tage)</span>
      </div>
    </div>
  );
}

function RecoveryChart({ list }: { list: Recovery[] }) {
  const [ref, w] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const H = 156, L = 34, R = 6, T = 8, B = 26;
  const t0 = Math.min(...list.map((r) => r.end)), t1 = Math.max(...list.map((r) => r.end));
  const max = Math.max(10, ...list.map((r) => r.drop60)), min = Math.min(0, ...list.map((r) => r.drop60));
  const ys = ticks(max).filter((v) => v >= min);
  const top = Math.max(max, ys[ys.length - 1]);
  const x = (t: number) => L + ((t - t0) / Math.max(1, t1 - t0)) * (w - L - R), y = (v: number) => T + ((top - v) / (top - min || 1)) * (H - T - B);
  // Median je Woche (Montag), als Linie
  const weeks = new Map<number, number[]>();
  for (const r of list) {
    const d = new Date(r.end); d.setHours(0, 0, 0, 0); d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
    weeks.set(d.getTime(), [...(weeks.get(d.getTime()) ?? []), r.drop60]);
  }
  const wk = [...weeks].sort((a, b) => a[0] - b[0]).map(([t, v]) => [t + 3.5 * 86_400_000, median(v)] as const);
  const path = wk.map(([t, v], i) => `${i ? "L" : "M"}${x(Math.min(t1, Math.max(t0, t))).toFixed(1)} ${y(v).toFixed(1)}`).join("");
  const pick = (e: React.PointerEvent) => {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect(), px = e.clientX - r.left, py = e.clientY - r.top;
    let best = -1, d = 24 ** 2;
    list.forEach((p, i) => { const dd = (x(p.end) - px) ** 2 + (y(p.drop60) - py) ** 2; if (dd < d) { d = dd; best = i; } });
    setHover(best < 0 ? null : best);
  };
  const h = hover != null ? list[hover] : null;
  return (
    <div ref={ref} className="chart" data-g="heart">
      <svg width={w} height={H} role="img" aria-label={`Pulsabfall 60 Sekunden nach ${list.length} Trainings, mit Wochenmedian`}
        onPointerMove={pick} onPointerLeave={() => setHover(null)}>
        {ys.map((v) => (
          <g key={v}>
            <line x1={L} x2={w - R} y1={y(v)} y2={y(v)} className="grid" />
            <text x={L - 6} y={y(v) + 3} className="axis" textAnchor="end">{fmt(v)}</text>
          </g>
        ))}
        <text x={L} y={H - 6} className="axis">{short(localDay(new Date(t0)))}</text>
        <text x={w - R} y={H - 6} className="axis" textAnchor="end">{short(localDay(new Date(t1)))}</text>
        {wk.length > 1 && <path d={path} className="line-fit" />}
        {list.map((p, i) => <circle key={p.end} cx={x(p.end)} cy={y(p.drop60)} r={hover === i ? 5 : 4} className="chart-dot" />)}
      </svg>
      {h && (
        <div className="chart-tip" style={{ left: Math.min(w - 170, Math.max(0, x(h.end) + 10)) }}>
          <b>{short(h.day)} · {fmt(h.minutes)} min Training</b>
          <span>{fmt(h.hr_end)} bpm am Ende</span>
          <span>−{fmt(h.drop60)} bpm nach 60 s{h.drop120 != null ? ` · −${fmt(h.drop120)} nach 120 s` : ""}</span>
          <span className="muted">{h.src}</span>
        </div>
      )}
      <div className="chart-legend" aria-hidden="true">
        <span><i className="chart-key dot-key" />ein Training</span>
        <span><i className="chart-key fit-key" />Median je Woche</span>
      </div>
    </div>
  );
}

const AFTER_LABEL: Record<string, [string, string, number]> = { rhr: ["Ruhepuls", "bpm", 1], hrv: ["HRV", "ms", 1] };
const AFTER_MAX: Record<string, number> = { rhr: 6, hrv: 15 };

export function Load() {
  const [view, setView] = useState<LoadView | null>(null);
  const [error, setError] = useState("");
  const load = () => invoke<LoadView>("load_view", { today: localDay() }).then((v) => { setView(v); setError(""); }).catch((e) => setError(String(e)));
  useEffect(() => {
    void load();
    const un = listen<unknown>("health", (e) => { if (e.payload) void load(); });
    const t = setInterval(() => void load(), 30 * 60_000);
    return () => { void un.then((f) => f()); clearInterval(t); };
  }, []);

  if (!view || !view.days.length) {
    return (
      <section className="days" aria-label="Belastung">
        <div className="n-eyebrow">Belastung</div>
        <p className="fine">{error || (view ? "Noch keine Trainings mit Puls. Sobald Trainings aus Health da sind, erscheinen sie hier." : "…")}</p>
      </section>
    );
  }
  const last = view.days[view.days.length - 1];
  const form = last.fitness - last.fatigue;
  const recent = view.recoveries.filter((r) => r.end > Date.now() - 28 * 86_400_000).map((r) => r.drop60);
  const before = view.recoveries.filter((r) => r.end <= Date.now() - 28 * 86_400_000 && r.end > Date.now() - 56 * 86_400_000).map((r) => r.drop60);
  const lastRec = view.recoveries[view.recoveries.length - 1];

  return (
    <section className="days" aria-label="Belastung">
      <article className="day-group n-card lg" data-g="move">
        <div className="n-eyebrow">Trainingslast · 90 Tage</div>
        <div className="stat-row" title={form < 0 ? "Die letzten 7 Tage waren belastender als die letzten 6 Wochen im Schnitt." : "Die letzten 7 Tage waren weniger belastend als die letzten 6 Wochen im Schnitt."}>
          <div className="stat"><span className="n-eyebrow">Fitness</span><b>{fmt(last.fitness)}</b></div>
          <div className="stat"><span className="n-eyebrow">Ermüdung</span><b>{fmt(last.fatigue)}</b></div>
          <div className="stat"><span className="n-eyebrow">Form</span><b>{signed(form)}</b></div>
        </div>
        <DivBar value={form} max={Math.max(20, Math.abs(form) * 1.5)} label={`Form ${signed(form)}: ${form < 0 ? "die letzten 7 Tage waren belastender" : "die letzten 7 Tage waren weniger belastend"} als die letzten 6 Wochen im Schnitt`} />
        <LoadChart days={view.days} />
      </article>

      <article className="day-group n-card lg" data-g="heart">
        <div className="n-eyebrow">Pulserholung · nach dem Training</div>
        {lastRec ? (
          <>
            <div className="stat-row two" title={`Zuletzt ${short(lastRec.day)}, ab ${fmt(lastRec.hr_end)} bpm`}>
              <div className="stat"><span className="n-eyebrow">nach 60 s</span><b>−{fmt(lastRec.drop60)}</b></div>
              {lastRec.drop120 != null && <div className="stat"><span className="n-eyebrow">nach 120 s</span><b>−{fmt(lastRec.drop120)}</b></div>}
            </div>
            {recent.length >= 3 && before.length >= 3 && (
              <DivBar value={median(recent) - median(before)} max={15}
                label={`Median der letzten 4 Wochen −${fmt(median(recent))} bpm, 4 Wochen davor −${fmt(median(before))} bpm; rechts = stärkerer Abfall`} />
            )}
            {view.recoveries.length >= 2 && <RecoveryChart list={view.recoveries} />}
            <details className="hint-i">
              <summary>?</summary>
              <p>
                Pulsabfall in bpm nach Trainingsende{recent.length >= 3 ? `; Median der letzten 4 Wochen −${fmt(median(recent))} bpm (n = ${recent.length})` : ""}
                {before.length >= 3 ? `, 4 Wochen davor −${fmt(median(before))} bpm (n = ${before.length}). Der Balken zeigt den Unterschied: rechts = stärkerer Abfall als davor` : ""}.
              </p>
            </details>
          </>
        ) : (
          <p className="fine">Noch keine Messung: gebraucht wird ein Training ab 10 min, am Ende über 60 % des Höchstpulses, und Puls in den 2 Minuten danach.</p>
        )}
      </article>

      {view.hard != null && !!view.after.length && (
        <article className="day-group n-card lg" data-g="move">
          <div className="n-eyebrow">Nach harten Tagen · 365 Tage</div>
          <div className="rows">
            {view.after.map((a) => {
              const [label, unit, d] = AFTER_LABEL[a.key] ?? [a.key, "", 1];
              const diff = a.mean - a.other_mean, clear = Number.isFinite(a.ci) && Math.abs(diff) > a.ci;
              const said = `${label}, ${a.lag === 1 ? "Tag danach" : "2 Tage danach"}: ${signed(a.mean, d)} ${unit} (übrige Tage ${signed(a.other_mean, d)}), ± ${fmt(a.ci, d)}, n = ${a.n}; ${clear ? (diff > 0 ? "höher als sonst" : "niedriger als sonst") : "kein deutlicher Unterschied"}`;
              return (
                <div className="row-viz" key={`${a.key}${a.lag}`} title={said}>
                  <span>{label} · +{a.lag} T</span>
                  <DivBar value={diff} max={AFTER_MAX[a.key] ?? 10} ci={a.ci} clear={clear} label={said} />
                  <span>{signed(a.mean, d)}</span>
                </div>
              );
            })}
          </div>
          <details className="hint-i">
            <summary>?</summary>
            <p>Hart = Trainingslast ab {fmt(view.hard)} (die oberen 20 % deiner Trainingstage). Der Balken zeigt den Abstand zu deinem Median der 60 Tage davor, der Strich den 95-%-Bereich. Gefüllter Punkt = deutlicher Unterschied, hohler Punkt = kein deutlicher Unterschied.</p>
          </details>
        </article>
      )}
      {error && <p className="fine warn" role="alert">{error}</p>}
      <details className="more n-card lg">
        <summary>Wie wird gerechnet?</summary>
        <p className="fine">
          Trainingslast (Edwards-TRIMP): je Trainingsminute 1 bis 5 Punkte nach Pulszone (50–60 % bis über 90 % von 200 bpm, wie auf der Seite
          Puls). Puls vom Helio Strap, sonst aus Health. Fitness und Ermüdung (Banister) sind gleitende Mittel der Last über 42 und 7 Tage,
          Form ist ihr Unterschied. Tage ohne erfasstes Training zählen als 0. Pulserholung: Puls in den 15 s vor Trainingsende minus Puls 60 s
          (und 120 s) danach, nur bei Trainings ab 10 min. „± …“ ist der 95-%-Bereich; „deutlich“ heißt, der Unterschied zu den übrigen Tagen
          ist größer als dieser Bereich. Das beschreibt deine Werte und ist keine Trainingsempfehlung.
        </p>
      </details>
    </section>
  );
}

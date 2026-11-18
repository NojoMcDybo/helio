import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { clock, dur, fmt } from "./metrics";
import { gapPath, localDay, short, useWidth } from "./chart";
import type { ReactNode } from "react";
import { ArcGauge, ClockDial, HeatStrip } from "./viz";

// Seite „Rhythmus“: Tagesprofil mit Kosinor, IS/IV, Schlafregelmäßigkeit (SRI), sozialer Jetlag (rhythm.rs).
// Diagramme einfarbig in Tinte wie auf der Seite Belastung; Kacheln wie auf der Seite Heute.
type Cosinor = { mesor: number; amp: number; peak: number; r2: number; n: number };
type Week = { start: string; days: number; steps: Cosinor | null; hr: Cosinor | null; is: number | null; iv: number | null; sri: number | null; mid: number | null };
type Jetlag = { work: number; free: number; n_work: number; n_free: number };
type RhythmView = { to: string; profile: (number | null)[]; now: Week; sri_before: number | null; jetlag: Jetlag | null; weeks: Week[] };

const hourClock = (h: number) => clock(h * 60);
const cos = (c: Cosinor, t: number) => c.mesor + c.amp * Math.cos((2 * Math.PI * (t - c.peak)) / 24);

function ProfileChart({ profile, fit }: { profile: (number | null)[]; fit: Cosinor | null }) {
  const [ref, w] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const H = 150, L = 34, R = 6, T = 8, B = 20;
  const max = Math.max(1, ...profile.map((v) => v ?? 0), fit ? fit.mesor + fit.amp : 0);
  const x = (t: number) => L + (t / 24) * (w - L - R), y = (v: number) => T + (1 - Math.max(0, v) / max) * (H - T - B);
  const bw = Math.max(2, ((w - L - R) / 24) * 0.6);
  const curve = fit ? Array.from({ length: 97 }, (_, i) => i / 4).map((t, i) => `${i ? "L" : "M"}${x(t).toFixed(1)} ${y(cos(fit, t)).toFixed(1)}`).join("") : "";
  const pick = (e: React.PointerEvent) => {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    setHover(Math.max(0, Math.min(23, Math.floor(((e.clientX - r.left - L) / (w - L - R)) * 24))));
  };
  const v = hover != null ? profile[hover] : null;
  return (
    <div ref={ref} className="chart" data-g="move" data-g2="warm">
      <svg width={w} height={H} role="img" aria-label="Mittlere Schritte je Stunde über 4 Wochen mit angepasster Tageskurve"
        onPointerMove={pick} onPointerLeave={() => setHover(null)}>
        <line x1={L} x2={w - R} y1={y(0)} y2={y(0)} className="grid" />
        <line x1={L} x2={w - R} y1={y(max)} y2={y(max)} className="grid" />
        <text x={L - 6} y={y(max) + 3} className="axis" textAnchor="end">{fmt(max)}</text>
        <text x={L - 6} y={y(0) + 3} className="axis" textAnchor="end">0</text>
        {[0, 6, 12, 18, 24].map((h) => <text key={h} x={x(h)} y={H - 6} className="axis" textAnchor={h ? (h < 24 ? "middle" : "end") : "start"}>{hourClock(h % 24)}</text>)}
        {profile.map((p, h) => p != null && p > 0 && (
          <rect key={h} x={x(h + 0.5) - bw / 2} y={y(p)} width={bw} height={y(0) - y(p)} rx={Math.min(2, bw / 2)} className="load-bar" />
        ))}
        {fit && <path d={curve} className="line-fat" />}
        {hover != null && <line x1={x(hover + 0.5)} x2={x(hover + 0.5)} y1={T} y2={H - B} className="crosshair" />}
      </svg>
      {hover != null && (
        <div className="chart-tip" style={{ left: Math.min(w - 150, Math.max(0, x(hover + 0.5) + 10)) }}>
          <b>{hourClock(hover)}–{hourClock(hover + 1)}</b>
          <span><i className="chart-key bar-key" />{v != null ? `${fmt(v)} Schritte im Mittel` : "keine Werte"}</span>
          {fit && <span><i className="chart-key fat-key" />{fmt(Math.max(0, cos(fit, hover + 0.5)))} laut Kurve</span>}
        </div>
      )}
      <div className="chart-legend" aria-hidden="true">
        <span><i className="chart-key bar-key" />Schritte je Stunde, Mittel über 4 Wochen</span>
        {fit && <span><i className="chart-key fat-key" />angepasste 24-h-Kurve</span>}
      </div>
    </div>
  );
}

/// Uhrzeiten je Woche: Schlafmitte und Höchstwert der Schritte, eine Achse (beides Uhrzeit)
function ClockChart({ weeks }: { weeks: Week[] }) {
  const [ref, w] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const H = 170, L = 38, R = 6, T = 8, B = 20, n = weeks.length;
  const mid = weeks.map((k) => (k.mid != null ? k.mid / 60 : null)), peak = weeks.map((k) => k.steps?.peak ?? null);
  const all = [...mid, ...peak].filter((v): v is number => v != null);
  const lo = Math.floor(Math.min(...all) / 3) * 3, hi = Math.ceil(Math.max(...all) / 3) * 3 || lo + 3;
  const ys = Array.from({ length: (hi - lo) / 3 + 1 }, (_, i) => lo + i * 3);
  const x = (i: number) => L + ((i + 0.5) / n) * (w - L - R), y = (h: number) => T + ((hi - h) / (hi - lo || 1)) * (H - T - B);
  const pick = (e: React.PointerEvent) => {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    setHover(Math.max(0, Math.min(n - 1, Math.round(((e.clientX - r.left - L) / (w - L - R)) * n - 0.5))));
  };
  const h = hover != null ? weeks[hover] : null;
  return (
    <div ref={ref} className="chart" data-g="sleep" data-g2="move">
      <svg width={w} height={H} role="img" aria-label={`Schlafmitte und Uhrzeit der meisten Schritte je Woche, ${n} Wochen`}
        onPointerMove={pick} onPointerLeave={() => setHover(null)}>
        {ys.map((v) => (
          <g key={v}>
            <line x1={L} x2={w - R} y1={y(v)} y2={y(v)} className="grid" />
            <text x={L - 6} y={y(v) + 3} className="axis" textAnchor="end">{hourClock(v)}</text>
          </g>
        ))}
        <text x={x(0)} y={H - 6} className="axis">{short(weeks[0].start)}</text>
        <text x={x(n - 1)} y={H - 6} className="axis" textAnchor="end">{short(weeks[n - 1].start)}</text>
        <path d={gapPath(mid, x, y, 0)} className="line-fit" />
        <path d={gapPath(peak, x, y, 0)} className="line-fat" />
        {[mid, peak].flatMap((s, k) => s.map((v, i) => v != null && s[i - 1] == null && s[i + 1] == null && <circle key={`${k}-${i}`} cx={x(i)} cy={y(v)} r={3} className="chart-dot" />))}
        {h && hover != null && <line x1={x(hover)} x2={x(hover)} y1={T} y2={H - B} className="crosshair" />}
      </svg>
      {h && hover != null && (
        <div className="chart-tip" style={{ left: Math.min(w - 170, Math.max(0, x(hover) + 10)) }}>
          <b>Woche ab {short(h.start)}</b>
          <span><i className="chart-key fit-key" />Schlafmitte {h.mid != null ? clock(h.mid) : "–"}</span>
          <span><i className="chart-key fat-key" />meiste Schritte {h.steps ? `gegen ${hourClock(h.steps.peak)}` : "–"}</span>
        </div>
      )}
      <div className="chart-legend" aria-hidden="true">
        <span><i className="chart-key fit-key" />Schlafmitte (Median der Woche)</span>
        <span><i className="chart-key fat-key" />Höchstwert der Tageskurve, Schritte</span>
      </div>
    </div>
  );
}

function Tile({ label, value, note, series, g, viz }: { label: string; value: string; note: string; series?: (number | null)[]; g: string; viz?: ReactNode }) {
  return (
    <div className="trend-tile lg" data-g={g} title={`${label}: ${value}. ${note}`}>
      <span className="n-eyebrow">{label}</span>
      <b>{value}</b>
      {viz}
      {series && <HeatStrip values={series} label={`${label}, Verlauf der letzten Wochen, dunkler = höher`} />}
    </div>
  );
}

export function Rhythm() {
  const [view, setView] = useState<RhythmView | null>(null);
  const [error, setError] = useState("");
  const load = () => invoke<RhythmView | null>("rhythm_view", { today: localDay() }).then((v) => { setView(v); setError(""); }).catch((e) => setError(String(e)));
  useEffect(() => {
    void load();
    const un = listen<unknown>("health", (e) => { if (e.payload) void load(); });
    const t = setInterval(() => void load(), 30 * 60_000);
    return () => { void un.then((f) => f()); clearInterval(t); };
  }, []);

  const now = view?.now;
  if (!view || !now || (!now.days && now.sri == null)) {
    return (
      <section className="days" aria-label="Rhythmus">
        <div className="n-eyebrow">Rhythmus</div>
        <p className="fine">{error || (view ? "Noch zu wenig Werte aus den letzten 4 Wochen. Gebraucht werden Schritte über den Tag und erfasste Nächte." : "…")}</p>
      </section>
    );
  }
  const weeks = view.weeks, j = view.jetlag, diff = j ? j.free - j.work : 0;
  const shown = weeks.filter((k) => k.mid != null || k.steps).length >= 2;

  return (
    <section className="days" aria-label="Rhythmus">
      <article className="day-group n-card lg" data-g="move">
        <div className="n-eyebrow">Tagesprofil · 4 Wochen bis {short(view.to)}</div>
        {(now.steps || now.hr) && (
          <div className="pair" title={now.steps ? `Die Kurve erklärt ${fmt(now.steps.r2 * 100)} % der Schwankung zwischen den Stunden (${now.days} Tage mit Schritten)${now.hr ? `; Puls im Mittel ${fmt(now.hr.mesor)} bpm, ± ${fmt(now.hr.amp)} über den Tag` : ""}` : undefined}>
            <ClockDial label="Uhrzeit des Höchstwerts: Schritte und Puls"
              dots={[...(now.steps ? [{ v: now.steps.peak * 60 }] : []), ...(now.hr ? [{ v: now.hr.peak * 60, kind: "alt" as const }] : [])]} />
            <div className="pair-nums">
              {now.steps && <div><span className="n-eyebrow">Schritte</span>{hourClock(now.steps.peak)}</div>}
              {now.hr && <div><span className="n-eyebrow">Puls</span>{hourClock(now.hr.peak)}</div>}
            </div>
          </div>
        )}
        <ProfileChart profile={view.profile} fit={now.steps} />
      </article>

      <article className="day-group group-bare">
        <div className="n-eyebrow">Regelmäßigkeit · 4 Wochen</div>
        <div className="trend-tiles">
          <Tile g="sleep" label="Schlafregelmäßigkeit (SRI)" value={now.sri != null ? fmt(now.sri) : "–"} series={weeks.map((k) => k.sri)}
            viz={<ArcGauge min={0} max={100} value={now.sri} ghost={view.sri_before} label={`SRI ${now.sri != null ? fmt(now.sri) : "–"} von 100${view.sri_before != null ? `, 4 Wochen davor ${fmt(view.sri_before)}` : ""}`} />}
            note={now.sri == null ? "zu wenig erfasste Nächte" : view.sri_before != null ? `4 Wochen davor ${fmt(view.sri_before)} · 100 = jeden Tag gleich` : "100 = jeden Tag gleich"} />
          <Tile g="move" label="Gleichmaß zwischen Tagen (IS)" value={now.is != null ? fmt(now.is, 2) : "–"} series={weeks.map((k) => k.is)}
            viz={<ArcGauge min={0} max={1} value={now.is} label={`IS ${now.is != null ? fmt(now.is, 2) : "–"} von 1`} />}
            note="0 bis 1 · 1 = jeder Tag mit gleichem Bewegungsmuster" />
          <Tile g="move" label="Wechsel innerhalb des Tages (IV)" value={now.iv != null ? fmt(now.iv, 2) : "–"} series={weeks.map((k) => k.iv)}
            viz={<ArcGauge min={0} max={2} value={now.iv} label={`IV ${now.iv != null ? fmt(now.iv, 2) : "–"} von 2`} />}
            note="0 bis 2 · höher = häufiger Wechsel zwischen ruhig und aktiv" />
          <Tile g="sleep" label="Sozialer Jetlag" value={!j ? "–" : Math.abs(diff) < 15 ? "±0" : `${diff > 0 ? "+" : "−"}${dur(Math.abs(diff))}`}
            viz={j && <ClockDial label={`Schlafmitte an freien Tagen ${clock(j.free)}, an Werktagen ${clock(j.work)}`}
              spans={[{ from: Math.min(j.work, j.free), to: Math.max(j.work, j.free) }]} dots={[{ v: j.work }, { v: j.free, kind: "alt" }]} />}
            note={!j ? "zu wenig Nächte vor freien Tagen" : `Schlafmitte an freien Tagen ${clock(j.free)}, an Werktagen ${clock(j.work)} (${j.n_free} und ${j.n_work} Nächte)`} />
        </div>
      </article>

      {shown && (
        <article className="day-group n-card lg" data-g="sleep">
          <div className="n-eyebrow">Uhrzeiten je Woche · {weeks.length} Wochen</div>
          <ClockChart weeks={weeks} />
        </article>
      )}
      {error && <p className="fine warn" role="alert">{error}</p>}
      <details className="more n-card lg">
        <summary>Wie wird gerechnet?</summary>
        <p className="fine">
          Tageskurve (Kosinor): eine 24-Stunden-Welle, die am besten zu deinen Stundenwerten passt; daraus die Uhrzeit des Höchstwerts. IS und IV
          (Witting 1990) aus Schritten je Stunde, nur Tage ab 500 Schritten. SRI (Phillips 2017): wie oft du 24 Stunden später im selben Zustand
          bist, schlafend oder wach, in 5-Minuten-Abschnitten; nur Tage, deren Nacht davor und danach erfasst ist. Sozialer Jetlag (Wittmann 2006):
          Median der Schlafmitte in den Nächten auf Samstag und Sonntag minus Nächte auf Werktage, Feiertage zählen als Werktage. Das beschreibt
          deinen Tagesablauf und ist keine Bewertung und kein medizinischer Befund.
        </p>
      </details>
    </section>
  );
}

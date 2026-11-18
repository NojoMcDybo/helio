import type { ReactNode } from "react";

// Grafikbausteine statt Text. Alle zeichnen mit currentColor (Tinte der Kachel); die Marke („jetzt“) hat die Klasse .mk
// und nimmt die Markenfarbe (--mk) der Kachel. Darstellungsarten (Recherche: Uhr-Ring für Uhrzeiten, Bogenmesser, Streifen
// gegen den eigenen Bereich, Balkenverlauf, Wochen-Heatmap, gestapelter Schlafbalken, Abweichungsbalken):
//  ArcGauge  Wert auf einem Halbkreis mit üblichem Bereich       ClockDial  24-Stunden-Ring (Uhrzeiten, Schlaffenster)
//  DotStrip  die letzten Tage als Punkte, heute hervorgehoben     BarSpark   Tagesbalken mit üblichem Band
//  HeatStrip Wochen als Farbstreifen                              StageBar   Schlafphasen als Balken
//  DivBar    Abweichung links/rechts von null, mit Spielraum      StatusDots eine Marke je Kennzahl
const TAU = Math.PI * 2;
const clamp = (x: number, a = 0, b = 1) => Math.min(b, Math.max(a, x));
const pt = (cx: number, cy: number, r: number, a: number) => [cx + r * Math.cos(a), cy + r * Math.sin(a)] as const;
/// Bogen im Uhrzeigersinn von a0 nach a1 (Bogenmaß, 0 = rechts, SVG-Richtung)
const arc = (cx: number, cy: number, r: number, a0: number, a1: number) => {
  const [x0, y0] = pt(cx, cy, r, a0), [x1, y1] = pt(cx, cy, r, a1);
  return `M${x0.toFixed(2)} ${y0.toFixed(2)}A${r} ${r} 0 ${a1 - a0 > Math.PI ? 1 : 0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`;
};
const range = (vals: number[], pad = 0.1) => {
  const lo = Math.min(...vals), hi = Math.max(...vals), p = (hi - lo) * pad || 1;
  return [lo - p, hi + p] as const;
};
type Svg = { label: string; className?: string };
const Frame = ({ w, h, label, className, children }: Svg & { w: number; h: number; children: ReactNode }) => (
  <svg viewBox={`0 0 ${w} ${h}`} className={"viz " + (className ?? "")} role="img" aria-label={label}>{children}</svg>
);

/// Halbkreis-Messer: Spur mit Punkten, üblicher Bereich als dickes Band, Wert als Marke (ghost = Vergleichswert, hohl).
export function ArcGauge({ min, max, value, lo, hi, ghost, label }: { min: number; max: number; value: number | null; lo?: number; hi?: number; ghost?: number | null } & Svg) {
  const cx = 66, cy = 68, r = 52, ang = (v: number) => Math.PI + clamp((v - min) / (max - min || 1)) * Math.PI;
  const dots = Array.from({ length: 9 }, (_, i) => pt(cx, cy, r + 10, Math.PI + (i / 8) * Math.PI));
  const [vx, vy] = value != null ? pt(cx, cy, r, ang(value)) : [0, 0];
  const [gx, gy] = ghost != null ? pt(cx, cy, r, ang(ghost)) : [0, 0];
  return (
    <Frame w={132} h={80} label={label} className="gauge">
      {dots.map(([x, y], i) => <circle key={i} cx={x} cy={y} r={1.3} className="soft" />)}
      <path d={arc(cx, cy, r, Math.PI, TAU)} className="track" />
      {lo != null && hi != null && <path d={arc(cx, cy, r, ang(lo), Math.max(ang(hi), ang(lo) + 0.06))} className="band" />}
      {ghost != null && <circle cx={gx} cy={gy} r={4} className="ghost" />}
      {value != null && <circle cx={vx} cy={vy} r={6.5} className="mk" />}
    </Frame>
  );
}

export type Dial = { v: number; kind?: "main" | "alt" | "ghost" };
/// 24-Stunden-Ring, oben = Mitternacht. `usual` und `spans` sind Bögen (Minuten ab 0 Uhr), `dots` Marken.
export function ClockDial({ usual, spans, dots, label }: { usual?: { lo: number; hi: number } | null; spans?: { from: number; to: number }[]; dots: Dial[] } & Svg) {
  const c = 50, r = 36, ang = (m: number) => -Math.PI / 2 + (((m % 1440) + 1440) % 1440) / 1440 * TAU;
  const span = (from: number, to: number, cls: string) => {
    const len = ((to - from) % 1440 + 1440) % 1440;
    return len >= 1439 ? <circle key={cls + from} cx={c} cy={c} r={r} className={cls} /> : <path key={cls + from + to} d={arc(c, c, r, ang(from), ang(from) + Math.max(0.05, (len / 1440) * TAU))} className={cls} />;
  };
  return (
    <Frame w={100} h={100} label={label} className="dial">
      <circle cx={c} cy={c} r={r} className="track" />
      {[0, 6, 12, 18].map((h) => { const [x0, y0] = pt(c, c, r + 8, ang(h * 60)), [x1, y1] = pt(c, c, r + 13, ang(h * 60)); return <line key={h} x1={x0} y1={y0} x2={x1} y2={y1} className="soft-line" />; })}
      {usual && span(usual.lo, usual.hi, "band")}
      {spans?.map((s) => span(s.from, s.to, "span"))}
      {dots.map((d, i) => { const [x, y] = pt(c, c, r, ang(d.v)); return <circle key={i} cx={x} cy={y} r={d.kind === "ghost" ? 4 : 6} className={d.kind === "alt" ? "mk alt" : d.kind === "ghost" ? "ghost" : "mk"} />; })}
    </Frame>
  );
}

/// Die letzten Tage als Punkte auf einer Linie, üblicher Bereich als Band, heute als Marke.
export function DotStrip({ series, value, lo, hi, label }: { series: (number | null)[]; value: number | null; lo?: number; hi?: number } & Svg) {
  const W = 140, H = 34, vals = series.filter((v): v is number => v != null);
  const all = [...vals, ...(value != null ? [value] : []), ...(lo != null && hi != null ? [lo, hi] : [])];
  if (!all.length) return null;
  const [a, b] = range(all), x = (v: number) => 4 + ((v - a) / (b - a)) * (W - 8);
  return (
    <Frame w={W} h={H} label={label} className="strip">
      {lo != null && hi != null && <rect x={x(lo)} y={4} width={Math.max(3, x(hi) - x(lo))} height={H - 8} rx={5} className="band" />}
      <line x1={4} x2={W - 4} y1={H / 2} y2={H / 2} className="soft-line" />
      {series.map((v, i) => v != null && <circle key={i} cx={x(v)} cy={H / 2 + ((i % 3) - 1) * 6} r={2} className="dotp" />)}
      {value != null && <circle cx={x(value)} cy={H / 2} r={6} className="mk" />}
    </Frame>
  );
}

/// Tagesbalken vom Nullpunkt; üblicher Bereich als Band; der letzte Tag ist die Marke.
export function BarSpark({ series, lo, hi, label }: { series: (number | null)[]; lo?: number; hi?: number } & Svg) {
  const W = 140, H = 38, n = series.length, vals = series.filter((v): v is number => v != null);
  if (!vals.length) return null;
  const top = Math.max(...vals, hi ?? 0) * 1.08 || 1, y = (v: number) => H - 3 - (v / top) * (H - 6), bw = W / n;
  return (
    <Frame w={W} h={H} label={label} className="bars">
      {lo != null && hi != null && <rect x={0} y={y(hi)} width={W} height={Math.max(2, y(lo) - y(hi))} className="band" />}
      {series.map((v, i) => v != null && <rect key={i} x={i * bw + bw * 0.18} y={y(v)} width={bw * 0.64} height={Math.max(1.5, H - 3 - y(v))} rx={Math.min(2, bw * 0.3)} className={i === n - 1 ? "mk" : "bar"} />)}
    </Frame>
  );
}

/// Wochen als Streifen: je dunkler, desto höher (innerhalb der eigenen Spanne).
export function HeatStrip({ values, label, className }: { values: (number | null)[] } & Svg) {
  const vals = values.filter((v): v is number => v != null);
  if (vals.length < 2) return null;
  const lo = Math.min(...vals), hi = Math.max(...vals), n = values.length, W = 140, H = 16, cw = W / n;
  return (
    <Frame w={W} h={H} label={label} className={"heat " + (className ?? "")}>
      {values.map((v, i) => <rect key={i} x={i * cw + 0.5} y={1} width={Math.max(1, cw - 0.6)} height={H - 2} rx={1} className={v == null ? "gap" : "cell"} style={v == null ? undefined : { fillOpacity: 0.14 + 0.86 * ((v - lo) / (hi - lo || 1)) ** 1.2 }} />)}
    </Frame>
  );
}

/// Schlafphasen in einem Balken (Minuten): tief, kern, REM, wach.
export function StageBar({ deep, core, rem, awake, label }: { deep: number; core: number; rem: number; awake: number } & Svg) {
  const parts = [["deep", deep], ["core", core], ["rem", rem], ["awake", awake]] as const;
  const total = parts.reduce((s, [, m]) => s + m, 0);
  if (!total) return null;
  const W = 200, H = 14, gap = 2, avail = W - gap * 3;
  let x = 0;
  return (
    <Frame w={W} h={H} label={label} className="stages">
      {parts.map(([k, m], i) => {
        const w = Math.max(m > 0 ? 3 : 0, (m / total) * avail), rect = <rect key={k} x={x} y={0} width={w} height={H} rx={5} className={"stage-" + k} />;
        x += w + gap; void i;
        return m > 0 ? rect : null;
      })}
    </Frame>
  );
}

/// Abweichung von null: Balken nach links oder rechts, Spielraum als Strich, Marke gefüllt wenn deutlich.
export function DivBar({ value, max, ci, clear = true, label }: { value: number; max: number; ci?: number; clear?: boolean } & Svg) {
  const W = 140, H = 18, mid = W / 2, x = (v: number) => mid + clamp(v / (max || 1), -1, 1) * (mid - 8);
  return (
    <Frame w={W} h={H} label={label} className="div">
      <line x1={8} x2={W - 8} y1={H / 2} y2={H / 2} className="soft-line" />
      <line x1={mid} x2={mid} y1={2} y2={H - 2} className="zero" />
      <rect x={Math.min(mid, x(value))} y={H / 2 - 3} width={Math.abs(x(value) - mid)} height={6} rx={3} className="bar" />
      {ci != null && Number.isFinite(ci) && <line x1={x(value - ci)} x2={x(value + ci)} y1={H / 2} y2={H / 2} className="whisker" />}
      <circle cx={x(value)} cy={H / 2} r={5} className={clear ? "mk" : "ghost"} />
    </Frame>
  );
}

/// Eine Marke je Kennzahl; an = außerhalb des Üblichen.
export function StatusDots({ items, label }: { items: boolean[]; label: string }) {
  return <div className="status-dots" role="img" aria-label={label}>{items.map((on, i) => <i key={i} className={on ? "on" : ""} />)}</div>;
}

// ---------- Bausteine der Heute-Seite (Querformat) ----------
const FONT: Record<string, string> = {
  "0": "01110 10001 10011 10101 11001 10001 01110", "1": "00100 01100 00100 00100 00100 00100 01110", "2": "01110 10001 00001 00010 00100 01000 11111",
  "3": "11110 00001 00001 01110 00001 00001 11110", "4": "00010 00110 01010 10010 11111 00010 00010", "5": "11111 10000 11110 00001 00001 10001 01110",
  "6": "00110 01000 10000 11110 10001 10001 01110", "7": "11111 00001 00010 00100 01000 01000 01000", "8": "01110 10001 10001 01110 10001 10001 01110",
  "9": "01110 10001 10001 01111 00001 00010 01100",
};
/// Punktmatrix-Ziffern (5×7) für große Werte; Text steht zusätzlich als aria-label.
export function Dm({ text, h = 34, label }: { text: string; h?: number; label?: string }) {
  let x = 0;
  const dots: ReactNode[] = [];
  // --d: Verzögerung je Punkt (von links nach rechts), damit sich die Ziffern beim Erscheinen aufbauen (plate.css, dot-on)
  const add = (cx: number, cy: number, r: number) => dots.push(<circle key={dots.length} cx={cx} cy={cy} r={r} style={{ ["--d" as string]: `${Math.round(cx * 16 + cy * 5)}ms` }} />);
  for (const ch of text) {
    if (ch === ":" || ch === "." || ch === ",") {
      add(x + 1, ch === ":" ? 2 : 6, 0.38);
      if (ch === ":") add(x + 1, 5, 0.38);
      x += 2.2; continue;
    }
    if (ch === " ") { x += 3; continue; }
    const g = FONT[ch];
    if (!g) continue;
    g.split(" ").forEach((r, y) => [...r].forEach((b, i) => { if (b === "1") add(x + i + 0.5, y + 0.5, 0.4); }));
    x += 6;
  }
  return <svg key={text} className="dm" viewBox={`0 0 ${x} 7`} style={{ height: h, aspectRatio: `${x} / 7` }} role="img" aria-label={label ?? text}>{dots}</svg>;
}

/// Schlaf auf dem 24-Stunden-Ring (oben = Mitternacht): Bogen von Einschlafen bis Aufwachen, übliche Zeiten als Strichpunkte.
export function SleepRing({ onset, wake, size = 250, children, label }: { onset: number | null; wake: number | null; size?: number; children?: ReactNode; label: string }) {
  const c = 125, r = 108, ang = (m: number) => -Math.PI / 2 + (((m % 1440) + 1440) % 1440) / 1440 * TAU;
  const len = onset != null && wake != null ? ((wake - onset) % 1440 + 1440) % 1440 : 0;
  const ticks = Array.from({ length: 48 }, (_, i) => pt(c, c, 94, (i / 48) * TAU));
  return (
    <div className="sleep-ring" style={{ width: size, height: size }} role="img" aria-label={label}>
      <svg viewBox="0 0 250 250" width={size} height={size}>
        <circle cx={c} cy={c} r={r} className="ring-track" />
        {ticks.map(([x, y], i) => <circle key={i} cx={x} cy={y} r={i % 12 === 0 ? 1.8 : 1} className="ring-tick" />)}
        {len > 0 && onset != null && <path d={arc(c, c, r, ang(onset), ang(onset) + (len / 1440) * TAU)} className="ring-span" />}
        {onset != null && (() => { const [x, y] = pt(c, c, r, ang(onset)); return <circle cx={x} cy={y} r={9} className="mk" />; })()}
        {wake != null && (() => { const [x, y] = pt(c, c, r, ang(wake)); return <circle cx={x} cy={y} r={9} className="mk alt" />; })()}
      </svg>
      <div className="ring-center">{children}</div>
    </div>
  );
}

/// Waagerechte Skala: üblicher Bereich als dickerer Abschnitt, heute als Marke. Werte werden auf min..max abgebildet.
export function RangeRail({ min, max, value, lo, hi, label }: { min: number; max: number; value: number | null; lo?: number; hi?: number; label: string }) {
  const p = (v: number) => `${clamp((v - min) / (max - min || 1)) * 100}%`;
  return (
    <div className="rail-bar" role="img" aria-label={label}>
      {lo != null && hi != null && <s style={{ left: p(lo), width: `calc(${p(hi)} - ${p(lo)})` }} />}
      {value != null && <u style={{ left: p(value) }} />}
    </div>
  );
}

/// Verlauf als Linie (Lücken bleiben Lücken), letzter Wert als Marke.
/// Verlauf mit üblichem Band (und Median gestrichelt). mini = Kachelgröße ohne Beschriftung, sonst für die Einzelansicht.
export function BandChart({ series, lo, hi, median, mini, label }: { series: (number | null)[]; lo?: number; hi?: number; median?: number; mini?: boolean } & Svg) {
  const vals = series.filter((v): v is number => v != null), n = series.length;
  if (vals.length < 2) return null;
  const w = mini ? 240 : 560, h = mini ? 56 : 200, p = mini ? 4 : 8;
  const [a, b] = range(lo != null && hi != null ? [...vals, lo, hi] : vals, 0.1);
  const x = (i: number) => p + (i / Math.max(1, n - 1)) * (w - 2 * p), y = (v: number) => h - p - ((v - a) / (b - a || 1)) * (h - 2 * p);
  const d = series.reduce<string>((s, v, i) => (v == null ? s : s + (i && series[i - 1] != null ? "L" : "M") + `${x(i).toFixed(1)} ${y(v).toFixed(1)}`), "");
  const li = n - 1 - [...series].reverse().findIndex((v) => v != null);
  return (
    <svg viewBox={`0 0 ${w} ${h}`} className={"viz band-chart" + (mini ? " mini" : "")} role="img" aria-label={label}>
      {lo != null && hi != null && <rect className="bc-band" x={0} width={w} y={y(hi)} height={Math.max(2, y(lo) - y(hi))} rx={mini ? 3 : 6} />}
      {!mini && median != null && <line className="bc-med" x1={0} x2={w} y1={y(median)} y2={y(median)} />}
      <path d={d} className="bc-line" />
      <circle cx={x(li)} cy={y(series[li] as number)} r={mini ? 3 : 5} className="mk" />
    </svg>
  );
}

export function LineSpark({ series, w = 260, h = 60, label }: { series: (number | null)[]; w?: number; h?: number; label: string }) {
  const vals = series.filter((v): v is number => v != null);
  if (vals.length < 2) return null;
  const [a, b] = range(vals, 0.2), x = (i: number) => 3 + (i / Math.max(1, series.length - 1)) * (w - 6), y = (v: number) => h - 4 - ((v - a) / (b - a || 1)) * (h - 8);
  const d = series.reduce<string>((s, v, i) => (v == null ? s : s + (i && series[i - 1] != null ? "L" : "M") + `${x(i).toFixed(1)} ${y(v).toFixed(1)}`), "");
  const li = series.length - 1 - [...series].reverse().findIndex((v) => v != null);
  return (
    <svg viewBox={`0 0 ${w} ${h}`} className="viz line-spark" role="img" aria-label={label}>
      <path d={d} className="spark-path" />
      <circle cx={x(li)} cy={y(series[li] as number)} r={5} className="mk" />
    </svg>
  );
}

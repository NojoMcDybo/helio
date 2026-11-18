import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { METRIC, QUALITY, dur, fmt } from "./metrics";
import { gapPath, localDay } from "./chart";

// Seite „Heute“: letzte Nacht und letzter vollständiger Tag gegen den eigenen üblichen Bereich (normal.rs).
// Kein Gesamtwert, nur einzelne Abweichungen mit Abstand zum Median. Beschreibung, keine Bewertung.
// Kachel (Balken „heute gegen üblich“ + Verlauf mit Band) nach Haze `src/Body.tsx` TrendTile, an Helio angepasst.
type Usual = { median: number; lo: number; hi: number; n: number };
type Item = {
  key: string; day: string; value: number | null; src: string | null; quality: number;
  usual: Usual | null; status: "höher" | "niedriger" | "üblich" | null; series: (number | null)[];
};
type Debt = { minutes: number; need: number; half_life: number | null; rho: number | null; nights: number };
type Part = { key: string; value: number; usual: number; z: number };
type Check = { episodes: number; caught: number; runs: number; unmarked: number; years: number };
type Signal = { day: string; active: boolean; nights: number; score: number | null; threshold: number; parts: Part[]; maybe: string[]; history: (number | null)[]; check: Check };
type TodayView = { night_day: string | null; day_day: string | null; items: Item[]; debt: Debt | null; signal: Signal | null };

const NIGHT_KEYS = new Set(["sleep_min", "onset", "wake", "efficiency", "deep_min", "rem_min", "hr_night", "hrv", "rhr", "resp", "spo2"]);
const shortDate = (day: string) => new Date(`${day}T12:00:00`).toLocaleDateString("de-DE", { weekday: "short", day: "2-digit", month: "2-digit" });

/// Bei Uhrzeiten heißt „höher“ später.
const word = (it: Item) => {
  const clock = METRIC.get(it.key)?.kind === "clock";
  return it.status === "höher" ? (clock ? "später" : "höher") : it.status === "niedriger" ? (clock ? "früher" : "niedriger") : "";
};
const isOff = (it: Item) => it.status === "höher" || it.status === "niedriger";
/// Wie weit außerhalb des üblichen Bereichs, in Bereichsbreiten (0 = drin)
const reach = (it: Item) => {
  const u = it.usual, v = it.value;
  if (!u || v == null) return 0;
  return Math.max(u.lo - v, v - u.hi, 0) / (u.hi - u.lo || Math.abs(u.median) || 1);
};
const RECOVERY = new Set(["sleep_min", "hrv", "rhr", "hr_night", "resp"]);
const ALERT_KEY = "helio-notch-alert";

/// Notch „Heute“: die zwei auffälligsten Abweichungen; einmal am Tag kurz aufklappen,
/// wenn nach der Nacht Schlaf oder Erholung deutlich (≥ halbe Bereichsbreite) außerhalb liegen.
function toNotch(view: TodayView) {
  // Frühzeichen zuerst; dann bleibt Platz für eine Abweichung
  const sig = view.signal?.active ? view.signal : null;
  const top = view.items.filter(isOff).sort((a, b) => reach(b) - reach(a)).slice(0, sig ? 1 : 2);
  const text = [...(sig ? ["Frühzeichen wie vor einem Infekt"] : []), ...top.map((i) => `${METRIC.get(i.key)?.label ?? i.key} ${word(i)}`)].join(" · ") || null;
  const today = localDay();
  let alert = false;
  if ((sig && sig.day === today) || top.some((i) => i.day === today && RECOVERY.has(i.key) && reach(i) >= 0.5)) {
    try { alert = localStorage.getItem(ALERT_KEY) !== today; } catch { /* ohne Speicher lieber kein Aufklappen als eins bei jedem Abgleich */ }
  }
  // Erst als gezeigt merken, wenn es wirklich rausging (Notch im Tray an)
  void invoke<boolean>("notch_today", { text, alert })
    .then((sent) => { if (alert && sent) try { localStorage.setItem(ALERT_KEY, today); } catch { /* s. o. */ } })
    .catch(() => {});
}

const distance = (it: Item, d: number) => {
  const m = METRIC.get(it.key)!;
  return m.kind === "value" ? m.show(Math.abs(d)) : dur(Math.abs(d));
};

function Tile({ it }: { it: Item }) {
  const m = METRIC.get(it.key);
  if (!m) return null;
  const u = it.usual, v = it.value;
  const vals = it.series.filter((x): x is number => x != null);
  const lo = Math.min(...vals, u?.lo ?? Infinity, v ?? Infinity), hi = Math.max(...vals, u?.hi ?? -Infinity, v ?? -Infinity);
  const pad = (hi - lo) * 0.12 || 1, a = lo - pad, b = hi + pad, pc = (x: number) => ((x - a) / (b - a)) * 100;
  const W = 140, H = 34, n = it.series.length;
  const x = (i: number) => (i / Math.max(1, n - 1)) * W, y = (val: number) => H - ((val - a) / (b - a)) * H;
  const line = gapPath(it.series, x, y);
  const off = isOff(it);
  const arrow = it.status === "höher" ? "↑" : it.status === "niedriger" ? "↓" : "";
  const text = v == null ? (u ? "kein Wert" : "")
    : !u ? "noch kein Normalbereich (unter 14 Tagen)"
    : off ? `${word(it)} als üblich · ${distance(it, v - u.median)} ${v > u.median ? "über" : "unter"} dem Median`
    : it.status === "üblich" ? "im üblichen Bereich"
    : QUALITY[it.quality];
  return (
    <div className={"trend-tile" + (off ? " off" : "")} data-g={m.g}>
      <span className="n-eyebrow">{m.label}</span>
      <b>{v == null ? "–" : m.show(v)}{arrow && <em className={"dir " + (it.status === "höher" ? "up" : "down")} aria-hidden="true"> {arrow}</em>}</b>
      {u && (
        <div className="bullet" role="img" aria-label={`${v == null ? "kein Wert" : m.show(v)}, üblich ${m.show(u.lo)} bis ${m.show(u.hi)}`}>
          <i className="bullet-usual" style={{ left: `${pc(u.lo)}%`, width: `${Math.max(1, pc(u.hi) - pc(u.lo))}%` }} />
          {v != null && <i className="bullet-mark" style={{ left: `${pc(v)}%` }} />}
        </div>
      )}
      {vals.length > 1 && (
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="trend-spark" aria-hidden="true">
          {u && <rect x={0} y={y(u.hi)} width={W} height={Math.max(1, y(u.lo) - y(u.hi))} className="usual-band" />}
          <path d={line} className="spark-line" />
        </svg>
      )}
      <small>{text}{u && !off ? ` · üblich ${m.show(u.lo)}–${m.show(u.hi)}` : ""}</small>
    </div>
  );
}

const MAYBE: Record<string, string> = { alcohol: "Alkohol am Vorabend", training: "hartes Training am Vortag", short_sleep: "eine kurze Nacht" };

/// Frühzeichen (signal.rs): gemeinsame Abweichung der Nachtwerte über mehrere Nächte. Beschreibung, kein Befund.
function SignalCard({ s }: { s: Signal }) {
  const up = s.parts.filter((p) => p.z >= 1);
  const c = s.check, months = Math.round(c.years * 12);
  const W = 140, H = 28, vals = s.history.filter((v): v is number => v != null);
  const top = Math.max(2, ...vals), bot = Math.min(-1, ...vals), y = (v: number) => H - ((v - bot) / (top - bot)) * H;
  const show = (key: string, v: number) => METRIC.get(key)?.show(v) ?? fmt(v, 1);
  return (
    <article className={"day-group n-card signal" + (s.active ? " on" : "")}>
      <div className="n-eyebrow">Frühzeichen</div>
      {s.active ? (
        <>
          <b>Seit {s.nights} Nächten weichen deine Nachtwerte gemeinsam ab, wie oft vor einem Infekt.</b>
          <ul>
            {up.map((p) => <li key={p.key}>{METRIC.get(p.key)?.label ?? p.key} {show(p.key, p.value)} <span className="muted">(üblich {show(p.key, p.usual)})</span></li>)}
          </ul>
          {s.maybe.length > 0 && <small>Dasselbe Muster kann auch kommen von: {s.maybe.map((k) => MAYBE[k] ?? k).join(", ")}.</small>}
          <small>Welche Krankheit, lässt sich daraus nicht sagen; Erkältung, Grippe und Corona sehen in diesen Werten gleich aus. Kein Befund.</small>
        </>
      ) : (
        <small>Keine gemeinsamen Abweichungen von Puls, Atemfrequenz und HRV in den letzten Nächten.</small>
      )}
      {vals.length > 1 && (
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="trend-spark" role="img"
          aria-label="Gemeinsame Abweichung je Nacht, letzte 4 Wochen; die gestrichelte Linie ist die Schwelle">
          <line x1={0} x2={W} y1={y(s.threshold)} y2={y(s.threshold)} className="signal-line" />
          <path d={gapPath(s.history, (i) => (i / Math.max(1, s.history.length - 1)) * W, y)} className="spark-line" />
        </svg>
      )}
      <small className="muted">
        {c.episodes
          ? `Rückblick: Bei ${c.caught} von ${c.episodes} markierten Krank-Phasen kam der Hinweis bis 3 Tage vorher. ${c.unmarked} von ${c.runs} Hinweisen ohne markierte Krankheit.`
          : `Rückblick: ${c.runs} Hinweise in ${months} Monaten. Markiere Krank-Tage auf der Seite Tage, dann zeigt Helio, wie oft der Hinweis vorher kam.`}
      </small>
    </article>
  );
}

function DebtCard({ d }: { d: Debt }) {
  const more = d.minutes < 0;
  return (
    <article className="day-group n-card debt" data-g="sleep">
      <div className="n-eyebrow">Schlafschuld</div>
      <b>{Math.abs(d.minutes) < 15 ? "ausgeglichen" : `${dur(Math.abs(d.minutes))} ${more ? "mehr" : "weniger"} geschlafen als üblich`}</b>
      <small>
        Üblich sind {dur(d.need)} je Nacht (dein Median der 60 Nächte davor).{" "}
        {d.half_life != null
          ? `Ältere Nächte zählen weniger (Halbwertszeit ${fmt(d.half_life)} Tage, aus deinem Verlauf: nach kurzen Nächten schläfst du länger, ρ = ${fmt(d.rho ?? 0, 2)}).`
          : `Summe der letzten 7 Nächte${d.nights < 7 ? ` (${d.nights} davon erfasst)` : ""}. Dass du kurze Nächte danach ausgleichst, zeigt dein Verlauf nicht deutlich${d.rho != null ? ` (ρ = ${fmt(d.rho, 2)})` : ""}.`}
      </small>
    </article>
  );
}

export function Today() {
  const [view, setView] = useState<TodayView | null>(null);
  const [error, setError] = useState("");
  const load = () =>
    invoke<TodayView>("today_view", { today: localDay() })
      .then((v) => { setView(v); setError(""); toNotch(v); })
      .catch((e) => setError(String(e)));
  useEffect(() => {
    void load();
    const un = listen<unknown>("health", (e) => { if (e.payload) void load(); });
    // Notch im Tray wieder eingeschaltet: „Heute“ gleich neu schicken
    const un2 = listen<boolean>("notch", (e) => { if (e.payload) void load(); });
    // Krank-Tage geändert (Seite Tage): Rückblick neu
    const un3 = listen("marks", () => void load());
    // Nach Mitternacht ist „heute“ ein anderer Tag
    const t = setInterval(() => void load(), 30 * 60_000);
    return () => { void un.then((f) => f()); void un2.then((f) => f()); void un3.then((f) => f()); clearInterval(t); };
  }, []);

  if (!view || (!view.items.length && !view.debt)) {
    return (
      <section className="days" aria-label="Heute">
        <div className="n-eyebrow">Heute</div>
        <p className="fine">{error || (view ? "Noch keine Werte. Sobald Apple-Health-Werte da sind (Seite Quellen), erscheinen sie hier." : "…")}</p>
      </section>
    );
  }
  const off = view.items.filter(isOff);
  // Abweichungen zuerst, sonst Reihenfolge wie in normal.rs
  const sorted = (keep: (i: Item) => boolean) => view.items.filter(keep).sort((a, b) => Number(!!b.status && b.status !== "üblich") - Number(!!a.status && a.status !== "üblich"));
  const night = sorted((i) => NIGHT_KEYS.has(i.key)), day = sorted((i) => !NIGHT_KEYS.has(i.key));

  return (
    <section className="days" aria-label="Heute">
      <p className="today-sum" role="status">
        {off.length
          ? <>Außerhalb deines Üblichen: {off.map((i, n) => <span key={i.key} data-g={METRIC.get(i.key)?.g}>{n ? ", " : ""}<b>{METRIC.get(i.key)?.label}</b> {word(i)}</span>)}.</>
          : "Alles im üblichen Bereich."}
      </p>
      {error && <p className="fine warn" role="alert">{error}</p>}
      {/* aktives Frühzeichen gleich oben, sonst bei den Nachtwerten */}
      {view.signal?.active && <SignalCard s={view.signal} />}
      {view.night_day && !!night.length && (
        <article className="day-group n-card">
          <div className="n-eyebrow">Nacht zum {shortDate(view.night_day)}</div>
          <div className="trend-tiles">{night.map((i) => <Tile key={i.key} it={i} />)}</div>
        </article>
      )}
      {view.signal && !view.signal.active && <SignalCard s={view.signal} />}
      {view.debt && <DebtCard d={view.debt} />}
      {view.day_day && !!day.length && (
        <article className="day-group n-card">
          <div className="n-eyebrow">Tag · {shortDate(view.day_day)}</div>
          <div className="trend-tiles">{day.map((i) => <Tile key={i.key} it={i} />)}</div>
        </article>
      )}
      <details className="more n-card">
        <summary>Was heißt „üblich“?</summary>
        <p className="fine">
          Üblich ist der Bereich, in dem 8 von 10 deiner Tage aus den 60 Tagen davor lagen (dunkler Abschnitt im Balken, helles Band im Verlauf
          der letzten 4 Wochen). „Höher“ oder „niedriger“ heißt: außerhalb davon. Ab 14 Tagen mit Werten gibt es einen Bereich. Der heutige Tag
          läuft noch, darum stehen Bewegung und Essen vom letzten vollständigen Tag. Das ist eine Beschreibung deiner Werte, keine Bewertung und
          kein medizinischer Befund.
        </p>
      </details>
    </section>
  );
}

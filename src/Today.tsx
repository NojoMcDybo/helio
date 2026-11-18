import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { METRIC, clock, dur, fmt } from "./metrics";
import { localDay } from "./chart";
import { ArcGauge, BarSpark, DivBar, Dm, LineSpark, RangeRail, SleepRing, StageBar, StatusDots } from "./viz";

// Seite „Heute“: letzte Nacht und letzter vollständiger Tag gegen den eigenen üblichen Bereich (normal.rs).
// Kein Gesamtwert, nur einzelne Abweichungen mit Abstand zum Median. Beschreibung, keine Bewertung.
// Aufbau (Querformat): oben die Hauptanzeigen ohne Kartenfläche (Nacht auf dem 24-Stunden-Ring, daneben das Auffälligste),
// darunter je Bereich ein anderes Format. Die Lichter im Hintergrund (lights.ts) folgen data-light (Auffälligkeit).
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

/// Bei Uhrzeiten heißt „höher“ später.
const word = (it: Item) => {
  const clockKind = METRIC.get(it.key)?.kind === "clock";
  return it.status === "höher" ? (clockKind ? "später" : "höher") : it.status === "niedriger" ? (clockKind ? "früher" : "niedriger") : "";
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

const MAYBE: Record<string, string> = { alcohol: "Alkohol am Vorabend", training: "hartes Training am Vortag", short_sleep: "eine kurze Nacht" };
const HUE = { sleep: "#b79ad0", heart: "#dc308f", move: "#ee8a52", food: "#8fb59a", body: "#6f87ff" };
type G = keyof typeof HUE;
const hm = (m: number) => `${Math.floor(Math.round(m) / 60)}:${String(Math.round(m) % 60).padStart(2, "0")}`;
const clamp01 = (x: number) => Math.min(1, Math.max(0, x));
/// Auffälligkeit eines Werts für die Lichter: je weiter außerhalb des Üblichen, desto heller
const interest = (it?: Item) => (it ? clamp01(0.25 + reach(it) * 0.9) : 0.25);
const arrow = (it?: Item) => (it?.status === "höher" ? "↑" : it?.status === "niedriger" ? "↓" : "");
const bounds = (it: Item) => {
  const all = [...it.series.filter((x): x is number => x != null), ...(it.value != null ? [it.value] : []), ...(it.usual ? [it.usual.lo, it.usual.hi] : [])];
  const lo = Math.min(...all), hi = Math.max(...all), pad = (hi - lo) * 0.12 || 1;
  return [lo - pad, hi + pad] as const;
};

/// Zeile: Name, Wert und eine Skala (üblicher Bereich als Strecke, heute als Marke)
function Row({ it, label }: { it: Item; label?: string }) {
  const m = METRIC.get(it.key);
  if (!m || it.value == null) return null;
  const u = it.usual, [lo, hi] = bounds(it);
  return (
    <div className="mrow" title={`${m.label} ${m.show(it.value)}${u ? `, üblich ${m.show(u.lo)} bis ${m.show(u.hi)}` : ""}`}>
      <small>{label ?? m.label}</small>
      <b>{m.show(it.value)}<i className="ar">{arrow(it)}</i></b>
      <RangeRail min={lo} max={hi} value={it.value} lo={u?.lo} hi={u?.hi} label={`${m.label}: ${m.show(it.value)}`} />
    </div>
  );
}

function Eb({ g, children, right }: { g: G; children: string; right?: string }) {
  return <div className="eb" style={{ ["--c" as string]: HUE[g] }}><i />{children}{right && <em>{right}</em>}</div>;
}

function Stat({ it }: { it?: Item }) {
  const m = it && METRIC.get(it.key);
  if (!it || !m || it.value == null) return null;
  const [num, ...unit] = m.show(it.value).split(" ");
  return <div><small>{m.label}</small><b>{num}{unit.length > 0 && <span>{unit.join(" ")}</span>}<i className="ar">{arrow(it)}</i></b></div>;
}

/// Hauptanzeige links: die Nacht auf dem 24-Stunden-Ring, ohne Kartenfläche
function SleepHero({ by, view, off }: { by: (k: string) => Item | undefined; view: TodayView; off: Item[] }) {
  const sl = by("sleep_min"), on = by("onset"), wk = by("wake");
  if (!sl || sl.value == null) return null;
  const u = sl.usual, delta = u ? sl.value - u.median : 0;
  const said = !u ? "Noch kein üblicher Bereich" : isOff(sl) ? `${dur(Math.abs(delta))} ${delta < 0 ? "weniger" : "mehr"} als üblich` : "Im üblichen Bereich";
  return (
    <div className="hero-a" id="hSleep" data-light={Math.min(0.9, interest(sl))} data-lc={HUE.sleep}>
      <SleepRing onset={on?.value ?? null} wake={wk?.value ?? null} size={290} label={`Geschlafen ${dur(sl.value)}`}>
        <Dm text={hm(sl.value)} h={52} label={dur(sl.value)} />
        <div className="sub">Stunden</div>
      </SleepRing>
      <div className="hero-text">
        <Eb g="sleep" right={on?.value != null && wk?.value != null ? `${clock(on.value)} – ${clock(wk.value)}` : undefined}>Schlaf</Eb>
        <div className="lead2">{said}{isOff(sl) && <i className="ar big">{arrow(sl)}</i>}</div>
        {view.night_day && <div className="sub gap">Nacht zum {new Date(`${view.night_day}T12:00:00`).toLocaleDateString("de-DE", { weekday: "short", day: "2-digit", month: "2-digit" })}</div>}
        <div className="stats"><Stat it={by("efficiency")} /><Stat it={by("deep_min")} /><Stat it={by("rem_min")} /></div>
        <StatusDots label="Eine Marke je Kennzahl, gefüllt = außerhalb des Üblichen" items={view.items.map(isOff)} />
        <div className="sub">{view.items.length - off.length} von {view.items.length} Werten im üblichen Bereich</div>
      </div>
    </div>
  );
}

/// Hauptanzeige rechts: Frühzeichen, wenn aktiv; sonst der auffälligste Wert als großer Bogen
function FocusHero({ by, view, off }: { by: (k: string) => Item | undefined; view: TodayView; off: Item[] }) {
  const s = view.signal;
  if (s?.active) {
    const c = s.check, months = Math.round(c.years * 12);
    return (
      <div className="hero-b" id="hSig" data-light="0.95" data-lc={HUE.heart}>
        <Eb g="heart" right={`seit ${s.nights} Nächten`}>Frühzeichen</Eb>
        <div className="sig-head"><Dm text={String(s.nights)} h={76} label={`${s.nights} Nächte`} />
          <div><div className="lead2">Auffällig wie vor einem Infekt</div><div className="sub">Nachtwerte weichen gemeinsam ab</div></div></div>
        <div className="mrows">{s.parts.map((p) => { const it = by(p.key); return it ? <Row key={p.key} it={it} /> : null; })}</div>
        <details className="hint-i">
          <summary>Was heißt das?</summary>
          <p>Seit {s.nights} Nächten weichen deine Nachtwerte gemeinsam ab, wie oft vor einem Infekt. Die Skalen zeigen den Abstand vom Üblichen.</p>
          {s.maybe.length > 0 && <p>Dasselbe Muster kann auch kommen von: {s.maybe.map((k) => MAYBE[k] ?? k).join(", ")}.</p>}
          <p>Welche Krankheit, lässt sich daraus nicht sagen; Erkältung, Grippe und Corona sehen in diesen Werten gleich aus. Kein Befund.</p>
          <p>{c.episodes ? `Rückblick: Bei ${c.caught} von ${c.episodes} markierten Krank-Phasen kam der Hinweis bis 3 Tage vorher. ${c.unmarked} von ${c.runs} Hinweisen ohne markierte Krankheit.` : `Rückblick: ${c.runs} Hinweise in ${months} Monaten. Markiere Krank-Tage auf der Seite Tage, dann zeigt Helio, wie oft der Hinweis vorher kam.`}</p>
        </details>
      </div>
    );
  }
  const it = [...off].sort((a, b) => reach(b) - reach(a))[0] ?? by("rhr") ?? view.items.find((i) => i.value != null);
  const m = it && METRIC.get(it.key);
  if (!it || !m || it.value == null) return null;
  const u = it.usual, [lo, hi] = bounds(it), [num, ...unit] = m.show(it.value).split(" "), g = (m.g in HUE ? m.g : "heart") as G;
  return (
    <div className="hero-b" id="hSig" data-light={interest(it)} data-lc={HUE[g]}>
      <Eb g={g}>{m.label}</Eb>
      <div className="focus">
        <ArcGauge label={m.label} min={lo} max={hi} value={it.value} lo={u?.lo} hi={u?.hi} />
        <div className="focus-num"><Dm text={num} h={58} label={m.show(it.value)} /><span className="u">{unit.join(" ")}<i className="ar">{arrow(it)}</i></span></div>
      </div>
      <div className="sub">{isOff(it) && u ? `${word(it)} als üblich · ${distance(it, it.value - u.median)} ${it.value > u.median ? "über" : "unter"} dem Median` : "Im üblichen Bereich"}</div>
    </div>
  );
}

function Debt({ d }: { d: Debt }) {
  const surplus = -d.minutes, flat = Math.abs(surplus) < 15;
  const said = flat ? "Schlaf ausgeglichen" : `${dur(Math.abs(d.minutes))} ${surplus > 0 ? "mehr" : "weniger"} geschlafen als üblich`;
  return (
    <section className="s6 sec">
      <Eb g="sleep" right={`${d.nights} Nächte`}>Schlafschuld</Eb>
      <div className="debt2"><Dm text={flat ? "0:00" : hm(Math.abs(surplus))} h={30} label={said} /><DivBar value={surplus} max={240} label={said} /></div>
      <details className="hint-i">
        <summary>Was heißt das?</summary>
        <p>
          Üblich sind {dur(d.need)} je Nacht (dein Median der 60 Nächte davor). Rechts: mehr geschlafen, links: weniger.{" "}
          {d.half_life != null
            ? `Ältere Nächte zählen weniger (Halbwertszeit ${fmt(d.half_life)} Tage, aus deinem Verlauf: nach kurzen Nächten schläfst du länger, ρ = ${fmt(d.rho ?? 0, 2)}).`
            : `Summe der letzten 7 Nächte${d.nights < 7 ? ` (${d.nights} davon erfasst)` : ""}. Dass du kurze Nächte danach ausgleichst, zeigt dein Verlauf nicht deutlich${d.rho != null ? ` (ρ = ${fmt(d.rho, 2)})` : ""}.`}
        </p>
      </details>
    </section>
  );
}

const USED = new Set(["sleep_min", "onset", "wake", "efficiency", "deep_min", "rem_min", "hr_night", "rhr", "hrv", "resp", "spo2", "steps", "active_kcal", "workout_min", "energy", "protein", "weight"]);

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
  const by = (k: string) => view.items.find((i) => i.key === k);
  const sl = by("sleep_min"), deep = by("deep_min"), rem = by("rem_min"), rhr = by("rhr"), steps = by("steps"), weight = by("weight"), energy = by("energy"), protein = by("protein");
  const core = sl?.value != null ? Math.max(0, sl.value - (deep?.value ?? 0) - (rem?.value ?? 0)) : 0;
  const rest = view.items.filter((i) => !USED.has(i.key) && METRIC.has(i.key) && i.value != null);
  const kcal = by("active_kcal")?.value, train = by("workout_min")?.value;

  return (
    <section className="today" aria-label="Heute">
      {error && <p className="fine warn" role="alert">{error}</p>}
      <div className="hero2">
        <SleepHero by={by} view={view} off={off} />
        <FocusHero by={by} view={view} off={off} />
      </div>

      <div className="sections">
        {sl?.value != null && (
          <section className="s7">
            <Eb g="sleep" right="Phasen">Die Nacht</Eb>
            <StageBar deep={deep?.value ?? 0} core={core} rem={rem?.value ?? 0} awake={0} label="Tiefschlaf, Kernschlaf und REM der Nacht" />
            <div className="stats pad-t">
              {deep?.value != null && <div><small>Tief</small><b>{dur(deep.value)}</b></div>}
              <div><small>Kern</small><b>{dur(core)}</b></div>
              {rem?.value != null && <div><small>REM</small><b>{dur(rem.value)}</b></div>}
              <Stat it={by("hr_night")} />
            </div>
          </section>
        )}

        {rhr?.value != null && (
          <section className="s5">
            <Eb g="heart" right={arrow(rhr) ? `Ruhepuls ${arrow(rhr)}` : undefined}>Herz</Eb>
            <div className="heart-grid">
              <div className="lg gauge-card" data-g="heart" data-r="46" data-e="34" data-m="0.07" data-light={interest(rhr)} data-lc={HUE.heart}>
                <ArcGauge label="Ruhepuls" min={bounds(rhr)[0]} max={bounds(rhr)[1]} value={rhr.value} lo={rhr.usual?.lo} hi={rhr.usual?.hi} />
                <div className="gauge-num"><Dm text={String(Math.round(rhr.value))} h={28} label={`${Math.round(rhr.value)} bpm`} /><span className="u">bpm</span></div>
              </div>
              <div className="mrows">{["hrv", "resp", "spo2"].map((k) => { const it = by(k); return it ? <Row key={k} it={it} /> : null; })}</div>
            </div>
          </section>
        )}

        {steps?.value != null && (
          <section className="s7 sec" data-light={interest(steps) * 0.8} data-lc={HUE.move}>
            <Eb g="move" right="gestern">Bewegung</Eb>
            <div className="big-line">
              <div className="num"><Dm text={fmt(steps.value)} h={34} label={`${fmt(steps.value)} Schritte`} /><span className="u">Schritte</span></div>
              <span className="sub">{[arrow(steps) && `${arrow(steps)} ${word(steps)} als üblich`, kcal != null && `${fmt(kcal)} kcal aktiv`, train ? `Training ${dur(train)}` : ""].filter(Boolean).join(" · ")}</span>
            </div>
            <BarSpark label="Schritte der letzten Tage" series={steps.series} lo={steps.usual?.lo} hi={steps.usual?.hi} />
          </section>
        )}

        {(energy?.value != null || protein?.value != null) && (
          <section className="s5 sec">
            <Eb g="food" right="gestern">Ernährung</Eb>
            {energy?.value != null && <div className="num"><Dm text={fmt(energy.value)} h={30} label={`${fmt(energy.value)} kcal`} /><span className="u">kcal</span></div>}
            <div className="mrows">{energy && <Row it={energy} label="Energie" />}{protein && <Row it={protein} />}</div>
          </section>
        )}

        {weight?.value != null && (
          <section className="s6">
            <div className="lg card-w" data-g="body" data-r="52" data-e="40" data-m="0.07" data-light={interest(weight) * 0.6} data-lc={HUE.body}>
              <Eb g="body" right="28 Tage">Gewicht</Eb>
              <div className="big-line">
                <div className="num"><Dm text={fmt(weight.value, 1)} h={34} label={`${fmt(weight.value, 1)} kg`} /><span className="u">kg</span></div>
                <LineSpark series={weight.series} label="Gewicht der letzten 28 Tage" />
              </div>
            </div>
          </section>
        )}

        {view.debt && <Debt d={view.debt} />}

        {rest.length > 0 && (
          <section className="s12 sec">
            <Eb g="body">Weitere Werte</Eb>
            <div className="mrows wide">{rest.map((it) => <Row key={it.key} it={it} />)}</div>
          </section>
        )}

        <details className="more s12">
          <summary>Was heißt „üblich“?</summary>
          <p className="fine">
            Üblich ist der Bereich, in dem 8 von 10 deiner Tage aus den 60 Tagen davor lagen (hellere Strecke auf der Skala, Band im Verlauf).
            „Höher“ oder „niedriger“ heißt: außerhalb davon. Ab 14 Tagen mit Werten gibt es einen Bereich. Der heutige Tag läuft noch, darum
            stehen Bewegung und Essen vom letzten vollständigen Tag. Das ist eine Beschreibung deiner Werte, keine Bewertung und kein medizinischer Befund.
          </p>
        </details>
      </div>
    </section>
  );
}

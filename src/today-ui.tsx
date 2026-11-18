import { METRIC, clock, dur, fmt } from "./metrics";
import { ArcGauge, BandChart, Dm, RangeRail, SleepRing, StageBar, StatusDots, DivBar } from "./viz";
import { HUE, MAYBE, arrow, bounds, distance, hm, interest, isOff, reach, word, type Debt, type G, type Item, type TodayView } from "./today-data";
import { useSheet } from "./sheet";

// Bausteine für Heute, Schlaf und Körper. Zahlen als Punktmatrix (Dm), Werte gegen den eigenen üblichen Bereich (RangeRail).
type By = (k: string) => Item | undefined;

/// Zeile: Name, Wert und eine Skala (üblicher Bereich als Strecke, heute als Marke). Klick öffnet die Einzelansicht.
export function Row({ it, label }: { it: Item; label?: string }) {
  const open = useSheet(), m = METRIC.get(it.key);
  if (!m || it.value == null) return null;
  const u = it.usual, [lo, hi] = bounds(it);
  return (
    <div className="mrow tap" role="button" tabIndex={0} onClick={() => open(it.key)} onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); open(it.key); } }}
      title={`${m.label} ${m.show(it.value)}${u ? `, üblich ${m.show(u.lo)} bis ${m.show(u.hi)}` : ""}`}>
      <small>{label ?? m.label}</small>
      <b>{m.show(it.value)}<i className="ar">{arrow(it)}</i></b>
      <RangeRail min={lo} max={hi} value={it.value} lo={u?.lo} hi={u?.hi} label={`${m.label}: ${m.show(it.value)}`} />
    </div>
  );
}

export function Eb({ g, children, right, to }: { g: G; children: string; right?: string; to?: { label: string; go: () => void } }) {
  return (
    <div className="eb" style={{ ["--c" as string]: HUE[g] }}>
      <i />{children}
      {to ? <button className="eb-more" onClick={to.go}>{to.label} →</button> : right && <em>{right}</em>}
    </div>
  );
}

export function Stat({ it }: { it?: Item }) {
  const m = it && METRIC.get(it.key);
  if (!it || !m || it.value == null) return null;
  const [num, ...unit] = m.show(it.value).split(" ");
  return <div><small>{m.label}</small><b>{num}{unit.length > 0 && <span>{unit.join(" ")}</span>}<i className="ar">{arrow(it)}</i></b></div>;
}

/// Hauptanzeige links: die Nacht auf dem 24-Stunden-Ring, ohne Kartenfläche
export function SleepHero({ by, view, off }: { by: By; view: TodayView; off: Item[] }) {
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
export function FocusHero({ by, view, off }: { by: By; view: TodayView; off: Item[] }) {
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
          <p>{c.episodes ? `Rückblick: Bei ${c.caught} von ${c.episodes} markierten Krank-Phasen kam der Hinweis bis 3 Tage vorher. ${c.unmarked} von ${c.runs} Hinweisen ohne markierte Krankheit.` : `Rückblick: ${c.runs} Hinweise in ${months} Monaten. Markiere Krank-Tage unter Verlauf › Tage, dann zeigt Helio, wie oft der Hinweis vorher kam.`}</p>
        </details>
      </div>
    );
  }
  const it = focusItem(by, view, off);
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

/// Der Wert, den die rechte Hauptanzeige zeigt (ohne Frühzeichen): der auffälligste, sonst der Ruhepuls
export const focusItem = (by: By, view: TodayView, off: Item[]) =>
  [...off].sort((a, b) => reach(b) - reach(a))[0] ?? by("rhr") ?? view.items.find((i) => i.value != null);

/// Auffälliges als Karte: Wert, Abstand zum Median und der Verlauf der letzten 28 Tage mit dem üblichen Band
export function Story({ it }: { it: Item }) {
  const open = useSheet(), m = METRIC.get(it.key);
  if (!m || it.value == null) return null;
  const u = it.usual, [num, ...unit] = m.show(it.value).split(" "), g = (m.g in HUE ? m.g : "body") as G;
  return (
    <button className="lg story" data-g={g} data-r="40" data-e="30" data-m="0.05" data-light={interest(it) * 0.8} data-lc={HUE[g]} onClick={() => open(it.key)}>
      <Eb g={g}>{m.label}</Eb>
      <div className="story-top"><Dm text={num} h={36} label={m.show(it.value)} /><span className="u">{unit.join(" ")}<i className="ar">{arrow(it)}</i></span></div>
      <div className="sub">{u ? `${word(it)} als üblich · ${distance(it, it.value - u.median)} ${it.value > u.median ? "über" : "unter"} dem Median` : "Noch kein üblicher Bereich"}</div>
      <BandChart mini label={`${m.label}, letzte Tage`} series={it.series} lo={u?.lo} hi={u?.hi} />
    </button>
  );
}

/// Die Nacht in Phasen
export function Night({ by }: { by: By }) {
  const sl = by("sleep_min"), deep = by("deep_min"), rem = by("rem_min");
  if (sl?.value == null) return null;
  const core = Math.max(0, sl.value - (deep?.value ?? 0) - (rem?.value ?? 0));
  return (
    <section className="s12">
      <Eb g="sleep" right="Phasen">Die Nacht</Eb>
      <StageBar deep={deep?.value ?? 0} core={core} rem={rem?.value ?? 0} awake={0} label="Tiefschlaf, Kernschlaf und REM der Nacht" />
      <div className="stats pad-t">
        {deep?.value != null && <div><small>Tief</small><b>{dur(deep.value)}</b></div>}
        <div><small>Kern</small><b>{dur(core)}</b></div>
        {rem?.value != null && <div><small>REM</small><b>{dur(rem.value)}</b></div>}
        <Stat it={by("hr_night")} />
      </div>
    </section>
  );
}

export function DebtSection({ d }: { d: Debt }) {
  const surplus = -d.minutes, flat = Math.abs(surplus) < 15;
  const said = flat ? "Schlaf ausgeglichen" : `${dur(Math.abs(d.minutes))} ${surplus > 0 ? "mehr" : "weniger"} geschlafen als üblich`;
  return (
    <section className="s5">
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

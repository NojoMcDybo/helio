import { dur, fmt } from "./metrics";
import { BarSpark, Dm, LineSpark, ArcGauge } from "./viz";
import { HUE, arrow, bounds, interest, isOff, lookup, reach, useToday, word } from "./today-data";
import { Eb, FocusHero, Row, SleepHero, Story, focusItem } from "./today-ui";
import { Empty, PAGE, Skeleton, go } from "./chrome";

// Seite „Heute“ (Oura: Today = Top Stories): oben die zwei Hauptanzeigen ohne Kartenfläche (Nacht auf dem 24-Stunden-Ring, daneben das
// Auffälligste), dann „Auffällig“ als Karten, dann je Bereich ein Format mit Sprung in die Seite dahinter. Die Lichter im Hintergrund
// (lights.ts) folgen data-light (Auffälligkeit). Nacht, Schlafschuld und Rhythmus liegen auf Schlaf, alle Kennzahlen auf Körper.
export function Today() {
  const { view, error } = useToday();
  if (!view || (!view.items.length && !view.debt)) {
    if (!view && !error) return <Skeleton />;
    return <Empty title={error ? "Werte konnten nicht geladen werden" : "Noch keine Werte"} text={error || "Sobald Apple-Health-Werte da sind, erscheinen sie hier."} retry={!!error} to={error ? undefined : { label: "Zu den Quellen", page: PAGE.quellen }} />;
  }
  const by = lookup(view), off = view.items.filter(isOff);
  const rhr = by("rhr"), steps = by("steps"), weight = by("weight"), energy = by("energy"), protein = by("protein");
  const kcal = by("active_kcal")?.value, train = by("workout_min")?.value;
  // Nicht doppelt zeigen, was eine Hauptanzeige schon trägt
  const shown = new Set(view.signal?.active ? view.signal.parts.map((p) => p.key) : [focusItem(by, view, off)?.key]);
  const stories = [...off].filter((i) => !shown.has(i.key)).sort((a, b) => reach(b) - reach(a)).slice(0, 3);

  return (
    <section className="today" aria-label="Heute">
      <div className="hero2">
        <SleepHero by={by} view={view} off={off} />
        <FocusHero by={by} view={view} off={off} />
      </div>

      <div className="sections">
        {stories.length > 0 && (
          <section className="s12">
            <Eb g="heart" to={{ label: "Alle Werte", go: () => go(PAGE.koerper) }}>Auffällig</Eb>
            <div className="stories">{stories.map((it) => <Story key={it.key} it={it} />)}</div>
          </section>
        )}

        {rhr?.value != null && (
          <section className="s5">
            <Eb g="heart" to={{ label: "Körper", go: () => go(PAGE.koerper) }}>Herz</Eb>
            <div className="heart-grid">
              <div className="lg gauge-card" data-g="heart" data-r="46" data-e="34" data-m="0.07" data-light={interest(rhr)} data-lc={HUE.heart}>
                <ArcGauge label="Ruhepuls" min={bounds(rhr)[0]} max={bounds(rhr)[1]} value={rhr.value} lo={rhr.usual?.lo} hi={rhr.usual?.hi} />
                <div className="gauge-num"><Dm text={String(Math.round(rhr.value))} h={28} label={`${Math.round(rhr.value)} bpm`} /><span className="u">bpm{arrow(rhr) && <i className="ar">{arrow(rhr)}</i>}</span></div>
              </div>
              <div className="mrows">{["hrv", "resp", "spo2"].map((k) => { const it = by(k); return it ? <Row key={k} it={it} /> : null; })}</div>
            </div>
          </section>
        )}

        {steps?.value != null && (
          <section className="s7 sec" data-light={interest(steps) * 0.8} data-lc={HUE.move}>
            <Eb g="move" to={{ label: "Training", go: () => go(PAGE.training) }}>Bewegung</Eb>
            <div className="big-line">
              <div className="num"><Dm text={fmt(steps.value)} h={34} label={`${fmt(steps.value)} Schritte`} /><span className="u">Schritte · gestern</span></div>
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
          <section className="s7">
            <div className="lg card-w" data-g="body" data-r="52" data-e="40" data-m="0.07" data-light={interest(weight) * 0.6} data-lc={HUE.body}>
              <Eb g="body" right="28 Tage">Gewicht</Eb>
              <div className="big-line">
                <div className="num"><Dm text={fmt(weight.value, 1)} h={34} label={`${fmt(weight.value, 1)} kg`} /><span className="u">kg</span></div>
                <LineSpark series={weight.series} label="Gewicht der letzten 28 Tage" />
              </div>
            </div>
          </section>
        )}
      </div>
    </section>
  );
}


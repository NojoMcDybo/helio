import { dur, fmt } from "./metrics";
import { BarSpark, Dm } from "./viz";
import { isOff, lookup, useToday } from "./today-data";
import { DebtSection, Eb, Night, Row } from "./today-ui";
import { Empty, PAGE, Skeleton } from "./chrome";
import { Rhythm } from "./Rhythm";

const NIGHT = ["sleep_min", "onset", "wake", "efficiency", "deep_min", "rem_min", "hr_night"];

/// Befund der Nacht in einem Satz (Kopfzeile)
export function sleepSub(view: ReturnType<typeof useToday>["view"]) {
  const sl = lookup(view)("sleep_min");
  if (!sl || sl.value == null) return "Nächte, Schlafschuld und Tagesrhythmus";
  const u = sl.usual, d = u ? sl.value - u.median : 0;
  return `Letzte Nacht ${dur(sl.value)}${u && isOff(sl) ? ` · ${dur(Math.abs(d))} ${d < 0 ? "unter" : "über"} üblich` : u ? " · im üblichen Bereich" : ""}`;
}

// Seite „Schlaf“: die Nacht in Phasen und gegen das Übliche, die Schlafschuld, darunter der Rhythmus der letzten Wochen (Rhythm.tsx).
export function Sleep() {
  const { view, error } = useToday();
  if (!view) return error ? <Empty title="Werte konnten nicht geladen werden" text={error} retry /> : <Skeleton />;
  const by = lookup(view), sl = by("sleep_min"), rows = NIGHT.map(by).filter((i) => i?.value != null);
  if (!rows.length && !view.debt) return <Empty title="Noch keine Nächte" text="Sobald Schlafdaten da sind, erscheinen sie hier." to={{ label: "Zu den Quellen", page: PAGE.quellen }} />;
  return (
    <>
      <div className="sections first">
        {sl?.value != null && (
          <section className="s7" data-light="0.5" data-lc="#b79ad0">
            <Eb g="sleep" right="28 Nächte">Schlafdauer</Eb>
            <div className="big-line"><div className="num"><Dm text={fmt((sl.value) / 60, 1)} h={44} label={dur(sl.value)} /><span className="u">Stunden letzte Nacht</span></div></div>
            <BarSpark label="Schlafdauer der letzten Nächte" series={sl.series} lo={sl.usual?.lo} hi={sl.usual?.hi} />
          </section>
        )}
        {view.debt && <DebtSection d={view.debt} />}
        <Night by={by} />
        {rows.length > 0 && (
          <section className="s12 sec">
            <Eb g="sleep">Die Nacht gegen dein Übliches</Eb>
            <div className="mrows wide">{rows.map((it) => <Row key={it!.key} it={it!} />)}</div>
          </section>
        )}
      </div>
      <div className="chapter">Rhythmus</div>
      <Rhythm />
    </>
  );
}

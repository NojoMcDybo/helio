import { GROUPS, METRIC } from "./metrics";
import { arrow, isOff, useToday, type G, type Item } from "./today-data";
import { Eb } from "./today-ui";
import { BandChart } from "./viz";
import { Empty, PAGE, Skeleton } from "./chrome";
import { useSheet } from "./sheet";

/// Befund in einem Satz (Kopfzeile)
export function bodySub(items: Item[] | undefined) {
  const n = items?.filter((i) => i.value != null).length ?? 0;
  if (!n) return "Alle Kennzahlen im Vergleich mit deinem Üblichen";
  const off = items!.filter(isOff).length;
  return `${n} Kennzahlen · ${off ? `${off} außerhalb des Üblichen` : "alle im üblichen Bereich"}`;
}

function Tile({ it }: { it: Item }) {
  const open = useSheet(), m = METRIC.get(it.key);
  if (!m || it.value == null) return null;
  const [num, ...unit] = m.show(it.value).split(" "), u = it.usual;
  return (
    <button className={"tile" + (isOff(it) ? " off" : "")} onClick={() => open(it.key)} title={u ? `üblich ${m.show(u.lo)} bis ${m.show(u.hi)}` : undefined}>
      <small>{m.label}</small>
      <b>{num}{unit.length > 0 && <span>{unit.join(" ")}</span>}<i className="ar">{arrow(it)}</i></b>
      <BandChart mini label={`${m.label}, letzte Tage`} series={it.series} lo={u?.lo} hi={u?.hi} />
    </button>
  );
}

// Seite „Körper“ (Oura: Vitals): jede Kennzahl als kleine Kachel mit Verlauf und üblichem Band, gleiche Skala-Logik überall
// (Tufte: Small Multiples). Antippen öffnet die Einzelansicht. Gruppiert nach Schlaf, Herz, Bewegung, Ernährung, Körper.
export function Body() {
  const { view, error } = useToday();
  if (!view) return error ? <Empty title="Werte konnten nicht geladen werden" text={error} retry /> : <Skeleton />;
  const groups = GROUPS.map((g) => ({ g, items: view.items.filter((i) => i.value != null && METRIC.get(i.key)?.g === g.g) })).filter((x) => x.items.length);
  if (!groups.length) return <Empty title="Noch keine Werte" text="Sobald Apple-Health-Werte da sind, erscheinen sie hier." to={{ label: "Zu den Quellen", page: PAGE.quellen }} />;
  return (
    <div className="body-groups">
      {groups.map(({ g, items }) => (
        <section key={g.g}>
          <Eb g={g.g as G}>{g.title}</Eb>
          <div className="tiles">{items.map((it) => <Tile key={it.key} it={it} />)}</div>
        </section>
      ))}
      <details className="more">
        <summary>Was heißt „üblich“?</summary>
        <p className="fine">
          Üblich ist der Bereich, in dem 8 von 10 deiner Tage aus den 60 Tagen davor lagen (hellere Strecke auf der Skala, Band im Verlauf).
          „Höher“ oder „niedriger“ heißt: außerhalb davon. Ab 14 Tagen mit Werten gibt es einen Bereich. Der heutige Tag läuft noch, darum
          stehen Bewegung und Essen vom letzten vollständigen Tag. Das ist eine Beschreibung deiner Werte, keine Bewertung und kein medizinischer Befund.
        </p>
      </details>
    </div>
  );
}


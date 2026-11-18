import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { GROUPS, QUALITY } from "./metrics";
import { ClockDial, StageBar } from "./viz";

// Seite „Tage“: die Tagesmatrix eines Tages (daily.rs), gruppiert, mit Quelle und Datenqualität.
// Nur beschreibend: Werte so, wie sie aus Health kommen, ohne Bewertung.
type Metric = { key: string; value: number; src: string | null; quality: number };

const addDays = (day: string, n: number) => {
  const d = new Date(`${day}T12:00:00Z`);
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
};
const longDate = (day: string) =>
  new Date(`${day}T12:00:00`).toLocaleDateString("de-DE", { weekday: "short", day: "2-digit", month: "2-digit", year: "numeric" });

/// Bei Schlaf stehen die Zahlen hinter „Werte“: das Bild zeigt das Wichtige.
const Wrap = ({ fold, children }: { fold: boolean; children: React.ReactNode }) => (fold ? <details className="hint-i"><summary>Werte</summary>{children}</details> : <>{children}</>);

/// Schlaf als Bild: Fenster auf dem 24-h-Ring (Einschlafen = Marke, Aufwachen = Ring-Marke) und die Phasen als Balken.
function SleepViz({ get }: { get: (k: string) => number | undefined }) {
  const onset = get("onset"), wake = get("wake"), deep = get("deep_min") ?? 0, core = get("core_min") ?? 0, rem = get("rem_min") ?? 0, awake = get("awake_min") ?? 0;
  const stages = deep + core + rem + awake > 0;
  if (onset == null && wake == null && !stages) return null;
  return (
    <div className="sleep-viz">
      {onset != null && wake != null
        ? <ClockDial label="Schlaffenster auf dem 24-Stunden-Ring" spans={[{ from: onset, to: wake }]} dots={[{ v: onset }, { v: wake, kind: "alt" }]} />
        : <span />}
      {stages && (
        <StageBar deep={deep} core={core} rem={rem} awake={awake}
          label={`Schlafphasen: tief ${Math.round(deep)} min, Kern ${Math.round(core)} min, REM ${Math.round(rem)} min, wach ${Math.round(awake)} min`} />
      )}
    </div>
  );
}

export function Days() {
  const [range, setRange] = useState<[string, string] | null>(null);
  const [day, setDay] = useState<string | null>(null);
  const [rows, setRows] = useState<Metric[] | null>(null);
  const [error, setError] = useState("");
  const [sick, setSick] = useState<Set<string>>(new Set());
  const loadMarks = () => invoke<string[]>("marks").then((m) => setSick(new Set(m))).catch((e) => setError(String(e)));
  const toggleSick = (d: string) => invoke("mark_sick", { day: d, sick: !sick.has(d) }).then(loadMarks).catch((e) => setError(String(e)));

  const loadRange = async () => {
    try {
      const r = await invoke<[string, string] | null>("daily_range");
      setRange(r);
      setDay((d) => d ?? r?.[1] ?? null);
    } catch (e) {
      setError(String(e));
    }
  };
  useEffect(() => {
    void loadRange();
    void loadMarks();
    // Nach jedem Abgleich (Ergebnis ≠ null) können sich Tage geändert haben
    const un = listen<unknown>("health", (e) => { if (e.payload) { void loadRange(); setRows(null); } });
    return () => void un.then((f) => f());
  }, []);
  useEffect(() => {
    if (!day) return;
    let live = true;
    invoke<Metric[]>("daily_day", { day })
      .then((m) => { if (live) { setRows(m); setError(""); } })
      .catch((e) => live && setError(String(e)));
    return () => { live = false; };
  }, [day, range]);

  if (!range || !day) {
    return (
      <section className="days" aria-label="Tage">
        <div className="n-eyebrow">Tage</div>
        <p className="fine">{error || "Noch keine Tage berechnet. Sobald Apple-Health-Werte da sind (Seite Quellen), erscheinen sie hier."}</p>
      </section>
    );
  }

  const byKey = new Map((rows ?? []).map((m) => [m.key, m]));
  const groups = GROUPS.map((g) => ({ ...g, items: g.items.filter(([k]) => byKey.has(k)) })).filter((g) => g.items.length);

  return (
    <section className="days" aria-label="Tage">
      <div className="day-nav">
        <button className="n-btn" aria-label="Tag davor" disabled={day <= range[0]} onClick={() => setDay(addDays(day, -1))}>‹</button>
        <label>
          <span className="sr-only">Tag wählen</span>
          <b aria-hidden="true">{longDate(day)}</b>
          <input type="date" value={day} min={range[0]} max={range[1]} onChange={(e) => e.target.value && setDay(e.target.value)} />
        </label>
        <button className="n-btn" aria-label="Tag danach" disabled={day >= range[1]} onClick={() => setDay(addDays(day, 1))}>›</button>
      </div>

      <button className={"n-btn mark-sick" + (sick.has(day) ? " on" : "")} aria-pressed={sick.has(day)} onClick={() => void toggleSick(day)}>
        {sick.has(day) ? "Krank an diesem Tag ✓" : "Krank an diesem Tag"}
      </button>
      {error && <p className="fine warn" role="alert">{error}</p>}
      {rows && !groups.length && <p className="fine">Für diesen Tag liegen keine Werte vor.</p>}

      {groups.map((g) => (
        <article key={g.title} className="day-group n-card lg" data-g={g.g}>
          <div className="n-eyebrow">{g.title}</div>
          {g.g === "sleep" && <SleepViz get={(k) => byKey.get(k)?.value} />}
          <Wrap fold={g.g === "sleep"}><table>
            <tbody>
              {g.items.map(([k, label, show]) => {
                const m = byKey.get(k)!;
                return (
                  <tr key={k} className={m.quality < 2 ? "weak" : ""}>
                    <td>{label}{m.src && <small>{m.src}</small>}</td>
                    <td className="n-num">{show(m.value)}</td>
                    <td><i className={`q q${m.quality}`} title={QUALITY[m.quality]} /><span className="sr-only">{QUALITY[m.quality]}</span></td>
                  </tr>
                );
              })}
            </tbody>
          </table></Wrap>
        </article>
      ))}
      <details className="hint-i">
        <summary>?</summary>
        <p>
          Je Kennzahl zählt eine Quelle; Schritte, Strecke, Energie und Essen werden je Stunde aus der Quelle mit dem größten Wert genommen, damit
          nichts doppelt zählt. Der Punkt zeigt, wie vollständig die Daten des Tages sind. Die Nacht gehört zum Tag, an dem du aufwachst.
          „Krank an diesem Tag“ bleibt nur in Helio und dient dem Rückblick der Frühzeichen auf der Seite Heute.
        </p>
      </details>
    </section>
  );
}

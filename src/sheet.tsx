import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { METRIC, QUALITY, fmt } from "./metrics";
import { BandChart, Dm } from "./viz";
import { HUE, arrow, distance, isOff, useToday, word, type G } from "./today-data";

// Einzelansicht einer Kennzahl (Oura/Whoop: Antippen führt vom Überblick in den Verlauf). Ein Klick auf eine Zeile oder Karte
// öffnet sie als Glasblatt rechts; Esc, Klick daneben oder ✕ schließen. Daten kommen aus dem gemeinsamen Speicher (today-data.ts).
const Ctx = createContext<(k: string | null) => void>(() => {});
export const useSheet = () => useContext(Ctx);

function Sheet({ k, onClose }: { k: string; onClose: () => void }) {
  const { view } = useToday(), close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    close.current?.focus();
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } };
    addEventListener("keydown", key, true);
    return () => { removeEventListener("keydown", key, true); prev?.focus?.(); };
  }, []);
  const it = view?.items.find((i) => i.key === k), m = METRIC.get(k);
  if (!it || !m || it.value == null) return null;
  const u = it.usual, g = (m.g in HUE ? m.g : "body") as G, vals = it.series.filter((v): v is number => v != null);
  const [num, ...unit] = m.show(it.value).split(" ");
  const said = !u ? "Noch kein üblicher Bereich (ab 14 Tagen mit Werten)" : isOff(it) ? `${word(it)} als üblich · ${distance(it, it.value - u.median)} ${it.value > u.median ? "über" : "unter"} dem Median` : "Im üblichen Bereich";
  const cell = (label: string, v: string) => <div><small>{label}</small><b>{v}</b></div>;
  return (
    <div className="sheet-wrap" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <aside className="lg sheet" data-g={g} data-r="44" data-e="34" data-m="0.04" role="dialog" aria-modal="true" aria-label={m.label}>
        <header>
          <div className="eb" style={{ ["--c" as string]: HUE[g] }}><i />{m.label}<em>{new Date(`${it.day}T12:00:00`).toLocaleDateString("de-DE", { weekday: "short", day: "2-digit", month: "2-digit" })}</em></div>
          <button ref={close} className="x" onClick={onClose} aria-label="Schließen">✕</button>
        </header>
        <div className="sheet-num"><Dm text={num} h={64} label={m.show(it.value)} /><span className="u">{unit.join(" ")}<i className="ar">{arrow(it)}</i></span></div>
        <p className="sheet-said">{said}</p>
        <div className="sheet-chart">
          <BandChart series={it.series} lo={u?.lo} hi={u?.hi} median={u?.median} label={`${m.label}, letzte ${it.series.length} Tage mit üblichem Bereich`} />
          <div className="axis"><span>vor {it.series.length - 1} Tagen</span><span>heute</span></div>
        </div>
        <div className="sheet-grid">
          {u && cell("Median", m.show(u.median))}
          {u && cell("Üblich", `${m.show(u.lo)} – ${m.show(u.hi)}`)}
          {vals.length > 1 && cell("Tiefster", m.show(Math.min(...vals)))}
          {vals.length > 1 && cell("Höchster", m.show(Math.max(...vals)))}
          {cell("Tage mit Wert", `${vals.length} von ${it.series.length}`)}
          {cell("Datenmenge", QUALITY[it.quality] ?? "–")}
        </div>
        <p className="fine">
          Das Band ist der übliche Bereich: 8 von 10 deiner Tage aus den 60 Tagen davor{u ? ` (${fmt(u.n)} Tage)` : ""}. Die Marke ist der letzte Wert{it.src ? `, gemessen von ${it.src}` : ""}.
          Beschreibung deiner Werte, keine Bewertung.
        </p>
      </aside>
    </div>
  );
}

export function SheetHost({ children }: { children: ReactNode }) {
  const [key, setKey] = useState<string | null>(null);
  return (
    <Ctx.Provider value={setKey}>
      {children}
      {key && <Sheet key={key} k={key} onClose={() => setKey(null)} />}
    </Ctx.Provider>
  );
}

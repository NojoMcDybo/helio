import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { glassLight, lightScroller, liquid, segments, windowControls } from "./nojo/nojo-ui";
import { Pager } from "./Pager";
import { Pulse } from "./Pulse";
import { Rhythm } from "./Rhythm";
import { Ask } from "./Ask";
import { Days } from "./Days";
import { Load } from "./Load";
import { Longterm } from "./Longterm";
import { Sources } from "./Sources";
import { Today } from "./Today";
import { UpdateBanner } from "./Update";

const LABELS = ["Heute", "Belastung", "Rhythmus", "Langzeit", "Tage", "Puls", "Fragen", "Quellen"];

// Reiter als Segment der gemeinsamen Designsprache; Strg+Pfeiltasten blättern (wie in Haze).
function Tabs({ page, onPage }: { page: number; onPage: (p: number) => void }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.altKey || e.metaKey) return;
      if (e.key === "ArrowRight" && page < LABELS.length - 1) { e.preventDefault(); onPage(page + 1); }
      if (e.key === "ArrowLeft" && page > 0) { e.preventDefault(); onPage(page - 1); }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [page]);
  // bei schmalem Fenster scrollt die Leiste: gewählten Reiter sichtbar halten
  // Klammern nötig: scrollIntoView gibt in neuem Chromium ein Promise zurück, React hielte es für die Aufräumfunktion
  useEffect(() => {
    void document.querySelector(".page-tabs .active")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [page]);
  return (
    <nav className="n-seg page-tabs" role="tablist" aria-label="Ansicht">
      {LABELS.map((l, i) => (
        <button key={l} role="tab" aria-selected={page === i} aria-controls={`page-${i}`} className={page === i ? "active" : ""} onClick={() => onPage(i)}>
          {l}
        </button>
      ))}
    </nav>
  );
}

function App() {
  const [page, setPage] = useState(0);
  return (
    <>
      <UpdateBanner />
      <Tabs page={page} onPage={setPage} />
      <Pager page={page} onPage={setPage} labels={LABELS}>
        <Today />
        <Load />
        <Rhythm />
        <Longterm />
        <Days />
        <Pulse />
        <Ask />
        <Sources />
      </Pager>
    </>
  );
}

const win = getCurrentWindow();
// Fensterknöpfe als Glaspille (gemeinsame Designsprache); Schließen = in den Tray
windowControls({ minimize: () => void win.minimize(), close: () => void win.hide(), closeTitle: "In den Tray" });
glassLight();
liquid();
const main = document.getElementById("main")!;
createRoot(main).render(<App />);
segments();
// Lichtleiste statt Bildlaufleiste
lightScroller(main, { insetTop: 56 });

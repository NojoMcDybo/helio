import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { glassLight, lightScroller, liquid, segments, windowControls } from "./nojo/nojo-ui";
import { glass } from "./glass";
import { startLights } from "./lights";
import { Pulse } from "./Pulse";
import { Rhythm } from "./Rhythm";
import { Ask } from "./Ask";
import { Days } from "./Days";
import { Load } from "./Load";
import { Longterm } from "./Longterm";
import { Sources } from "./Sources";
import { Today } from "./Today";
import { UpdateBanner } from "./Update";
import { NOJO_ICONS } from "./nojo/nojo-icons";

const LABELS = ["Heute", "Belastung", "Rhythmus", "Langzeit", "Tage", "Puls", "Fragen", "Quellen"];

// Menü rechts, senkrecht, als Glasleiste; Strg+Pfeiltasten blättern (wie in Haze).
const RAIL_ICONS: Record<string, string> = {
  Heute: '<circle cx="12" cy="12" r="4"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6 7 7M17 17l1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4"/>',
  Belastung: '<path d="M5 20V12M10 20V6M15 20v-9M20 20V9"/>',
  Rhythmus: '<path d="M2 12c3-8 5-8 8 0s5 8 8 0 3-4 4-3"/>',
  Langzeit: '<path d="M3 18l5-6 4 3 8-10"/><path d="M3 21h18"/>',
  Tage: '<rect x="4" y="5" width="16" height="15" rx="3"/><path d="M4 10h16M9 3v4M15 3v4"/>',
  Puls: '<path d="M2 12h5l2-6 4 12 2-6h7"/>',
  Fragen: '<path d="M5 5h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-7l-5 4v-4H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z"/>',
  Quellen: '<path d="M12 4 3 8l9 4 9-4-9-4zM3 12l9 4 9-4M3 16l9 4 9-4"/>',
};

function Rail({ page, onPage }: { page: number; onPage: (p: number) => void }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.altKey || e.metaKey) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest("input,textarea,select,[contenteditable]")) return;
      if ((e.key === "ArrowRight" || e.key === "ArrowDown") && page < LABELS.length - 1) { e.preventDefault(); onPage(page + 1); }
      if ((e.key === "ArrowLeft" || e.key === "ArrowUp") && page > 0) { e.preventDefault(); onPage(page - 1); }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [page]);
  return (
    <nav className="rail lg" role="tablist" aria-label="Ansicht" aria-orientation="vertical" data-r="30" data-e="22" data-m="0">
      {LABELS.map((l, i) => (
        <button key={l} role="tab" aria-selected={page === i} aria-controls={`page-${i}`} className={page === i ? "on" : ""} onClick={() => onPage(i)} title={l}>
          <svg viewBox="0 0 24 24" aria-hidden="true" dangerouslySetInnerHTML={{ __html: RAIL_ICONS[l] }} />
          {l}
        </button>
      ))}
    </nav>
  );
}

function App() {
  const [page, setPage] = useState(0);
  const field = useRef<HTMLDivElement>(null), stage = useRef<HTMLDivElement>(null), scrolls = useRef<number[]>([]);
  const go = (p: number) => {
    scrolls.current[page] = main.scrollTop;
    setPage(p);
    requestAnimationFrame(() => main.scrollTo(0, scrolls.current[p] ?? 0));
  };
  useEffect(() => {
    const l = startLights(field.current!, stage.current!, main, () => stage.current!.querySelector<HTMLElement>(".page:not([hidden])"));
    return () => l.stop();
  }, []);
  const pages = [<Today />, <Load />, <Rhythm />, <Longterm />, <Days />, <Pulse />, <Ask />, <Sources />];
  return (
    <>
      <div className="stage" ref={stage}>
        <div className="field" ref={field} aria-hidden="true" />
        <UpdateBanner />
        {pages.map((p, i) => (
          <section key={LABELS[i]} className="page" role="tabpanel" id={`page-${i}`} aria-label={LABELS[i]} hidden={i !== page}>{p}</section>
        ))}
      </div>
      <Rail page={page} onPage={go} />
    </>
  );
}

const win = getCurrentWindow();
// Fensterknöpfe als Glaspille (gemeinsame Designsprache); Schließen = in den Tray
const wc = windowControls({ minimize: () => void win.minimize(), close: () => void win.hide(), closeTitle: "In den Tray" });
// Hell/Dunkel: Knopf vor den Fensterknöpfen, Wahl bleibt in localStorage
const root = document.documentElement;
const themeBtn = document.createElement("button");
themeBtn.className = "n-ico sm theme-btn";
const paintTheme = () => {
  const dark = root.dataset.theme !== "light", i = NOJO_ICONS[dark ? "sun" : "moon"];
  const attr = i.kind === "line" ? 'fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"' : 'fill="currentColor"';
  themeBtn.title = dark ? "Hell" : "Dunkel";
  themeBtn.setAttribute("aria-label", themeBtn.title);
  themeBtn.innerHTML = `<svg viewBox="0 0 24 24" ${attr} aria-hidden="true">${i.body}</svg>`;
};
themeBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  root.dataset.theme = root.dataset.theme === "light" ? "dark" : "light";
  try { localStorage.setItem("helio-theme", root.dataset.theme); } catch { /* ohne Speicher gilt die Wahl nur bis zum Neustart */ }
  paintTheme();
});
paintTheme();
wc.prepend(themeBtn);
glassLight();
liquid();
glass();
const main = document.getElementById("main")!;
createRoot(main).render(<App />);
segments();
// Lichtleiste statt Bildlaufleiste
lightScroller(main, { insetTop: 56 });

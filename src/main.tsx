import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { glassLight, lightScroller, liquid, segments, windowControls } from "./nojo/nojo-ui";
import { glass } from "./glass";
import { startLights } from "./lights";
import { Pulse } from "./Pulse";
import { Ask } from "./Ask";
import { Load } from "./Load";
import { Sources } from "./Sources";
import { Today } from "./Today";
import { Sleep, sleepSub } from "./Sleep";
import { Body, bodySub } from "./Body";
import { History } from "./History";
import { PageHead } from "./chrome";
import { SheetHost } from "./sheet";
import { isOff, startToday, useToday } from "./today-data";
import { UpdateBanner } from "./Update";
import { NOJO_ICONS } from "./nojo/nojo-icons";

// Seiten (Oura: wenige Ziele statt vieler Reiter): Heute, Schlaf, Körper, Training, Verlauf, Live; Fragen und Quellen als zweite Gruppe.
const LABELS = ["Heute", "Schlaf", "Körper", "Training", "Verlauf", "Live", "Fragen", "Quellen"];
const SECOND = 6; // ab hier die zweite Gruppe im Menü

// Menü rechts, senkrecht, als Glasleiste; Strg+Pfeiltasten blättern (wie in Haze), Strg+1 bis 8 springen.
const RAIL_ICONS: Record<string, string> = {
  Heute: '<circle cx="12" cy="12" r="4"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6 7 7M17 17l1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4"/>',
  Schlaf: '<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>',
  Körper: '<path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z"/>',
  Training: '<path d="M5 20V12M10 20V6M15 20v-9M20 20V9"/>',
  Verlauf: '<path d="M3 18l5-6 4 3 8-10"/><path d="M3 21h18"/>',
  Live: '<path d="M2 12h5l2-6 4 12 2-6h7"/>',
  Fragen: '<path d="M5 5h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-7l-5 4v-4H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z"/>',
  Quellen: '<path d="M12 4 3 8l9 4 9-4-9-4zM3 12l9 4 9-4M3 16l9 4 9-4"/>',
};

function Rail({ page, onPage }: { page: number; onPage: (p: number) => void }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.altKey || e.metaKey) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("input,textarea,select,[contenteditable]")) return;
      const n = Number(e.key);
      if (n >= 1 && n <= LABELS.length) { e.preventDefault(); onPage(n - 1); return; }
      if ((e.key === "ArrowRight" || e.key === "ArrowDown") && page < LABELS.length - 1) { e.preventDefault(); onPage(page + 1); }
      if ((e.key === "ArrowLeft" || e.key === "ArrowUp") && page > 0) { e.preventDefault(); onPage(page - 1); }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [page]);
  return (
    <nav className="rail lg" role="tablist" aria-label="Ansicht" aria-orientation="vertical" data-r="30" data-e="22" data-m="0">
      {LABELS.map((l, i) => (
        <span key={l} className="rail-i">
          {i === SECOND && <hr role="presentation" />}
          <button role="tab" aria-selected={page === i} aria-controls={`page-${i}`} className={page === i ? "on" : ""} onClick={() => onPage(i)} title={`${l} (Strg+${i + 1})`}>
            <svg viewBox="0 0 24 24" aria-hidden="true" dangerouslySetInnerHTML={{ __html: RAIL_ICONS[l] }} />
            {l}
          </button>
        </span>
      ))}
    </nav>
  );
}

const hhmm = (t: number) => new Date(t).toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" });

function App() {
  const [page, setPage] = useState(0);
  const { view, at } = useToday();
  const field = useRef<HTMLDivElement>(null), stage = useRef<HTMLDivElement>(null), scrolls = useRef<number[]>([]), cur = useRef(0);
  const go = (p: number) => {
    if (p === cur.current || p < 0 || p >= LABELS.length) return;
    scrolls.current[cur.current] = main.scrollTop;
    cur.current = p;
    setPage(p);
    requestAnimationFrame(() => main.scrollTo(0, scrolls.current[p] ?? 0));
  };
  useEffect(() => {
    startToday();
    const l = startLights(field.current!, stage.current!, main, () => stage.current!.querySelector<HTMLElement>(".page:not([hidden])"));
    const h = (e: Event) => go((e as CustomEvent<number>).detail);
    addEventListener("helio-go", h);
    return () => { l.stop(); removeEventListener("helio-go", h); };
  }, []);

  const off = view?.items.filter(isOff).length ?? 0;
  const today = view ? (view.signal?.active ? `Frühzeichen seit ${view.signal.nights} Nächten${off ? ` · ${off} Werte außerhalb des Üblichen` : ""}` : off ? `${off} ${off === 1 ? "Wert" : "Werte"} außerhalb des Üblichen` : "Alles im üblichen Bereich") : undefined;
  const heads: (React.ReactNode)[] = [
    <PageHead title="Heute" sub={today} right={at ? <span title="Letzter Abgleich">aktualisiert {hhmm(at)}</span> : undefined} />,
    <PageHead title="Schlaf" sub={sleepSub(view)} />,
    <PageHead title="Körper" sub={bodySub(view?.items)} />,
    <PageHead title="Training" sub="Belastung, Form und Erholung nach dem Training" />,
    <PageHead title="Verlauf" sub="Trends über Monate und einzelne Tage" />,
    null,
    null,
    <PageHead title="Quellen" sub="Woher die Werte kommen" />,
  ];
  const pages = [<Today />, <Sleep />, <Body />, <Load />, <History />, <Pulse />, <Ask />, <Sources />];
  return (
    <SheetHost>
      <div className="stage" ref={stage}>
        <div className="field" ref={field} aria-hidden="true" />
        <UpdateBanner />
        {pages.map((p, i) => (
          <section key={LABELS[i]} className="page" role="tabpanel" id={`page-${i}`} aria-label={LABELS[i]} hidden={i !== page}>{heads[i]}{p}</section>
        ))}
      </div>
      <Rail page={page} onPage={go} />
    </SheetHost>
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

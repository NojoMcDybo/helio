import type { ReactNode } from "react";
import { loadToday } from "./today-data";

// Gemeinsames Gerüst der Seiten: Kopfzeile mit Befund in einem Satz, Ladezustand in Form des Inhalts, leerer Zustand mit Ausweg.
export const PAGE = { heute: 0, schlaf: 1, koerper: 2, training: 3, verlauf: 4, live: 5, fragen: 6, quellen: 7 } as const;
/// Seitenwechsel von überall (main.tsx hört zu)
export const go = (p: number) => dispatchEvent(new CustomEvent("helio-go", { detail: p }));

export function PageHead({ title, sub, right }: { title: string; sub?: ReactNode; right?: ReactNode }) {
  return (
    <header className="phead">
      <div><h1>{title}</h1>{sub && <p>{sub}</p>}</div>
      {right && <div className="phead-r">{right}</div>}
    </header>
  );
}

/// Ladezustand: grobe Formen an den Stellen, an denen gleich Inhalt steht (kein Drehkreis)
export function Skeleton() {
  return (
    <div className="skel" aria-busy="true" aria-label="Lädt">
      <div className="skel-hero"><i className="ring" /><div><i /><i /><i className="s" /></div><div><i /><i /><i /><i className="s" /></div></div>
      <div className="skel-row"><i /><i /><i /></div>
    </div>
  );
}

export function Empty({ title, text, retry, to }: { title: string; text: string; retry?: boolean; to?: { label: string; page: number } }) {
  return (
    <div className="empty" role="status">
      <h2>{title}</h2>
      <p>{text}</p>
      <div className="empty-act">
        {retry && <button className="n-btn" onClick={() => void loadToday()}>Erneut versuchen</button>}
        {to && <button className="n-btn" onClick={() => go(to.page)}>{to.label}</button>}
      </div>
    </div>
  );
}

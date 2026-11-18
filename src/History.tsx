import { useState } from "react";
import { Longterm } from "./Longterm";
import { Days } from "./Days";

const TABS = ["Langzeit", "Tage"];

// Seite „Verlauf“: Langzeit (Trends über Monate) und Tage (ein Tag nach dem anderen) unter einem Dach; beide bleiben geladen.
export function History() {
  const [t, setT] = useState(0);
  return (
    <>
      <div className="seg-tabs" role="tablist" aria-label="Verlauf">
        {TABS.map((l, i) => (
          <button key={l} role="tab" aria-selected={t === i} className={t === i ? "on" : ""} onClick={() => setT(i)}>{l}</button>
        ))}
      </div>
      <div hidden={t !== 0}><Longterm /></div>
      <div hidden={t !== 1}><Days /></div>
    </>
  );
}

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Selbst-Update (update.rs): Zustand kommt als Ereignis "update". Banner oben bei neuer Version, Zeile auf der Seite Quellen.
type State = { status: "idle" | "checking" | "current" | "available" | "downloading" | "error"; current?: string; version?: string; message?: string };

function useUpdate() {
  const [s, setS] = useState<State | null>(null);
  useEffect(() => {
    void invoke<State>("update_state").then(setS).catch(() => {});
    const un = listen<State>("update", (e) => setS(e.payload));
    return () => void un.then((f) => f());
  }, []);
  const install = () => void invoke("update_install").catch(() => {}); // Fehler kommen als Zustand "error"
  const check = () => void invoke<State>("update_check").then(setS).catch(() => {});
  return { s, install, check };
}

export function UpdateBanner() {
  const { s, install } = useUpdate();
  if (s?.status !== "available" && s?.status !== "downloading") return null;
  const busy = s.status === "downloading";
  return (
    <div className="update-banner n-card lg" role="status">
      <span>{busy ? "Lädt … Helio startet gleich neu" : <>Helio <b>{s.version}</b> ist da <small>(installiert: {s.current})</small></>}</span>
      <button className="n-btn primary" disabled={busy} onClick={install}>{busy ? "…" : "Installieren"}</button>
    </div>
  );
}

export function UpdateRow() {
  const { s, install, check } = useUpdate();
  const st = s?.status ?? "idle", v = s?.current ?? "";
  const text = {
    idle: `Version ${v} · sucht beim Start nach Updates`,
    checking: "Sucht …",
    current: `Version ${v} · aktuell`,
    available: `Version ${s?.version ?? ""} ist da (installiert: ${v})`,
    downloading: "Lädt … Helio startet gleich neu",
    error: s?.message ?? "Prüfung fehlgeschlagen",
  }[st];
  return (
    <article className="source-tile n-card lg">
      <i className="source-dot" aria-hidden="true" />
      <div><b>Helio-Version</b><small className={st === "error" ? "warn" : ""}>{text}</small></div>
      <button className={"n-btn" + (st === "available" ? " primary" : "")} disabled={st === "checking" || st === "downloading"} onClick={st === "available" ? install : check}>
        {st === "available" ? "Installieren" : "Nach Updates suchen"}
      </button>
    </article>
  );
}

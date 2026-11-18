import { Fragment, type ReactNode, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Seite „Fragen“ (chat.rs): Fragen an Claude über das installierte Claude Code; Claude bekommt nur Helios Werkzeuge (mcp.rs).
// Standardmäßig aus, weil Fragen und abgefragte Werte an Anthropic gehen.
type Status = { enabled: boolean; claude: boolean; running: boolean };
type Tool = { name: string; input: Record<string, unknown> };
type Msg = { role: "user" | "claude"; text: string; tools: Tool[]; error?: string; login?: boolean };
type ChatEvent =
  | { kind: "text"; text: string }
  | { kind: "tools"; tools: Tool[] }
  | { kind: "done"; error: boolean; text: string; login: boolean }
  | { kind: "error"; stopped: boolean; text: string };

const TOOL: Record<string, string> = {
  zeitraum: "Zeitraum", heute: "Heute", belastung: "Belastung", rhythmus: "Rhythmus", langzeit: "Langzeit",
  prognose: "Prognose", tag: "Tag", kennzahl: "Kennzahl",
};
const IDEAS = ["Wie habe ich diese Woche geschlafen?", "Was hat sich seit dem Sommer verändert?", "Wie erhole ich mich nach harten Trainings?"];

/// Kleines Markdown: Absätze, Listen, **fett**. Kein HTML aus der Antwort.
function Rich({ text }: { text: string }) {
  const inline = (s: string) => s.split(/(\*\*[^*]+\*\*)/g).map((p, i) => (p.startsWith("**") && p.endsWith("**") ? <b key={i}>{p.slice(2, -2)}</b> : <Fragment key={i}>{p}</Fragment>));
  const blocks: ReactNode[] = [];
  let list: string[] = [];
  const flush = () => { if (list.length) blocks.push(<ul key={blocks.length}>{list.map((l, i) => <li key={i}>{inline(l)}</li>)}</ul>); list = []; };
  for (const line of text.split("\n")) {
    const item = line.match(/^\s*(?:[-*•]|\d+\.)\s+(.*)/);
    if (item) { list.push(item[1]); continue; }
    flush();
    const h = line.match(/^#+\s+(.*)/);
    if (h) blocks.push(<p key={blocks.length}><b>{inline(h[1])}</b></p>);
    else if (line.trim()) blocks.push(<p key={blocks.length}>{inline(line)}</p>);
  }
  flush();
  return <>{blocks}</>;
}

export function Ask() {
  const [status, setStatus] = useState<Status | null>(null);
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const end = useRef<HTMLDivElement>(null);

  const refresh = () => invoke<Status>("chat_status").then(setStatus).catch((e) => setError(String(e)));
  useEffect(() => {
    void refresh();
    // letzte Claude-Nachricht fortschreiben
    const patch = (f: (m: Msg) => Msg) => setMsgs((ms) => ms.length && ms[ms.length - 1].role === "claude" ? [...ms.slice(0, -1), f(ms[ms.length - 1])] : ms);
    const un = listen<ChatEvent>("chat", ({ payload: e }) => {
      if (e.kind === "text") patch((m) => ({ ...m, text: m.text + e.text }));
      // nach einem Werkzeugaufruf beginnt eine neue Nachricht: Absatz statt aneinanderkleben
      else if (e.kind === "tools") patch((m) => ({ ...m, text: m.text && m.text + "\n\n", tools: [...m.tools, ...e.tools] }));
      else {
        setBusy(false);
        if (e.kind === "done" && e.error) patch((m) => ({ ...m, error: e.text || "Claude hat einen Fehler gemeldet.", login: e.login }));
        if (e.kind === "error") patch((m) => ({ ...m, error: e.stopped ? "Gestoppt." : e.text || "Claude Code hat sich ohne Antwort beendet." }));
      }
    });
    return () => { void un.then((f) => f()); };
  }, []);
  useEffect(() => { end.current?.scrollIntoView({ block: "nearest" }); }, [msgs]);

  const ask = (q: string) => {
    q = q.trim();
    if (!q || busy) return;
    setError("");
    setMsgs((ms) => [...ms, { role: "user", text: q, tools: [] }, { role: "claude", text: "", tools: [] }]);
    setDraft("");
    setBusy(true);
    invoke("chat_ask", { question: q }).catch((e) => { setBusy(false); setMsgs((ms) => ms.slice(0, -1)); setError(String(e)); });
  };
  const enable = (on: boolean) => invoke("chat_enable", { on }).then(refresh).catch((e) => setError(String(e)));
  const reset = () => invoke("chat_reset").then(() => { setMsgs([]); setBusy(false); }).catch((e) => setError(String(e)));

  if (!status) return <section className="days" aria-label="Fragen"><p className="fine">{error || "…"}</p></section>;
  if (!status.enabled) {
    return (
      <section className="days" aria-label="Fragen">
        <article className="day-group n-card lg ask-intro">
          <div className="n-eyebrow">Fragen an Claude</div>
          <p>Stell Fragen zu deinen Werten in eigenen Worten. Claude fragt dafür Helios Auswertungen ab und antwortet mit Zahlen.</p>
          <ul>
            <li>Läuft über dein installiertes Claude Code und dein Claude-Konto, in Helio liegt kein Passwort.</li>
            <li><b>Deine Fragen und die abgefragten Werte gehen an Anthropic.</b> Claude bekommt nur Helios Auswertungen, keine Dateien, keine Shell, kein Web.</li>
            <li>Jede Frage zählt gegen dein Claude-Kontingent.</li>
            <li>Antworten kommen von Claude, nicht von Helio: beschreibend, kein Befund.</li>
          </ul>
          {!status.claude && <p className="fine warn">Claude Code ist nicht installiert. Anleitung: code.claude.com/docs</p>}
          <button className="n-btn ask-on" onClick={() => void enable(true)}>Einschalten: Fragen gehen an Anthropic</button>
        </article>
        {error && <p className="fine warn" role="alert">{error}</p>}
      </section>
    );
  }

  return (
    <section className="days ask" aria-label="Fragen">
      {!msgs.length && (
        <article className="day-group n-card lg">
          <div className="n-eyebrow">Fragen an Claude</div>
          <small className="fine">Zum Beispiel:</small>
          <div className="ask-ideas">{IDEAS.map((q) => <button key={q} className="n-btn" onClick={() => ask(q)} disabled={busy}>{q}</button>)}</div>
        </article>
      )}
      <div className="ask-log" role="log" aria-live="polite">
        {msgs.map((m, i) => m.role === "user" ? (
          <p key={i} className="ask-q">{m.text}</p>
        ) : (
          <article key={i} className="day-group n-card lg ask-a">
            {m.tools.length > 0 && (
              <div className="ask-tools">{m.tools.map((t, k) => <span key={k}>Helio · {TOOL[t.name] ?? t.name}{t.input?.schluessel ? ` ${t.input.schluessel}` : ""}</span>)}</div>
            )}
            {m.text ? <Rich text={m.text} /> : !m.error && <p className="fine">Claude denkt nach …</p>}
            {m.error && <p className="fine warn">{m.login ? "Die Anmeldung bei Claude ist abgelaufen. In einem Terminal „claude“ starten und „/login“ eingeben, dann noch einmal fragen." : m.error}</p>}
          </article>
        ))}
        <div ref={end} />
      </div>
      {error && <p className="fine warn" role="alert">{error}</p>}
      <form className="ask-form" onSubmit={(e) => { e.preventDefault(); ask(draft); }}>
        <textarea className="n-input" rows={2} value={draft} maxLength={4000} placeholder="Frage zu deinen Werten …" aria-label="Frage an Claude"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); ask(draft); } }} />
        {busy
          ? <button type="button" className="n-btn" onClick={() => void invoke("chat_stop")}>Stopp</button>
          : <button type="submit" className="n-btn" disabled={!draft.trim()}>Fragen</button>}
      </form>
      <div className="ask-foot">
        <small className="fine">Antworten von Claude (Anthropic), beschreibend, kein Befund.</small>
        <button className="n-btn" onClick={() => void reset()} disabled={busy || !msgs.length}>Neues Gespräch</button>
        <button className="n-btn" onClick={() => void enable(false)}>Ausschalten</button>
      </div>
    </section>
  );
}

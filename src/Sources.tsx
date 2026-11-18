import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { typeLabel } from "./health-types";
import { UpdateRow } from "./Update";

// Seite „Quellen“: Was kommt an, seit wann, wie viel, und genau eine nächste Handlung je Quelle (wie in Haze).
type TypeCount = { t: string; n: number; last: number };
type HealthStatus = {
  dir: string; exists: boolean; samples: number; first_day: string | null; last_day: string | null;
  types: TypeCount[]; sources: [string, number][]; running: boolean; error: string | null;
};
type ImportResult = { files: number; added: number; known: number; skipped: number };
type BleStatus = { state: string };

const num = (n: number) => n.toLocaleString("de-DE");
const date = (v: number | string | null | undefined) => {
  if (v == null) return "–";
  const d = typeof v === "number" ? new Date(v) : new Date(`${v}T12:00:00`);
  return d.toLocaleDateString("de-DE", { day: "2-digit", month: "2-digit", year: "2-digit" });
};
const ago = (ms: number) => {
  const m = Math.round((Date.now() - ms) / 60_000);
  return m < 1 ? "gerade" : m < 60 ? `vor ${m} min` : m < 48 * 60 ? `vor ${Math.round(m / 60)} h` : `vor ${Math.round(m / 1440)} Tagen`;
};
const BLE: Record<string, string> = { start: "startet", suche: "sucht Band", verbinde: "verbindet", live: "live", "kein-adapter": "kein Bluetooth" };

function Tile({ name, ok, detail, children }: { name: string; ok: boolean; detail: string; children?: React.ReactNode }) {
  return (
    <article className={"source-tile n-card lg" + (ok ? " ok" : "")}>
      <i className="source-dot" aria-hidden="true" />
      <div>
        <b>{name}</b>
        <small>{detail}</small>
        <span className="sr-only">{ok ? "verbunden" : "nicht verbunden"}</span>
      </div>
      {children}
    </article>
  );
}

export function Sources() {
  const [health, setHealth] = useState<HealthStatus | null>(null);
  const [ble, setBle] = useState<BleStatus | null>(null);
  const [strap, setStrap] = useState<[number, number] | null>(null);
  const [result, setResult] = useState<ImportResult | null>(null);
  const [error, setError] = useState("");

  const load = async () => {
    try {
      const [h, r, s] = await Promise.all([
        invoke<HealthStatus>("health_status"),
        invoke<[number, number] | null>("range"),
        invoke<{ status: BleStatus }>("snapshot"),
      ]);
      setHealth(h); setStrap(r); setBle(s.status); setError("");
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    void load();
    const un = [
      // null = Abgleich beginnt, sonst Ergebnis (oder null bei Fehler) am Ende
      listen<ImportResult | null>("health", (e) => { if (e.payload) setResult(e.payload); void load(); }),
      listen<BleStatus>("status", (e) => setBle(e.payload)),
    ];
    return () => un.forEach((p) => void p.then((f) => f()));
  }, []);

  const h = health;
  const healthDetail = !h ? "…"
    : h.running ? "Abgleich läuft …"
    : h.error ? `Abgleich fehlgeschlagen: ${h.error}`
    : h.samples ? `${date(h.first_day)} – ${date(h.last_day)} · ${num(h.samples)} Werte · ${h.types.length} Wertarten`
    : h.exists ? "Ordner da, noch keine Werte"
    : "Noch nichts angekommen. In der iPhone-App „Health abgleichen“ oder den Health-Export über Teilen › Notch schicken.";

  const live = ble?.state === "live";
  const strapDetail = [ble ? BLE[ble.state] ?? ble.state : "…", strap ? `aufgezeichnet seit ${date(strap[0])}` : null].filter(Boolean).join(" · ");

  return (
    <section className="sources" aria-label="Datenquellen">
      <div className="n-eyebrow">Datenquellen</div>
      <Tile name="Helio Strap" ok={live} detail={strapDetail}>
        {ble?.state === "suche" && <small className="hint">Sucht er lange: in Zepp › Helio Strap › „Heart Rate Push“ einschalten.</small>}
      </Tile>
      <Tile name="Apple Health" ok={!!h?.samples} detail={healthDetail}>
        <button className="n-btn" disabled={!h?.exists || h.running} onClick={() => void invoke("health_sync")}>
          Jetzt abgleichen
        </button>
      </Tile>

      {result && (result.added > 0 || result.skipped > 0) && (
        <p className="fine" role="status">
          Letzter Abgleich: {num(result.added)} neue Werte{result.skipped ? `, ${num(result.skipped)} unlesbare Zeilen übersprungen` : ""}.
        </p>
      )}
      {error && <p className="fine warn" role="alert">{error}</p>}

      {!!h?.types.length && (
        <details className="more n-card lg">
          <summary>Was aus Apple Health ankommt</summary>
          <table>
            <tbody>
              {h.types.map((t) => (
                <tr key={t.t}>
                  <td>{typeLabel(t.t)}</td>
                  <td className="n-num">{num(t.n)}</td>
                  <td>{ago(t.last)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="n-eyebrow">Von</div>
          <table>
            <tbody>
              {h.sources.map(([src, n]) => (
                <tr key={src}>
                  <td>{src}</td>
                  <td className="n-num">{num(n)}</td>
                  <td></td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="fine">Ordner: {h.dir}</p>
        </details>
      )}
      <UpdateRow />
      <p className="fine">Helio liest diese Quellen nur. Es schreibt nichts zurück und braucht keine Anmeldung.</p>
    </section>
  );
}

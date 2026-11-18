import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { METRIC, dur } from "./metrics";
import { localDay } from "./chart";

// Gemeinsame Daten für Heute, Schlaf und Körper: eine Abfrage (today_view, normal.rs), ein Speicher, alle Seiten lesen daraus.
// Letzte Nacht und letzter vollständiger Tag gegen den eigenen üblichen Bereich. Kein Gesamtwert, nur Abweichungen. Beschreibung, keine Bewertung.
export type Usual = { median: number; lo: number; hi: number; n: number };
export type Item = {
  key: string; day: string; value: number | null; src: string | null; quality: number;
  usual: Usual | null; status: "höher" | "niedriger" | "üblich" | null; series: (number | null)[];
};
export type Debt = { minutes: number; need: number; half_life: number | null; rho: number | null; nights: number };
type Part = { key: string; value: number; usual: number; z: number };
type Check = { episodes: number; caught: number; runs: number; unmarked: number; years: number };
export type Signal = { day: string; active: boolean; nights: number; score: number | null; threshold: number; parts: Part[]; maybe: string[]; history: (number | null)[]; check: Check };
export type TodayView = { night_day: string | null; day_day: string | null; items: Item[]; debt: Debt | null; signal: Signal | null };

/// Bei Uhrzeiten heißt „höher“ später.
export const word = (it: Item) => {
  const clockKind = METRIC.get(it.key)?.kind === "clock";
  return it.status === "höher" ? (clockKind ? "später" : "höher") : it.status === "niedriger" ? (clockKind ? "früher" : "niedriger") : "";
};
export const isOff = (it: Item) => it.status === "höher" || it.status === "niedriger";
/// Wie weit außerhalb des üblichen Bereichs, in Bereichsbreiten (0 = drin)
export const reach = (it: Item) => {
  const u = it.usual, v = it.value;
  if (!u || v == null) return 0;
  return Math.max(u.lo - v, v - u.hi, 0) / (u.hi - u.lo || Math.abs(u.median) || 1);
};
export const distance = (it: Item, d: number) => {
  const m = METRIC.get(it.key)!;
  return m.kind === "value" ? m.show(Math.abs(d)) : dur(Math.abs(d));
};
export const MAYBE: Record<string, string> = { alcohol: "Alkohol am Vorabend", training: "hartes Training am Vortag", short_sleep: "eine kurze Nacht" };
export const HUE = { sleep: "#b79ad0", heart: "#dc308f", move: "#ee8a52", food: "#8fb59a", body: "#6f87ff" };
export type G = keyof typeof HUE;
export const hm = (m: number) => `${Math.floor(Math.round(m) / 60)}:${String(Math.round(m) % 60).padStart(2, "0")}`;
const clamp01 = (x: number) => Math.min(1, Math.max(0, x));
/// Auffälligkeit eines Werts für die Lichter: je weiter außerhalb des Üblichen, desto heller
export const interest = (it?: Item) => (it ? clamp01(0.25 + reach(it) * 0.9) : 0.25);
export const arrow = (it?: Item) => (it?.status === "höher" ? "↑" : it?.status === "niedriger" ? "↓" : "");
export const bounds = (it: Item) => {
  const all = [...it.series.filter((x): x is number => x != null), ...(it.value != null ? [it.value] : []), ...(it.usual ? [it.usual.lo, it.usual.hi] : [])];
  const lo = Math.min(...all), hi = Math.max(...all), pad = (hi - lo) * 0.12 || 1;
  return [lo - pad, hi + pad] as const;
};

const RECOVERY = new Set(["sleep_min", "hrv", "rhr", "hr_night", "resp"]);
const ALERT_KEY = "helio-notch-alert";

/// Notch „Heute“: die zwei auffälligsten Abweichungen; einmal am Tag kurz aufklappen,
/// wenn nach der Nacht Schlaf oder Erholung deutlich (≥ halbe Bereichsbreite) außerhalb liegen.
function toNotch(view: TodayView) {
  // Frühzeichen zuerst; dann bleibt Platz für eine Abweichung
  const sig = view.signal?.active ? view.signal : null;
  const top = view.items.filter(isOff).sort((a, b) => reach(b) - reach(a)).slice(0, sig ? 1 : 2);
  const text = [...(sig ? ["Frühzeichen wie vor einem Infekt"] : []), ...top.map((i) => `${METRIC.get(i.key)?.label ?? i.key} ${word(i)}`)].join(" · ") || null;
  const today = localDay();
  let alert = false;
  if ((sig && sig.day === today) || top.some((i) => i.day === today && RECOVERY.has(i.key) && reach(i) >= 0.5)) {
    try { alert = localStorage.getItem(ALERT_KEY) !== today; } catch { /* ohne Speicher lieber kein Aufklappen als eins bei jedem Abgleich */ }
  }
  // Erst als gezeigt merken, wenn es wirklich rausging (Notch im Tray an)
  void invoke<boolean>("notch_today", { text, alert })
    .then((sent) => { if (alert && sent) try { localStorage.setItem(ALERT_KEY, today); } catch { /* s. o. */ } })
    .catch(() => {});
}

// ---------- Speicher ----------
type State = { view: TodayView | null; error: string; at: number };
let state: State = { view: null, error: "", at: 0 };
const subs = new Set<() => void>();
const put = (p: Partial<State>) => { state = { ...state, ...p }; subs.forEach((f) => f()); };

export const loadToday = () =>
  invoke<TodayView>("today_view", { today: localDay() })
    .then((v) => { put({ view: v, error: "", at: Date.now() }); toNotch(v); })
    .catch((e) => put({ error: String(e) }));

let started = false;
/// Einmal beim Start: laden und bei neuen Werten, Notch-Einschalten, Krank-Markierungen und nach Mitternacht neu laden.
export function startToday() {
  if (started) return;
  started = true;
  void loadToday();
  void listen<unknown>("health", (e) => { if (e.payload) void loadToday(); });
  void listen<boolean>("notch", (e) => { if (e.payload) void loadToday(); });
  void listen("marks", () => void loadToday());
  setInterval(() => void loadToday(), 30 * 60_000);
}

export function useToday() {
  const [s, setS] = useState(state);
  useEffect(() => {
    const f = () => setS(state);
    subs.add(f);
    f();
    return () => { subs.delete(f); };
  }, []);
  return s;
}

export const lookup = (view: TodayView | null) => (k: string) => view?.items.find((i) => i.key === k);

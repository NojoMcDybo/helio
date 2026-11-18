import { useEffect, useRef, useState } from "react";

// Gemeinsame Helfer der Seiten Heute, Belastung und Rhythmus.
export const localDay = (d = new Date()) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
export const short = (day: string) => new Date(`${day}T12:00:00`).toLocaleDateString("de-DE", { day: "2-digit", month: "2-digit" });

export function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(320);
  useEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver(([e]) => setW(Math.max(200, Math.round(e.contentRect.width))));
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);
  return [ref, w] as const;
}

/// y-Achse mit 3 runden Teilstrichen
export const ticks = (max: number) => {
  const step = 10 ** Math.floor(Math.log10(max / 3 || 1)), nice = [1, 2, 5, 10].map((m) => m * step).find((s) => max / s <= 4) ?? step * 10;
  return Array.from({ length: Math.floor(max / nice) + 1 }, (_, i) => i * nice);
};

export const median = (v: number[]) => {
  const s = [...v].sort((a, b) => a - b), m = s.length / 2;
  return s.length % 2 ? s[Math.floor(m)] : (s[m - 1] + s[m]) / 2;
};

/// Linie mit Lücken; einzelne Werte zwischen Lücken als kurzer Strich, sonst unsichtbar
export const gapPath = (v: (number | null)[], x: (i: number) => number, y: (v: number) => number, stub = 2) =>
  v.reduce<string>((s, p, i) => (p == null ? s : s + (i && v[i - 1] != null ? "L" : "M") + `${x(i).toFixed(1)} ${y(p).toFixed(1)}` + (v[i - 1] == null && v[i + 1] == null ? `h${stub}` : "")), "");

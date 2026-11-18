// Seite „Puls“: Live-Wert, Tagesverlauf und Zonen. Zeichnet direkt in die Elemente aus Pulse.tsx
// (Canvas und Animationen sind ohnehin imperativ); wird einmal geladen, sobald die Seite steht.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ICONS } from "./nojo/nojo-ui";

// ---------- Einstellungen ----------
/** Maximale Herzfrequenz für die Zonen (Faustformel 220 − Alter). */
const HRMAX = 200;
const LIVE_WINDOW = 5 * 60_000;
const ZONES = [
  { name: "Ruhe", color: "#8a8896" },
  { name: "Z1", color: "#b49bb4" },
  { name: "Z2", color: "#8fa3d0" },
  { name: "Z3", color: "#9aa56a" },
  { name: "Z4", color: "#efa91f" },
  { name: "Z5", color: "#c23a4d" },
];

type Status = { state: string; device?: string | null; detail?: string | null };
type Sample = { ts: number; bpm: number };
type Bucket = { t: number; avg: number; min: number; max: number };
type Stats = { count: number; min: number | null; max: number | null; avg: number | null; rest: number | null; zones: number[] };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
$("prev").innerHTML = ICONS.prev;
$("next").innerHTML = ICONS.next;

function zoneOf(bpm: number) {
  const p = bpm / HRMAX;
  return p < 0.5 ? 0 : p < 0.6 ? 1 : p < 0.7 ? 2 : p < 0.8 ? 3 : p < 0.9 ? 4 : 5;
}

// ---------- Status ----------
let searchingSince = 0;
function showStatus(s: Status) {
  const dot = $("dot");
  dot.className = "dot" + (s.state === "live" ? " live" : s.state === "kein-adapter" ? "" : " busy");
  if (s.state !== "suche") searchingSince = 0;
  else if (!searchingSince) searchingSince = Date.now();
  const text: Record<string, string> = {
    start: "startet",
    suche: "sucht Band",
    verbinde: "verbindet",
    live: "live",
    "kein-adapter": "kein Bluetooth",
  };
  $("state").textContent = text[s.state] ?? s.state;
  if (s.state !== "live") setLive(null);
}
// Sucht er zu lange, liegt es fast immer am Schalter in Zepp
setInterval(() => {
  if (searchingSince && Date.now() - searchingSince > 25_000) {
    $("state").textContent = "sucht Band · Heart Rate Push in Zepp an?";
  }
}, 2000);

// ---------- Live-Wert ----------
let lastSample = 0;
function setLive(bpm: number | null) {
  const heart = $("heart");
  if (bpm == null) {
    $("bpm").textContent = "–";
    $("zone").textContent = "";
    heart.classList.add("off");
    pulse.bpm = 0;
    showMeter(null);
    return;
  }
  $("bpm").textContent = String(bpm);
  heart.classList.remove("off");
  pulse.bpm = bpm;
  const z = zoneOf(bpm);
  const zone = $("zone");
  zone.textContent = z === 0 ? "Ruhe" : `${ZONES[z].name} · ${Math.round((bpm / HRMAX) * 100)} %`;
  zone.style.color = z === 0 ? "" : ZONES[z].color;
  showMeter(bpm);
}

// ---------- Zonen-Skala unter der Zahl: wo liegt der Puls gerade ----------
const meter = $("meter");
for (let i = 1; i < ZONES.length; i++) {
  const s = document.createElement("i");
  s.style.setProperty("--c", ZONES[i].color);
  meter.insertBefore(s, $("meterdot"));
}
function showMeter(bpm: number | null) {
  meter.classList.toggle("live", bpm != null);
  const segs = [...meter.querySelectorAll<HTMLElement>("i:not(.meter-dot)")];
  const z = bpm == null ? -1 : zoneOf(bpm);
  segs.forEach((s, i) => s.classList.toggle("on", i + 1 === z));
  if (bpm == null) return;
  // Z1..Z5 decken 50–100 % der Maximalfrequenz ab
  const p = Math.min(1, Math.max(0, (bpm / HRMAX - 0.5) / 0.5));
  $("meterdot").style.left = `${p * 100}%`;
}

// ---------- Herzschlag im Takt des Live-Pulses ----------
// Das Band liefert nur die Rate, keine einzelnen Schläge (keine RR-Intervalle). Der Taktgeber
// läuft deshalb mit der aktuellen Rate und behält seine Phase: ändert sich der Puls, wird der
// nächste Schlag früher/später, statt die Animation mitten im Schlag neu zu starten.
const BEAT: Keyframe[] = [
  { transform: "scale(1)", offset: 0 },
  { transform: "scale(1.2)", offset: 0.14 },
  { transform: "scale(0.97)", offset: 0.3 },
  { transform: "scale(1.09)", offset: 0.44 },
  { transform: "scale(1)", offset: 0.72 },
  { transform: "scale(1)", offset: 1 },
];
const GLOW: Keyframe[] = [
  { filter: "drop-shadow(0 0 10px rgba(255,90,110,0.35))", offset: 0 },
  { filter: "drop-shadow(0 0 22px rgba(255,90,110,0.75))", offset: 0.14 },
  { filter: "drop-shadow(0 0 10px rgba(255,90,110,0.35))", offset: 1 },
];
const pulse = { bpm: 0, phase: 0, last: 0 };
const reduced = matchMedia("(prefers-reduced-motion: reduce)");
function tick(t: number) {
  const dt = pulse.last ? Math.min((t - pulse.last) / 1000, 2) : 0;
  pulse.last = t;
  if (pulse.bpm > 0 && !document.hidden) {
    pulse.phase += (dt * pulse.bpm) / 60;
    if (pulse.phase >= 1) {
      pulse.phase %= 1;
      if (!reduced.matches) {
        const heart = $("heart");
        const duration = Math.min(560, (60_000 / pulse.bpm) * 0.62);
        heart.animate(BEAT, { duration, easing: "ease-out" });
        heart.animate(GLOW, { duration, easing: "ease-out" });
      }
    }
  }
  requestAnimationFrame(tick);
}
requestAnimationFrame(tick);
// Kommt nichts mehr, nicht ewig den alten Wert zeigen
setInterval(() => {
  if (lastSample && Date.now() - lastSample > 10_000) setLive(null);
}, 2000);

// ---------- Canvas-Helfer ----------
function prep(c: HTMLCanvasElement) {
  const r = c.getBoundingClientRect();
  const d = window.devicePixelRatio || 1;
  const w = Math.round(r.width * d), h = Math.round(r.height * d);
  if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
  const g = c.getContext("2d")!;
  g.setTransform(d, 0, 0, d, 0, 0);
  g.clearRect(0, 0, r.width, r.height);
  return { g, w: r.width, h: r.height };
}
const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();

// ---------- Live-Kurve (letzte 5 min) ----------
let live: Sample[] = [];
function drawLive() {
  const { g, w, h } = prep($("live"));
  const now = Date.now();
  live = live.filter((s) => s.ts > now - LIVE_WINDOW);
  if (live.length < 2) return;
  let lo = Math.min(...live.map((s) => s.bpm)), hi = Math.max(...live.map((s) => s.bpm));
  if (hi - lo < 20) { const m = (hi + lo) / 2; lo = m - 10; hi = m + 10; }
  const pad = 14;
  const x = (t: number) => ((t - (now - LIVE_WINDOW)) / LIVE_WINDOW) * w;
  const y = (b: number) => pad + (1 - (b - lo) / (hi - lo)) * (h - 2 * pad);
  const acc = css("--accent");

  // Abschnitte ohne Lücken > 5 s getrennt zeichnen, jeder mit eigener Fläche
  const runs: Sample[][] = [];
  for (const s of live) {
    const r = runs[runs.length - 1];
    if (r && s.ts - r[r.length - 1].ts <= 5000) r.push(s); else runs.push([s]);
  }
  const grad = g.createLinearGradient(0, 0, 0, h);
  grad.addColorStop(0, "rgba(255,90,110,0.22)");
  grad.addColorStop(1, "rgba(255,90,110,0)");
  for (const r of runs) {
    if (r.length < 2) continue;
    const path = new Path2D();
    r.forEach((s, i) => (i ? path.lineTo(x(s.ts), y(s.bpm)) : path.moveTo(x(s.ts), y(s.bpm))));
    const area = new Path2D(path);
    area.lineTo(x(r[r.length - 1].ts), h);
    area.lineTo(x(r[0].ts), h);
    area.closePath();
    g.fillStyle = grad;
    g.fill(area);
    g.strokeStyle = acc;
    g.lineWidth = 2;
    g.lineJoin = "round";
    g.stroke(path);
  }

  // aktueller Punkt
  const last = live[live.length - 1];
  g.fillStyle = acc;
  g.beginPath();
  g.arc(x(last.ts), y(last.bpm), 3.5, 0, Math.PI * 2);
  g.fill();

  g.fillStyle = css("--fg3");
  g.font = "10px Segoe UI";
  g.textAlign = "left";
  g.fillText(String(Math.round(hi)), 8, 14);
  g.fillText(String(Math.round(lo)), 8, h - 6);
  g.textAlign = "right";
  g.fillText("5 min", w - 8, h - 6);
}

// ---------- Tag ----------
let dayStart = startOfDay(Date.now());
let dayData: Bucket[] = [];
let firstDay = dayStart;

function startOfDay(t: number) { const d = new Date(t); d.setHours(0, 0, 0, 0); return d.getTime(); }
function addDays(t: number, n: number) { const d = new Date(t); d.setDate(d.getDate() + n); return d.getTime(); }
const isToday = () => dayStart === startOfDay(Date.now());

function dayLabel(t: number) {
  const today = startOfDay(Date.now());
  if (t === today) return "Heute";
  if (t === addDays(today, -1)) return "Gestern";
  return new Date(t).toLocaleDateString("de-DE", { weekday: "short", day: "2-digit", month: "2-digit" });
}

async function loadDay() {
  const from = dayStart, to = addDays(dayStart, 1);
  $("date").textContent = dayLabel(dayStart);
  ($("next") as HTMLButtonElement).disabled = isToday();
  ($("prev") as HTMLButtonElement).disabled = dayStart <= firstDay;
  const [series, st] = await Promise.all([
    invoke<Bucket[]>("series", { from, to, bucket: 60_000 }),
    invoke<Stats>("stats", { from, to, hrmax: HRMAX }),
  ]);
  dayData = series;
  // Lichtstrich unter dem Datum: aufgezeichnete Minuten im Verhaeltnis zum (bisherigen) Tag
  const span = isToday() ? Math.max(1, Date.now() - dayStart) : 86_400_000;
  const recorded = series.length * 60_000;
  const share = Math.min(1, recorded / span);
  $("cover").style.width = `${share * 100}%`;
  const hrs = (ms: number) => (ms / 3_600_000).toFixed(1).replace(".", ",");
  $("date").parentElement!.title = `Aufgezeichnet: ${hrs(recorded)} h von ${hrs(span)} h`;
  const f = (v: number | null) => (v == null ? "–" : String(Math.round(v)));
  $("s-rest").textContent = f(st.rest);
  $("s-avg").textContent = f(st.avg);
  $("s-min").textContent = f(st.min);
  $("s-max").textContent = f(st.max);
  drawZones(st.zones);
  drawDay();
}

function drawZones(z: number[]) {
  const total = z.reduce((a, b) => a + b, 0);
  const bar = $("zbar"), leg = $("zlegend");
  bar.innerHTML = "";
  leg.innerHTML = "";
  if (!total) return;
  z.forEach((n, i) => {
    if (!n) return;
    const seg = document.createElement("div");
    seg.style.flexGrow = String(n);
    seg.style.background = ZONES[i].color;
    seg.style.setProperty("--c", ZONES[i].color);
    seg.dataset.zone = String(i);
    bar.appendChild(seg);
    const min = Math.round(n / 60);
    const l = document.createElement("span");
    l.dataset.zone = String(i);
    l.innerHTML = `<i style="background:${ZONES[i].color}"></i>${ZONES[i].name} ${min < 60 ? min + " min" : (min / 60).toFixed(1).replace(".", ",") + " h"}`;
    leg.appendChild(l);
  });
}

// Zonen-Leiste ist zugleich Filter: zeigen hebt die Minuten dieser Zone im Tagesverlauf hervor
let zoneFocus: number | null = null;
function setZoneFocus(z: number | null) {
  if (z === zoneFocus) return;
  zoneFocus = z;
  $("zbar").classList.toggle("focus", z != null);
  document.querySelectorAll<HTMLElement>("#zbar div, #zlegend span").forEach((e) => e.classList.toggle("hot", Number(e.dataset.zone) === z && z != null));
  drawDay();
}
for (const id of ["zbar", "zlegend"]) {
  $(id).addEventListener("pointerover", (e) => {
    const z = (e.target as HTMLElement).closest<HTMLElement>("[data-zone]")?.dataset.zone;
    if (z != null) setZoneFocus(Number(z));
  });
  $(id).addEventListener("pointerleave", () => setZoneFocus(null));
}

// Kacheln Min/Max: zeigen markiert den Zeitpunkt im Verlauf
let mark: "min" | "max" | null = null;
document.querySelectorAll<HTMLElement>(".tile[data-mark=min], .tile[data-mark=max]").forEach((t) => {
  t.addEventListener("pointerenter", () => { mark = t.dataset.mark as "min" | "max"; drawDay(); });
  t.addEventListener("pointerleave", () => { mark = null; drawDay(); });
});

let hoverX: number | null = null;
function drawDay() {
  const c = $("daychart") as HTMLCanvasElement;
  const { g, w, h } = prep(c);
  const padT = 12, padB = 20;
  const span = 86_400_000;
  const x = (t: number) => ((t - dayStart) / span) * w;
  let lo = 40, hi = 120;
  for (const b of dayData) { lo = Math.min(lo, b.min); hi = Math.max(hi, b.max); }
  lo = Math.floor(lo / 10) * 10; hi = Math.ceil(hi / 10) * 10;
  const y = (v: number) => padT + (1 - (v - lo) / (hi - lo)) * (h - padT - padB);

  // Raster: Stunden 0/6/12/18/24 und zwei Pulslinien
  g.strokeStyle = css("--line");
  g.lineWidth = 1;
  g.fillStyle = css("--fg3");
  g.font = "10px Segoe UI";
  g.textAlign = "center";
  for (let hr = 0; hr <= 24; hr += 6) {
    const xx = Math.round((hr / 24) * w) + 0.5;
    g.beginPath(); g.moveTo(xx, padT); g.lineTo(xx, h - padB); g.stroke();
    g.fillText(String(hr), Math.min(Math.max(xx, 8), w - 8), h - 6);
  }
  g.textAlign = "left";
  for (const v of [lo, hi]) g.fillText(String(v), 6, y(v) + (v === lo ? -3 : 10));

  if (!dayData.length) return;
  const acc = css("--accent");
  // Spanne min..max als Band, Durchschnitt als Linie; Luecken > 5 min bleiben leer
  const runs: Bucket[][] = [];
  for (const b of dayData) {
    const r = runs[runs.length - 1];
    if (r && b.t - r[r.length - 1].t <= 5 * 60_000) r.push(b); else runs.push([b]);
  }
  for (const r of runs) {
    const band = new Path2D();
    r.forEach((b, i) => (i ? band.lineTo(x(b.t + 30_000), y(b.max)) : band.moveTo(x(b.t + 30_000), y(b.max))));
    for (let i = r.length - 1; i >= 0; i--) band.lineTo(x(r[i].t + 30_000), y(r[i].min));
    band.closePath();
    g.fillStyle = "rgba(255,90,110,0.16)";
    g.fill(band);
    const line = new Path2D();
    r.forEach((b, i) => (i ? line.lineTo(x(b.t + 30_000), y(b.avg)) : line.moveTo(x(b.t + 30_000), y(b.avg))));
    g.strokeStyle = acc;
    g.lineWidth = 1.5;
    g.lineJoin = "round";
    g.stroke(line);
    if (r.length === 1) { g.fillStyle = acc; g.fillRect(x(r[0].t), y(r[0].avg) - 1, 2, 2); }
  }

  // Zone hervorheben: Minuten, deren Mittel in der Zone liegt, als leise Saeulen in Zonenfarbe
  if (zoneFocus != null) {
    g.fillStyle = ZONES[zoneFocus].color;
    g.globalAlpha = 0.22;
    for (const b of dayData) if (zoneOf(b.avg) === zoneFocus) g.fillRect(x(b.t), padT, Math.max(1, x(b.t + 60_000) - x(b.t)), h - padT - padB);
    g.globalAlpha = 1;
  }

  // Min/Max-Kachel unter der Maus: Zeitpunkt mit leuchtendem Punkt und Uhrzeit
  const tip = $("tip");
  if (mark && dayData.length) {
    const b = dayData.reduce((a, c) => (mark === "min" ? (c.min < a.min ? c : a) : (c.max > a.max ? c : a)));
    const v = mark === "min" ? b.min : b.max;
    const mx = x(b.t + 30_000), my = y(v);
    g.strokeStyle = "rgba(255,255,255,0.4)";
    g.setLineDash([2, 3]);
    g.beginPath(); g.moveTo(mx + 0.5, padT); g.lineTo(mx + 0.5, h - padB); g.stroke();
    g.setLineDash([]);
    g.shadowColor = "#fff"; g.shadowBlur = 10;
    g.fillStyle = "#fff";
    g.beginPath(); g.arc(mx, my, 3.5, 0, Math.PI * 2); g.fill();
    g.shadowBlur = 0;
    const time = new Date(b.t).toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" });
    tip.textContent = `${mark === "min" ? "Tiefster" : "Höchster"} Puls ${v} bpm · ${time}`;
    tip.style.left = `${Math.min(Math.max(mx, 90), w - 90)}px`;
    tip.classList.add("on");
    return;
  }

  // Fadenkreuz
  if (hoverX == null) { tip.classList.remove("on"); return; }
  const t = dayStart + (hoverX / w) * span;
  let best: Bucket | null = null;
  for (const b of dayData) if (!best || Math.abs(b.t - t) < Math.abs(best.t - t)) best = b;
  if (!best || Math.abs(best.t - t) > 10 * 60_000) { tip.classList.remove("on"); return; }
  const bx = x(best.t + 30_000);
  g.strokeStyle = "rgba(255,255,255,0.35)";
  g.beginPath(); g.moveTo(bx + 0.5, padT); g.lineTo(bx + 0.5, h - padB); g.stroke();
  g.fillStyle = "#fff";
  g.beginPath(); g.arc(bx, y(best.avg), 3, 0, Math.PI * 2); g.fill();
  const time = new Date(best.t).toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" });
  tip.textContent = `${time}  ${Math.round(best.avg)} bpm  (${best.min}–${best.max})`;
  tip.style.left = `${Math.min(Math.max(bx, 80), w - 80)}px`;
  tip.classList.add("on");
}

const dc = $("daychart");
dc.addEventListener("mousemove", (e) => { hoverX = e.offsetX; drawDay(); });
dc.addEventListener("mouseleave", () => { hoverX = null; drawDay(); });

$("prev").onclick = () => { dayStart = addDays(dayStart, -1); loadDay(); };
$("next").onclick = () => { if (!isToday()) { dayStart = addDays(dayStart, 1); loadDay(); } };

// ---------- Start ----------
async function init() {
  const snap = await invoke<{ status: Status }>("snapshot");
  showStatus(snap.status);
  const r = await invoke<[number, number] | null>("range");
  if (r) firstDay = startOfDay(r[0]);
  const now = Date.now();
  const recent = await invoke<Bucket[]>("series", { from: now - LIVE_WINDOW, to: now + 1000, bucket: 1000 });
  live = recent.map((b) => ({ ts: b.t, bpm: Math.round(b.avg) }));
  await loadDay();
  drawLive();

  await listen<Status>("status", (e) => showStatus(e.payload));
  await listen<Sample>("hr", (e) => {
    lastSample = Date.now();
    setLive(e.payload.bpm);
    live.push(e.payload);
    drawLive();
  });
  // Tagesansicht jede Minute auffrischen; um Mitternacht auf den neuen Tag springen
  let shownToday = startOfDay(Date.now());
  setInterval(() => {
    const today = startOfDay(Date.now());
    if (today !== shownToday) { if (dayStart === shownToday) dayStart = today; shownToday = today; }
    if (isToday()) loadDay();
  }, 60_000);
  new ResizeObserver(() => { drawLive(); drawDay(); }).observe(document.body);
}
init();

// Lichter im Hintergrund mit eigener Bewegungs-Engine. Sie lenken beim Öffnen einer Seite die Aufmerksamkeit auf das
// Auffälligste: Jedes Licht hängt an einem Element mit data-light (Auffälligkeit 0..1) und data-lc (Farbe); gibt es keins,
// nehmen die ersten Glasflächen der Seite die Rolle (absteigend schwächer).
//  Eröffnung: die Lichter fahren nacheinander auf ihr Element, das auffälligste zuerst; die wichtigsten pulsieren kurz.
//  Danach: Feder zum Element plus Schweifen (je auffälliger, desto enger und heller), Abstoßung untereinander,
//  alle 7 bis 13 s ein Fokuswechsel (gewichtet nach Auffälligkeit und Sichtbarkeit), Lichter außerhalb des Bildes werden dunkler.
// Bei „weniger Bewegung“ stehen die Lichter still an ihrem Element.
type Light = { d: HTMLDivElement; el: HTMLElement; k: number; s: number; x: number; y: number; vx: number; vy: number; op: number; rank: number; ph: number[]; f: number[]; boost: number; tight: number; delay: number };

export function startLights(field: HTMLElement, stage: HTMLElement, scroller: HTMLElement, activePage: () => HTMLElement | null) {
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  let lights: Light[] = [], raf = 0, t0 = 0, last = 0, nextFocus = 0, key = "";

  const pick = () => {
    const page = activePage();
    if (!page) return [] as { el: HTMLElement; k: number; c: string }[];
    const tagged = [...page.querySelectorAll<HTMLElement>("[data-light]")];
    const list = tagged.length
      ? tagged.map((el) => ({ el, k: Number(el.dataset.light) || 0.3, c: el.dataset.lc || "#b79ad0" }))
      : [...page.querySelectorAll<HTMLElement>(".lg")].slice(0, 4).map((el, i) => ({ el, k: 0.7 - i * 0.15, c: getComputedStyle(el).getPropertyValue("--hue").trim() || "#b79ad0" }));
    return list.sort((a, b) => b.k - a.k).slice(0, 6);
  };

  const rebuild = () => {
    const picked = pick(), sig = picked.map((p) => p.el.id + p.k + p.c).join("|") + (activePage()?.id ?? "");
    if (sig === key && lights.length) return;
    key = sig;
    field.replaceChildren();
    lights = picked.map((p, rank) => {
      const s = 200 + p.k * 190, d = document.createElement("div");
      d.className = "light";
      d.style.cssText = `width:${s}px;height:${s}px;margin:${-s / 2}px 0 0 ${-s / 2}px;background:radial-gradient(closest-side,${p.c},${p.c}00)`;
      field.append(d);
      return { d, el: p.el, k: p.k, s, x: -200, y: -160, vx: 0, vy: 0, op: 0, rank, delay: 0.3 + rank * 0.55,
        ph: [0, 1, 2, 3].map(() => Math.random() * 6.28), f: [0.11 + Math.random() * 0.1, 0.07 + Math.random() * 0.08], boost: 0, tight: 0 };
    });
    t0 = last = performance.now();
    nextFocus = 9;
  };

  const info = (l: Light) => {
    const p = stage.getBoundingClientRect(), e = l.el.getBoundingClientRect(), m = scroller.getBoundingClientRect();
    return { x: e.left - p.left + e.width / 2, y: e.top - p.top + e.height / 2, vis: e.bottom > m.top && e.top < m.bottom ? 1 : 0.3 };
  };

  const frame = (now: number) => {
    const dt = Math.min(0.05, (now - last) / 1000 || 0.016), T = (now - t0) / 1000;
    last = now;
    if (T > nextFocus && lights.length) {
      const w = lights.map((l) => l.k * info(l).vis);
      let r = Math.random() * w.reduce((a, b) => a + b, 0), f = lights[0];
      for (let i = 0; i < lights.length; i++) { r -= w[i]; if (r <= 0) { f = lights[i]; break; } }
      f.boost = 1; f.tight = 1; nextFocus = T + 7 + Math.random() * 6;
    }
    for (const l of lights) {
      if (!l.el.isConnected) continue;
      const a = info(l), k = l.k, rest = 0.07 + 0.2 * k;
      let tx: number, ty: number, top: number;
      if (still) { tx = a.x; ty = a.y; top = rest * a.vis; }
      else if (T < l.delay) { tx = -200; ty = -160; top = 0; }
      else {
        const amp = (1 - k) * 70 + 14, tt = T - l.delay;
        tx = a.x + (Math.sin(tt * l.f[0] * 6.28 + l.ph[0]) * amp + Math.sin(tt * l.f[1] * 6.28 + l.ph[1]) * amp * 0.5) * (1 - 0.7 * l.tight);
        ty = a.y + (Math.cos(tt * l.f[0] * 6.28 + l.ph[2]) * amp * 0.7 + Math.sin(tt * l.f[1] * 6.28 + l.ph[3]) * amp * 0.4) * (1 - 0.7 * l.tight);
        top = rest * a.vis + 0.16 * l.boost + (tt < 2.6 && k > 0.5 ? 0.12 * Math.sin(Math.min(1, tt / 2.6) * 3.14) : 0);
      }
      l.boost = Math.max(0, l.boost - dt * 0.35); l.tight = Math.max(0, l.tight - dt * 0.3);
      const ks = T < l.delay + 1.8 ? 9 : 5, kd = T < l.delay + 1.8 ? 4.2 : 3.6; // zielstrebig bei der Anfahrt, weich im Ruhezustand
      l.vx += ((tx - l.x) * ks - l.vx * kd) * dt; l.vy += ((ty - l.y) * ks - l.vy * kd) * dt;
      for (const o of lights) {
        if (o === l) continue;
        const dx = l.x - o.x, dy = l.y - o.y, d = Math.hypot(dx, dy) || 1, m = (l.s + o.s) * 0.38;
        if (d < m && T > l.delay) { const f = (m - d) * 3; l.vx += (dx / d) * f * dt * 20; l.vy += (dy / d) * f * dt * 20; }
      }
      l.x += l.vx * dt; l.y += l.vy * dt; l.op += (top - l.op) * Math.min(1, dt * 2.4);
      const sc = 1 + 0.06 * Math.sin(T * 0.8 + l.ph[0]) + 0.12 * l.boost;
      l.d.style.transform = `translate3d(${l.x}px,${l.y}px,0) scale(${sc})`;
      l.d.style.opacity = String(l.op);
    }
    raf = requestAnimationFrame(frame);
  };

  const start = () => { cancelAnimationFrame(raf); rebuild(); raf = requestAnimationFrame(frame); };
  // Seitenwechsel und spät geladene Inhalte (Heute lädt Werte nach) bauen die Lichter neu
  let pend = 0;
  const mo = new MutationObserver(() => { clearTimeout(pend); pend = window.setTimeout(start, 250); });
  mo.observe(stage, { subtree: true, childList: true, attributes: true, attributeFilter: ["hidden", "data-light"] });
  start();
  return { restart: () => { key = ""; start(); }, stop: () => { cancelAnimationFrame(raf); mo.disconnect(); } };
}

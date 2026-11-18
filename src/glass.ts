// Liquid Glass für .lg: kaum Weichzeichnung, starke Brechung am Rand, leichte Wölbung über die ganze Fläche und eine
// Lichtkante oben links. Eigenes Modell (nojo-ui.ts liquid() bricht nur 12 px am Rand und wölbt nicht): pro Größe ein
// SVG-Filter (Displacement-Map) im backdrop-filter. Nur Chromium (WebView2); sonst bleibt .lg einfach dunkles Glas.
// Stärke je Fläche: data-r (Brechung in px), data-e (Randbreite in px), data-m (Wölbung, Anteil des Abstands zur Mitte).
const sm = (a: number, b: number, x: number) => { const t = Math.min(1, Math.max(0, (x - a) / (b - a))); return t * t * (3 - 2 * t); };
const LD = (() => { const l = Math.hypot(0.45, 0.9); return [-0.45 / l, -0.9 / l] as const; })();
const clamp8 = (v: number) => Math.min(255, Math.max(0, Math.round(v)));

function maps(w: number, h: number, r: number, R: number, E: number, M: number, dpr: number) {
  const W = Math.round(w * dpr), H = Math.round(h * dpr), disp = new ImageData(W, H), spec = new ImageData(W, H);
  const hw = w / 2, hh = h / 2, rr = Math.min(r, hw, hh), S = 2 * (R + hw * M) + 2;
  for (let j = 0; j < H; j++) {
    for (let i = 0; i < W; i++) {
      const x = (i + 0.5) / dpr - hw, y = (j + 0.5) / dpr - hh, qx = Math.abs(x) - hw + rr, qy = Math.abs(y) - hh + rr;
      const d = Math.min(Math.max(qx, qy), 0) + Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) - rr;
      let nx: number, ny: number;
      if (qx > 0 && qy > 0) { const l = Math.hypot(qx, qy); nx = qx / l; ny = qy / l; } else if (qx > qy) { nx = 1; ny = 0; } else { nx = 0; ny = 1; }
      if (x < 0) nx = -nx;
      if (y < 0) ny = -ny;
      const depth = Math.max(-d, 0), edge = 1 - sm(0, E, depth), off = edge * edge * R, k = (j * W + i) * 4;
      // nach innen greifen: am Rand liegt, was eigentlich weiter innen ist (Lupe)
      disp.data[k] = clamp8(128 + (-(nx * off + x * M) / S) * 255);
      disp.data[k + 1] = clamp8(128 + (-(ny * off + y * M) / S) * 255);
      disp.data[k + 2] = 128;
      disp.data[k + 3] = 255;
      const facing = Math.max(nx * LD[0] + ny * LD[1], 0) ** 3, rim = Math.exp(-(((depth - 1) / 1.1) ** 2)), cover = 1 - sm(-0.6, 0.6, d);
      const a = (rim * (0.2 + 0.55 * facing) + 0.08 * edge * facing) * cover;
      spec.data[k] = spec.data[k + 1] = spec.data[k + 2] = 255;
      spec.data[k + 3] = clamp8(a * 255);
    }
  }
  const url = (im: ImageData) => { const c = document.createElement("canvas"); c.width = W; c.height = H; c.getContext("2d")!.putImageData(im, 0, 0); return c.toDataURL(); };
  return { disp: url(disp), rim: url(spec), S };
}

type F = { id: string; rim: string; refs: number };
const cache = new Map<string, F>();
let defs: SVGSVGElement | null = null, seq = 0, on = false;

const supported = () => typeof CSS !== "undefined" && CSS.supports("backdrop-filter", "url(#n)") && /Chrome\/\d+/.test(navigator.userAgent);

function apply(el: HTMLElement, keys: Map<HTMLElement, string>) {
  if (!el.isConnected) return;
  const w = Math.round(el.offsetWidth), h = Math.round(el.offsetHeight);
  if (w < 8 || h < 8) return;
  const r = parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0;
  const R = Number(el.dataset.r) || 40, E = Number(el.dataset.e) || 30, M = el.dataset.m != null ? Number(el.dataset.m) : 0.06;
  const dpr = Math.min(2, Math.max(1, devicePixelRatio || 1));
  const key = [w, h, r, R, E, M, dpr].join("x");
  if (keys.get(el) === key) return;
  let f = cache.get(key);
  if (!f) {
    if (!defs) {
      defs = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      defs.setAttribute("aria-hidden", "true");
      defs.style.cssText = "position:absolute;width:0;height:0;overflow:hidden;pointer-events:none";
      document.body.append(defs);
    }
    const m = maps(w, h, r, R, E, M, dpr), id = `lg-${++seq}`;
    defs.insertAdjacentHTML("beforeend",
      `<filter id="${id}" x="0" y="0" width="${w}" height="${h}" filterUnits="userSpaceOnUse" primitiveUnits="userSpaceOnUse" color-interpolation-filters="sRGB">` +
      `<feImage href="${m.disp}" x="0" y="0" width="${w}" height="${h}" preserveAspectRatio="none" result="m"/>` +
      `<feDisplacementMap in="SourceGraphic" in2="m" scale="${m.S}" xChannelSelector="R" yChannelSelector="G"/></filter>`);
    f = { id, rim: m.rim, refs: 0 };
    cache.set(key, f);
  }
  keys.set(el, key);
  el.style.setProperty("--f", `url(#${f.id})`);
  el.style.setProperty("--rim", `url("${f.rim}")`);
}

/// Einmal aufrufen: jedes .lg (auch später eingefügte) bekommt Brechung und Lichtkante passend zu seiner Größe.
export function glass() {
  if (on || !supported()) return;
  on = true;
  const keys = new Map<HTMLElement, string>(), timers = new WeakMap<HTMLElement, number>();
  const later = (el: HTMLElement, wait = 120) => { clearTimeout(timers.get(el)); timers.set(el, window.setTimeout(() => apply(el, keys), wait)); };
  const ro = new ResizeObserver((l) => l.forEach((e) => later(e.target as HTMLElement)));
  const seen = new WeakSet<HTMLElement>();
  const scan = () => document.querySelectorAll<HTMLElement>(".lg").forEach((el) => { if (!seen.has(el)) { seen.add(el); ro.observe(el); later(el, 0); } });
  let pending = 0;
  new MutationObserver(() => { cancelAnimationFrame(pending); pending = requestAnimationFrame(scan); }).observe(document.body, { subtree: true, childList: true });
  scan();
}

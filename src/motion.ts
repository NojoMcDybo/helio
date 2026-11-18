// Aus Haze (src/motion.ts): „Bewegung reduzieren“ in Helio oder Windows schaltet Federn und Unschärfe ab.
export const reducedMotion = () =>
  document.documentElement.dataset.motion === "reduced" || matchMedia("(prefers-reduced-motion: reduce)").matches;

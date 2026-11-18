import { useEffect } from "react";
import { Icon } from "./Icon";

// Seite „Puls“: nur das Gerüst. Zeichnen und Live-Werte übernimmt pulse-draw.ts (Canvas, Herzschlag-Takt).
export function Pulse() {
  useEffect(() => {
    void import("./pulse-draw");
  }, []);
  return (
    <>
      <section className="hero">
        <span className="heart" id="heart">
          <Icon name="heart" size={34} />
        </span>
        <div className="bpm n-display">
          <span id="bpm">–</span>
          <small>bpm</small>
        </div>
        {/* Zonen-Skala: zeigt, wo der Puls gerade liegt (Licht = jetzt) */}
        <div className="meter" id="meter" aria-hidden="true">
          <i className="meter-dot" id="meterdot"></i>
        </div>
        <div className="zone" id="zone"></div>
      </section>

      <section className="n-card lg live-card">
        <canvas id="live"></canvas>
      </section>

      <section className="day">
        <nav>
          <button className="n-ico sm n-glass n-liquid" id="prev" title="Vorheriger Tag"></button>
          <div className="date">
            <span id="date">Heute</span>
            {/* Lichtstrich: so viel des Tages ist aufgezeichnet */}
            <i className="cover-track">
              <i className="n-bar" id="cover"></i>
            </i>
          </div>
          <button className="n-ico sm n-glass n-liquid" id="next" title="Nächster Tag"></button>
        </nav>
        <div className="tiles">
          <div className="n-card lg tile" data-mark="rest"><b id="s-rest" className="n-num">–</b><span>Ruhe</span></div>
          <div className="n-card lg tile" data-mark="avg"><b id="s-avg" className="n-num">–</b><span>Ø</span></div>
          <div className="n-card lg tile" data-mark="min" title="Zeigen: wann"><b id="s-min" className="n-num">–</b><span>Min</span></div>
          <div className="n-card lg tile" data-mark="max" title="Zeigen: wann"><b id="s-max" className="n-num">–</b><span>Max</span></div>
        </div>
        <div className="n-card lg chart">
          <canvas id="daychart"></canvas>
          <div className="tip n-glass n-liquid" id="tip"></div>
        </div>
      </section>

      <section className="zones">
        <div className="n-eyebrow">Zonen · zeigen hebt sie im Verlauf hervor</div>
        <div className="bar" id="zbar"></div>
        <div className="legend" id="zlegend"></div>
      </section>
    </>
  );
}

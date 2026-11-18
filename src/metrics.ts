// Kennzahlen der Tagesmatrix (daily.rs): Name, Gruppe und Darstellung. Gemeinsam für die Seiten Tage und Heute.
export const fmt = (v: number, d = 0) => v.toLocaleString("de-DE", { maximumFractionDigits: d, minimumFractionDigits: d });
export const dur = (m: number) => {
  const r = Math.round(m); // erst runden: 59,6 min soll nicht „60 min“ werden, 449,7 nicht „7 h 60 min“
  return r >= 60 ? `${Math.floor(r / 60)} h ${r % 60} min` : `${r} min`;
};
// Minuten ab Mitternacht (negativ = Vorabend) → Uhrzeit
export const clock = (m: number) => {
  const x = ((Math.round(m) % 1440) + 1440) % 1440;
  return `${String(Math.floor(x / 60)).padStart(2, "0")}:${String(x % 60).padStart(2, "0")}`;
};
// Die Notch schreibt Prozentwerte als 0–100
const pct = (v: number) => `${fmt(v, 1)} %`;
const unit = (u: string, d = 0) => (v: number) => `${fmt(v, d)} ${u}`;

/// `g` = Farbgruppe (styles.css, data-g)
export const GROUPS: { title: string; g: string; items: [string, string, (v: number) => string][] }[] = [
  { title: "Schlaf", g: "sleep", items: [
    ["sleep_min", "Geschlafen", dur], ["inbed_min", "Im Bett", dur], ["onset", "Eingeschlafen", clock], ["wake", "Aufgewacht", clock],
    ["mid", "Schlafmitte", clock], ["efficiency", "Schlafeffizienz", unit("%")], ["deep_min", "Tiefschlaf", dur], ["rem_min", "REM", dur],
    ["core_min", "Kernschlaf", dur], ["awake_min", "Wach in der Nacht", dur], ["nap_min", "Nickerchen", dur], ["hr_night", "Puls im Schlaf", unit("bpm")],
    ["hr_night_min", "Niedrigster Puls im Schlaf", unit("bpm")],
  ] },
  { title: "Herz und Erholung", g: "heart", items: [
    ["rhr", "Ruhepuls", unit("bpm")], ["hrv", "HRV (SDNN)", unit("ms")], ["hr_avg", "Puls im Schnitt", unit("bpm")],
    ["hr_min", "Puls niedrigster", unit("bpm")], ["hr_max", "Puls höchster", unit("bpm")], ["resp", "Atemfrequenz", unit("/min", 1)],
    ["spo2", "Sauerstoffsättigung", pct], ["spo2_min", "Sauerstoff niedrigster", pct], ["walk_hr", "Puls beim Gehen", unit("bpm")],
    ["vo2max", "VO₂max", unit("ml/kg/min", 1)],
  ] },
  { title: "Bewegung", g: "move", items: [
    ["steps", "Schritte", (v) => fmt(v)], ["distance", "Strecke", (v) => `${fmt(v / 1000, 1)} km`], ["flights", "Stockwerke", (v) => fmt(v)],
    ["active_kcal", "Aktive Energie", unit("kcal")], ["basal_kcal", "Ruheenergie", unit("kcal")], ["workout_n", "Trainings", (v) => fmt(v)],
    ["workout_min", "Trainingszeit", dur], ["workout_kcal", "Trainingsenergie", unit("kcal")], ["trimp", "Trainingslast", (v) => fmt(v)], ["walk_speed", "Gehtempo", (v) => `${fmt(v * 3.6, 1)} km/h`],
    ["step_length", "Schrittlänge", (v) => `${fmt(v * 100)} cm`], ["asymmetry", "Gangasymmetrie", pct], ["double_support", "Doppelstandphase", pct],
    ["steadiness", "Gangsicherheit", pct],
  ] },
  { title: "Ernährung", g: "food", items: [
    ["energy", "Energie", unit("kcal")], ["protein", "Eiweiß", unit("g")], ["carbs", "Kohlenhydrate", unit("g")], ["fat", "Fett", unit("g")],
    ["sugar", "Zucker", unit("g")], ["fiber", "Ballaststoffe", unit("g")], ["caffeine", "Koffein", unit("mg")], ["caffeine_late", "Koffein ab 14 Uhr", unit("mg")], ["water", "Wasser", unit("ml")],
    ["alcohol", "Alkohol", (v) => `${fmt(v, 1)} Getränke`], ["last_meal", "Letzte Mahlzeit", clock],
  ] },
  { title: "Körper und Sonstiges", g: "body", items: [
    ["weight", "Gewicht", unit("kg", 1)], ["bodyfat", "Körperfett", pct], ["bmi", "BMI", (v) => fmt(v, 1)],
    ["headphone_db", "Kopfhörer-Lautstärke", unit("dB")], ["mindful_min", "Achtsamkeit", dur],
  ] },
];
export const QUALITY = ["zu wenig Daten", "wenige Daten", "genug Daten"];

type Show = (v: number) => string;
/// Name und Darstellung je Schlüssel; `kind` sagt, wie ein Abstand zu lesen ist (Uhrzeit: früher/später).
export const METRIC = new Map<string, { label: string; show: Show; kind: "clock" | "dur" | "value"; g: string }>(
  GROUPS.flatMap((g) => g.items.map(([k, label, show]) => [k, { label, show, kind: show === clock ? "clock" : show === dur ? "dur" : "value", g: g.g }] as const)),
);

# Helio

Live-Puls vom Amazfit Helio Strap auf dem PC, mit Verlauf, Tagesstatistik und Anzeige in der [Notch](../notch).
Wird zur Health-App: liest Apple Health (über die Notch); Diabetes bleibt in Haze.
Tauri v2 + Rust + React.

## Wie es funktioniert

- Das Band bleibt mit dem iPhone (Zepp) gekoppelt. In Zepp → Gerät → Helio Strap → Health Monitoring muss
  **Heart Rate Push** an sein. Dann wirbt das Band zusätzlich als normaler Bluetooth-Pulsgurt
  (Heart Rate Service `0x180D`, Messung `0x2A37`). Ohne Kopplung mit Windows, ohne Schlüssel.
- iPhone und PC hängen **gleichzeitig** am Band. Auf PC-Seite aber nur **ein** Programm:
  hält ein zweites die Verbindung (z. B. ein vergessenes Python-Skript), ist das Band für alle anderen unsichtbar.
  Darum läuft Helio nur einmal (Single-Instance) und gibt die Verbindung beim Beenden frei.
- Das Band sendet ~1 Wert pro Sekunde, **nur BPM, keine RR-Intervalle** — HRV lässt sich daraus nicht berechnen.

## Starten

```
npm install
npm run tauri dev
```

Fenster schließen = in den Tray, die Aufzeichnung läuft weiter. Beenden über das Tray-Menü.

Tray-Menü: Helio öffnen · In der Notch zeigen · Mit Windows starten · Helio beenden.
Der installierte Build schaltet „Mit Windows starten“ beim ersten Start selbst ein (startet dann versteckt mit `--hidden`).

## Installieren

Lokal bauen (ohne privaten Schlüssel): `npx tauri build --config "{\"bundle\":{\"createUpdaterArtifacts\":false}}"`, danach `tools\install-helio.cmd` per Doppelklick im Explorer (nicht aus der Claude-App,
sonst landen Programm und AppData im virtualisierten Paketordner).

## Updates und Veröffentlichen

Helio aktualisiert sich selbst: 20 s nach dem Start und dann alle 6 h fragt es `https://github.com/NojoMcDybo/helio/releases/latest/download/latest.json`. Gibt es eine neue Version, erscheint oben ein Banner mit **Installieren** (lädt, prüft die Signatur mit dem öffentlichen Schlüssel aus `tauri.conf.json`, derselbe wie bei Arena, Notch und Streamdock, und startet neu). Auf der Seite Quellen steht die Version mit „Nach Updates suchen“. Solange es Repository oder Release nicht gibt, bleibt die automatische Prüfung still.

Veröffentlichen: Version in `package.json`, `src-tauri/Cargo.toml` und `src-tauri/tauri.conf.json` erhöhen, committen, Tag `v<version>` pushen. `.github/workflows/release.yml` testet, baut, signiert (Secret `TAURI_SIGNING_PRIVATE_KEY`, der private Schlüssel liegt nur unter `%USERPROFILE%\.tauri\nojo-updater.key`) und hängt Installer und `latest.json` an das Release. **Einmalig:** Repository `NojoMcDybo/helio` anlegen und das Secret setzen. Ein lokal gebauter Installer hat keine Signatur: er taugt zum Installieren, nicht als Update.

## Daten

SQLite unter `%APPDATA%\de.nojo.helio\helio.db`, Tabelle `hr (ts INTEGER PRIMARY KEY /* ms */, bpm INTEGER)`.
Rohwerte bleiben vollständig erhalten — Analysen können später beliebig neu darauf rechnen.

Commands fürs Frontend (`src-tauri/src/store.rs`):

| Command  | Parameter              | Ergebnis |
|----------|------------------------|----------|
| `series` | `from, to, bucket` (ms)| `[{t, avg, min, max}]` je Zeitfenster |
| `stats`  | `from, to, hrmax`      | `{count, min, max, avg, rest, zones[6]}` — `rest` = niedrigster 5-min-Schnitt, `zones` = Sekunden je Zone |
| `range`  | –                      | `[erster, letzter]` Zeitstempel |

Apple Health (`src-tauri/src/health.rs`): Die Notch legt Werte vom iPhone als Tagesdateien unter `Dokumente\Notch\Health\JJJJ-MM-TT.jsonl` ab
(Format: `notch/docs/health-export.md`). Helio liest beim Start, alle 5 Minuten und auf Knopfdruck nur die neuen Zeilen
in die Tabelle `sample` (`id` = UUID aus Health, Sonderfelder als JSON in `extra`); `import_state` merkt sich je Datei die gelesene Länge.
Helio schreibt nie in den Ordner der Notch. Zum Testen zeigt die Umgebungsvariable `HELIO_HEALTH_DIR` auf einen anderen Ordner.

| Command | Ergebnis |
|---------|----------|
| `health_status` | Ordner, Anzahl Werte, erster/letzter Tag, Anzahl je Wertart und je Quelle, läuft gerade ein Abgleich |
| `health_sync` | Abgleich starten; Ereignis `health` zu Beginn (`null`) und am Ende (Ergebnis) |

Tagesmatrix (`src-tauri/src/daily.rs`): Nach jedem Abgleich rechnet Helio die betroffenen Tage neu in die Tabelle
`daily (day, key, value, src, quality)`, eine Zeile je Tag und Kennzahl. Regeln:

- Summen (Schritte, Strecke, Energie, Essen): je Stunde die Quelle mit dem größten Wert, damit zwei iPhones, Zepp und Nike nicht doppelt zählen.
- Mittel, Min, Max, letzter Wert: nur eine Quelle, Vorrang Zepp > WHOOP > iPhone > Rest.
- Schlaf: Abschnitte, die zwischen 18 Uhr am Vortag und 18 Uhr beginnen, aus der besten Quelle mit Schlafphasen. Die Nacht gehört zum Aufwachtag.
- Uhrzeiten in Ortszeit des iPhones (Spalte `lstart`), Einschlafen/Aufwachen als Minuten ab Mitternacht (negativ = Vorabend).
- Qualität 2/1/0 = genug / wenige / zu wenig Daten (z. B. Puls: Stunden mit Werten ≥ 18 / ≥ 8).

Ändern sich die Regeln, `daily::VERSION` erhöhen: der nächste Abgleich rechnet alle Tage neu. Fehlt `sample.lstart` (Datenbank aus Block 1),
liest Helio die Health-Werte einmal komplett neu ein; die Puls-Tabelle bleibt.

| Command | Ergebnis |
|---------|----------|
| `daily_day` | `day` → `[{key, value, src, quality}]` |
| `daily_range` | erster und letzter Tag mit Kennzahlen |

Normalbereiche (`src-tauri/src/normal.rs`, Seite Heute): Je Kennzahl ist „üblich“ der Bereich, in dem 8 von 10 Tagen der
60 Tage davor lagen (10. bis 90. Perzentil, ab 14 Tagen). Gezeigt werden die letzte Nacht (Schlaf, Ruhepuls, HRV, Atmung, SpO2)
und der letzte vollständige Tag (Bewegung, Essen, Gewicht); knapp außerhalb (≤ 5 % der Bereichsbreite und ≤ 1 % des Medians)
zählt nicht. Schlafschuld: Bedarf = Median der Schlafdauer der 60 Nächte davor; abklingend mit der Halbwertszeit, bei der
hohe Schuld am deutlichsten mit längerem Schlaf danach einhergeht (Spearman, ρ·√(n−1) ≥ 2), sonst Summe der letzten 7 Nächte.

Belastung (`src-tauri/src/load.rs`, Seite Belastung): Trainingslast `trimp` je Tag in der Tagesmatrix (Edwards-TRIMP:
je Trainingsminute 1–5 Punkte nach Zone in % von `HRMAX` = 200, Puls vom Strap, sonst Health). Fitness/Ermüdung nach Banister
(42/7 Tage), Pulserholung 60/120 s nach Trainings ab 10 min, Ruhepuls/HRV 1–2 Tage nach harten Tagen (oberste 20 % der Last)
gegen die übrigen Tage, mit 95-%-Bereich.

| Command | Ergebnis |
|---------|----------|
| `load_view` | `today` → `{days[{day, trimp, fitness, fatigue}] (90 Tage), recoveries[{day, end, minutes, hr_end, drop60, drop120, src}] (180 Tage), after[{key, lag, n, mean, ci, other_n, other_mean}], hard}` |
| `rhythm_view` | `today` → `{to, profile[24] (Schritte je Stunde, 4 Wochen), now: Woche über 4 Wochen, sri_before, jetlag{work, free, n_work, n_free}, weeks[{start, days, steps, hr: {mesor, amp, peak, r2, n}, is, iv, sri, mid}] (26 Wochen)}` |
| `longterm_view` | `today` → `{to, first, week_starts (52), metrics[{key, now, weeks, months[[JJJJ-MM, Mittel, Tage]], trend{n, per_month, change, p, q, status, slope, intercept}, changes[{day, before, after}]}], links[{x, y, lag, n, rho, ci, p, q, zero, low_max, high_min, low_median, high_median, status, confirmed}]}` |
| `today_view` | `today` (Ortszeit, JJJJ-MM-TT) → `{night_day, day_day, items[{key, value, usual{median,lo,hi,n}, status, series}], debt}` |

Rhythmus (`src-tauri/src/rhythm.rs`, Seite Rhythmus): Stundenwerte (Tabelle `hourly`: Schritte der größten Quelle, Puls als
Zeitmittel) aus der Tagesmatrix. Kosinor (24-h-Welle, Uhrzeit des Höchstwerts) auf Schritte und Puls, IS/IV (Witting 1990)
auf Schritte je Stunde, SRI (Phillips 2017) aus 5-min-Abschnitten schlafend/wach, sozialer Jetlag = Schlafmitte freie Tage
(Aufwachen Sa/So) minus Werktage. Je Woche (26 Wochen) und über die letzten 4 Wochen.

Langzeit (`src-tauri/src/longterm.rs`, Seite Langzeit): Wochen- und Monatsmittel, Trend über 52 Wochen (Theil-Sen,
Mann-Kendall, FDR über alle Kennzahlen, nur spürbar große Änderungen), Wendepunkte im letzten Jahr, 12 feste Fragen zu
Zusammenhängen (Spearman mit Permutationstest, Bootstrap-Intervall, FDR, Bestätigung im neuesten Drittel). Statistik in
`src-tauri/src/stats.rs`, übertragen aus Haze `shared/analysis/stats.mjs`.

Frühzeichen (`src-tauri/src/signal.rs`, Seite Heute und Notch): Puls im Schlaf, Ruhepuls, Atemfrequenz und HRV je Nacht
gegen dein Übliches (Median der Tage 8–60 davor, robuste Streuung). Auffällig ist eine Nacht, wenn der Mittelwert der Abstände
(je Wert bei 4 gedeckelt) ≥ 1,2 ist und mindestens zwei Werte, darunter ein Puls, je ≥ 1 abweichen; Hinweis nach zwei
auffälligen Nächten hintereinander, „wie oft vor einem Infekt“. Keine Unterscheidung von Krankheiten. Genannt wird, was dasselbe
Muster auch erklären kann (Alkohol am Vorabend, Training über deinem P90, Nacht unter deinem P10). Krank-Tage markierst du auf
der Seite Tage (Tabelle `mark`, nur in Helio); der Rückblick zählt, bei wie vielen Krank-Phasen der Hinweis bis 3 Tage vorher kam
und wie viele Hinweise ohne markierte Krankheit blieben. Die Regel ist fest (nicht an den Markierungen eingestellt). Nach
Mishra et al. 2020, Alavi et al. 2022 („NightSignal“), Quer et al. 2021.

Prognose-Experiment (`src-tauri/src/forecast.rs`, unten auf der Seite Langzeit, standardmäßig aus, nicht belastbar):
Ruhepuls und HRV der nächsten 7 Tage. Drei Modelle: wie zuletzt, Mittel der letzten 7 Tage, gelernte Rückkehr zum Üblichen
(Median der 28 Tage). Rückwärtsprüfung an jedem Tag der letzten 90 Tage; das gelernte Modell kennt diese Tage nicht. Gezeigt
wird das beste nur, wenn es „wie zuletzt“ um mindestens 5 % schlägt; das Band umfasst 8 von 10 Fehlern der Prüfung. Nie in
der Notch. Vorgehen nach Haze `shared/analysis/forecast.mjs` (1.11). TTM aus Haze 1.12 nicht übernommen: Es braucht 360
Werte am Stück, die Tageswerte reichen höchstens rund 60 Tage am Stück.

Zonen nach % der HFmax (`HRMAX = 200` in `src/pulse-draw.ts`, Faustformel 220 − Alter): Ruhe < 50 · Z1 50–60 · Z2 60–70 · Z3 70–80 · Z4 80–90 · Z5 ≥ 90.

## Notch

Jeder Wert geht als Live Activity `helio-hr` an `http://127.0.0.1:47800/activity` (ttl 15 s — reißt die Verbindung ab,
verschwindet der Puls von selbst). Klick auf die Zeile holt Helio nach vorn.

Dazu die Aktivität `helio-today` („Heute“, `app: "Helio Heute"`, sonst ordnet die Notch sie dem Puls zu): die ein bis zwei
Werte, die am weitesten außerhalb des üblichen Bereichs liegen (gemessen in Bereichsbreiten, Seite Heute), z. B.
„HRV (SDNN) höher · Ruhepuls höher“. Liegt alles im Üblichen, wird sie entfernt. Sie wird bei jedem Abgleich (5 Min.)
aufgefrischt, ttl 1 h. Einmal am Tag klappt die Notch kurz auf (`alert`), wenn unter den beiden ein Wert der letzten
Nacht (Schlafdauer, HRV, Ruhepuls, Nachtpuls, Atemfrequenz) mindestens eine halbe Bereichsbreite außerhalb liegt.
Die Entscheidung trifft Helio (`src/Today.tsx`), die Notch zeigt nur an. „In der Notch zeigen“ im Tray schaltet beides ab.

## Claude

Nur für Nojo selbst: Anthropic erlaubt die Anmeldung mit einem claude.ai-Konto nicht in weitergegebenen Apps.

**Helio als MCP-Server.** `helio.exe --mcp` spricht MCP über stdin/stdout und gibt Claude acht lesende Werkzeuge mit den
fertigen Auswertungen: `zeitraum`, `heute`, `belastung`, `rhythmus`, `langzeit`, `prognose`, `tag`, `kennzahl`. Glukose und
Insulin sind nicht dabei. Die Anfragen gehen über die Named Pipe `\\.\pipe\helio-mcp-<Benutzer>` an die laufende Helio; Helio
muss also laufen (sonst meldet das Werkzeug „Helio läuft nicht“). Grund für den Umweg: Programme, die die Claude-App startet,
sehen eine veraltete Kopie von `%APPDATA%`; eine Pipe ist davon nicht betroffen und für Webseiten nicht erreichbar.

- Claude-App: Einstellungen → Entwickler → Konfiguration bearbeiten, dort eintragen:
  `{"mcpServers": {"helio": {"command": "C:\\Users\\<Name>\\AppData\\Local\\Helio\\helio.exe", "args": ["--mcp"]}}}`
- Claude Code: `claude mcp add helio -- "%LOCALAPPDATA%\Helio\helio.exe" --mcp`

**Seite „Fragen“.** Fragen in eigenen Worten, die Antwort läuft live mit. Helio startet dafür das installierte Claude Code
(`claude -p`, Anmeldung mit dem eigenen Claude-Konto, kein Passwort in Helio) in einem leeren Ordner und gibt ihm nur Helios
Werkzeuge: keine eingebauten Werkzeuge (`--tools ""`), keine anderen MCP-Server, keine eigenen Einstellungen. Standardmäßig aus;
beim Einschalten steht da, dass Fragen und abgefragte Werte an Anthropic gehen. Jede Frage zählt gegen das Claude-Kontingent.
Ist die Anmeldung abgelaufen, in einem Terminal `claude` starten und `/login` eingeben.

## Aufbau

- `src-tauri/src/ble.rs` — Suche, Verbindung, Neuverbinden (btleplug), Freigabe beim Beenden
- `src-tauri/src/store.rs` — SQLite und Abfragen
- `src-tauri/src/health.rs` — Apple-Health-Import aus den Dateien der Notch
- `src-tauri/src/daily.rs` — Tagesmatrix: Kennzahlen je Tag, Quellenvorrang, Datenqualität
- `src-tauri/src/normal.rs` — persönliche Normalbereiche, Schlafschuld
- `src-tauri/src/load.rs` — Fitness/Ermüdung, Pulserholung, Erholung nach harten Tagen
- `src-tauri/src/longterm.rs` — Trends, Wendepunkte, Zusammenhänge; `src-tauri/src/stats.rs` Statistik (aus Haze)
- `src-tauri/src/rhythm.rs` — Tagesrhythmus (Kosinor, IS/IV), Schlafregelmäßigkeit, sozialer Jetlag
- `src-tauri/src/signal.rs` — Frühzeichen, Krank-Markierungen, Rückblick
- `src-tauri/src/forecast.rs` — Prognose-Experiment mit Rückwärtsprüfung
- `src-tauri/src/notch.rs` — Live Activities für die Notch (Puls, „Heute“)
- `src-tauri/src/mcp.rs` — MCP-Server (`--mcp`) und Pipe in der laufenden Helio; `src-tauri/src/chat.rs` + `src/Ask.tsx` Seite Fragen
- `src-tauri/src/lib.rs` — Tray, Single-Instance, Autostart, Fenster
- `src/main.tsx` — Reiter und Seiten (`Pager.tsx` und `Icon.tsx` sind Kopien aus Haze)
- `src/Pulse.tsx` + `src/pulse-draw.ts` — Seite Puls (Canvas und Herzschlag bleiben imperativ)
- `src/Load.tsx` — Seite Belastung (Diagramme in Tinte, Klassen mit `chart-` vor der Puls-Seite geschützt)
- `src/Longterm.tsx` — Seite Langzeit; `src/Forecast.tsx` Karte Prognose-Experiment
- `src/Rhythm.tsx` — Seite Rhythmus; `src/chart.ts` gemeinsame Diagramm-Helfer
- `src/Today.tsx` — Seite Heute (Kachel nach Haze `Body.tsx` TrendTile)
- `src/Days.tsx` — Seite Tage (Kennzahlen eines Tages); `src/metrics.ts` Namen und Formate der Kennzahlen
- `src/Sources.tsx`, `src/health-types.ts` — Seite Quellen, deutsche Namen der Wertarten
- `src/styles.css` — Aussehen

## Ideen für später

- Zepp-Cloud (inoffizielle API) für HRV, Schlaf, Stress, SpO2, BioCharge
- Trainings erkennen und als Einheiten speichern, Erholungskurve nach Belastung, Wochen-/Monatsvergleich

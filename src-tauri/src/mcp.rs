//! Helio als MCP-Server für Claude (Claude-App, Claude Code, Seite „Fragen“). Nur lesend, nur fertige Auswertungen.
//!
//! Zwei Teile:
//! - `helio.exe --mcp` spricht MCP (JSON-RPC 2.0, eine Zeile je Nachricht) über stdin/stdout und reicht jeden Werkzeugaufruf
//!   über die Named Pipe `\\.\pipe\helio-mcp-<Benutzer>` an die laufende Helio weiter.
//! - Die laufende Helio beantwortet die Pipe (`serve`) mit ihrer eigenen Datenbank.
//!
//! Warum der Umweg: Programme, die die Claude-App (MSIX) startet, sehen %APPDATA% virtualisiert, also eine veraltete Kopie der
//! Datenbank. Die Pipe ist nicht virtualisiert, und eine Pipe erreicht anders als ein Port auf 127.0.0.1 keine Webseite.

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::daily::day_num;
use crate::health;

const PROTOCOL: &str = "2025-06-18";
const MAX_REQUEST: u64 = 64 * 1024;
const MAX_DAYS: i64 = 1100;

const INSTRUCTIONS: &str = "Helio wertet Nojos Apple-Health-Daten lokal aus (Schlaf, Puls, HRV, Atmung, Bewegung, Training, Ernährung, Gewicht). \
Die Werkzeuge liefern fertige, beschreibende Ergebnisse als JSON. Regeln: Belege jede Aussage mit einer Zahl aus den Werkzeugen. \
Keine Diagnosen, keine Therapie-, Medikamenten- oder Dosierungshinweise; Helio ist kein Medizinprodukt. Zusammenhänge sind keine Ursachen. \
Glukose und Insulin sind nicht enthalten (eigene App). Nachtwerte gehören zum Tag des Aufwachens. Uhrzeiten (onset, wake, mid, last_meal) \
sind Minuten seit Mitternacht (negativ = vor Mitternacht), Dauern in Minuten. Antworte auf Deutsch.";

const KEYS: &str = "Schlüssel (Einheit): sleep_min, inbed_min, deep_min, rem_min, core_min, awake_min, nap_min (Minuten); onset, wake, mid, \
last_meal (Minuten seit Mitternacht); efficiency (%); hr_night, hr_night_min, rhr, hr_avg, hr_min, hr_max, walk_hr (bpm); hrv (ms, SDNN); \
resp (Atemzüge/min); spo2, spo2_min (%); vo2max (ml/kg/min); steps, flights, workout_n (Anzahl); distance (m); active_kcal, basal_kcal, \
workout_kcal, energy (kcal); workout_min (Minuten); trimp (Trainingslast, Edwards); walk_speed (m/s); step_length (m); asymmetry, \
double_support, steadiness, bodyfat (%); protein, carbs, fat, sugar, fiber (g); caffeine, caffeine_late (mg); water (ml); alcohol (Getränke); \
weight (kg); bmi; headphone_db (dB); mindful_min (Minuten).";

fn date_arg(required: bool) -> Value {
    let mut s = json!({"type": "object", "properties": {"datum": {"type": "string", "description": "Tag JJJJ-MM-TT in Ortszeit; ohne Angabe: heute"}}});
    if required {
        s["required"] = json!(["datum"]);
    }
    s
}

pub fn tools() -> Value {
    json!([
        {"name": "zeitraum", "description": "Welche Tage und Kennzahlen es gibt: erster und letzter Tag, je Schlüssel Anzahl Tage und Zeitraum.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "heute", "description": "Seite Heute: letzte Nacht und letzter vollständiger Tag gegen den eigenen üblichen Bereich (P10–P90 der 60 Tage davor), Schlafschuld, Frühzeichen (Nachtwerte weichen gemeinsam ab wie oft vor einem Infekt) mit Rückblick auf markierte Krank-Tage.",
         "inputSchema": date_arg(false)},
        {"name": "belastung", "description": "Seite Belastung: Trainingslast (TRIMP), Fitness/Ermüdung (Banister), Pulserholung nach Trainings, Erholung nach harten Tagen.",
         "inputSchema": date_arg(false)},
        {"name": "rhythmus", "description": "Seite Rhythmus: Tagesrhythmus (Kosinor, IS/IV), Schlafregelmäßigkeit (SRI), sozialer Jetlag, Stundenprofile.",
         "inputSchema": date_arg(false)},
        {"name": "langzeit", "description": "Seite Langzeit: Wochen- und Monatsmittel, Trends über 52 Wochen (Theil-Sen, Mann-Kendall, FDR), Wendepunkte, 12 feste Fragen zu Zusammenhängen (Spearman, Permutationstest, FDR).",
         "inputSchema": date_arg(false)},
        {"name": "prognose", "description": "Prognose-Experiment (nicht belastbar): Ruhepuls und HRV der nächsten 7 Tage mit Rückwärtsprüfung über 90 Tage.",
         "inputSchema": date_arg(false)},
        {"name": "tag", "description": "Alle Kennzahlen eines Tages mit Quelle und Datenqualität (0 zu wenig, 1 wenige, 2 genug Daten).",
         "inputSchema": date_arg(true)},
        {"name": "kennzahl", "description": format!("Tageswerte einer Kennzahl von–bis (höchstens {MAX_DAYS} Tage), nur Tage mit Wert. {KEYS}"),
         "inputSchema": {"type": "object", "required": ["schluessel", "von", "bis"], "properties": {
            "schluessel": {"type": "string"}, "von": {"type": "string", "description": "JJJJ-MM-TT"}, "bis": {"type": "string", "description": "JJJJ-MM-TT"}}}},
    ])
}

/// Heutiges Datum in Ortszeit (die Oberfläche schickt es sonst mit; hier fragt Helio Windows).
#[cfg(windows)]
pub fn local_today() -> String {
    // SAFETY: GetLocalTime füllt nur die übergebene Struktur
    let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

fn day(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key).and_then(Value::as_str) {
        None => Ok(None),
        Some(d) if day_num(d).is_some() && d.len() == 10 => Ok(Some(d.to_string())),
        Some(d) => Err(format!("Ungültiges Datum „{d}“, erwartet JJJJ-MM-TT")),
    }
}

/// Ein Werkzeug ausführen. `today` nur für Aufrufe ohne `datum`.
pub fn call(c: &Connection, name: &str, args: &Value, today: &str) -> Result<Value, String> {
    let e = |x: rusqlite::Error| x.to_string();
    let d = day(args, "datum")?.unwrap_or_else(|| today.to_string());
    let v = match name {
        "zeitraum" => {
            let mut st = c.prepare("SELECT key, COUNT(*), MIN(day), MAX(day) FROM daily WHERE quality >= 1 GROUP BY key ORDER BY key").map_err(e)?;
            let keys: Vec<Value> = st
                .query_map([], |r| Ok(json!({"schluessel": r.get::<_, String>(0)?, "tage": r.get::<_, i64>(1)?, "von": r.get::<_, String>(2)?, "bis": r.get::<_, String>(3)?})))
                .map_err(e)?
                .collect::<Result<_, _>>()
                .map_err(e)?;
            json!({"bereich": crate::daily::range_of(c).map_err(e)?, "heute": today, "kennzahlen": keys})
        }
        "heute" => json!(crate::normal::today_of(c, &d).map_err(e)?),
        "belastung" => json!(crate::load::load_of(c, &d).map_err(e)?),
        "rhythmus" => json!(crate::rhythm::rhythm_of(c, &d).map_err(e)?),
        "langzeit" => json!(crate::longterm::longterm_of(c, &d).map_err(e)?),
        "prognose" => json!({"hinweis": "Experiment, nicht belastbar", "prognosen": crate::forecast::forecasts(c, &d).map_err(e)?}),
        "tag" => {
            let d = day(args, "datum")?.ok_or("datum fehlt")?;
            json!({"tag": d, "kennzahlen": crate::daily::day_of(c, &d).map_err(e)?})
        }
        "kennzahl" => {
            let key = args.get("schluessel").and_then(Value::as_str).ok_or("schluessel fehlt")?;
            let (from, to) = (day(args, "von")?.ok_or("von fehlt")?, day(args, "bis")?.ok_or("bis fehlt")?);
            let span = day_num(&to).zip(day_num(&from)).map(|(a, b)| a - b).unwrap_or(0);
            if !(0..MAX_DAYS).contains(&span) {
                return Err(format!("Zeitraum muss 1 bis {MAX_DAYS} Tage lang sein"));
            }
            let mut st = c.prepare("SELECT day, value, quality FROM daily WHERE key = ?1 AND day BETWEEN ?2 AND ?3 ORDER BY day").map_err(e)?;
            let rows: Vec<Value> = st
                .query_map(params![key, from, to], |r| Ok(json!([r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, u8>(2)?])))
                .map_err(e)?
                .collect::<Result<_, _>>()
                .map_err(e)?;
            json!({"schluessel": key, "von": from, "bis": to, "spalten": ["tag", "wert", "qualitaet"], "werte": rows})
        }
        _ => return Err(format!("Unbekanntes Werkzeug „{name}“")),
    };
    Ok(v)
}

fn pipe_name() -> String {
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!(r"\\.\pipe\helio-mcp-{user}")
}

/// In der laufenden Helio: Pipe mit der eigenen Datenbank beantworten.
#[cfg(windows)]
pub fn serve(app: AppHandle) {
    serve_with(std::sync::Arc::new(move |name: &str, args: &Value| call(&health::conn(&app)?, name, args, &local_today())));
}

type Handler = std::sync::Arc<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>;

/// Je Verbindung eine Anfrage `{"name","arguments"}` → `{"ok"}` oder `{"error"}`.
#[cfg(windows)]
fn serve_with(handler: Handler) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader as TokioReader};
    use tokio::net::windows::named_pipe::ServerOptions;
    tauri::async_runtime::spawn(async move {
        let name = pipe_name();
        // erste Instanz: läuft schon eine Helio-Pipe (fremd oder doppelt), nicht daneben lauschen
        let Ok(mut server) = ServerOptions::new().first_pipe_instance(true).create(&name) else { return };
        loop {
            let connected = server.connect().await.is_ok();
            // nach einem Fehler ist die Instanz unbrauchbar: immer eine neue anlegen
            let Ok(next) = ServerOptions::new().create(&name) else { return };
            let client = std::mem::replace(&mut server, next);
            if !connected {
                continue;
            }
            let handler = handler.clone();
            tauri::async_runtime::spawn(async move {
                let (r, mut w) = tokio::io::split(client);
                let mut line = String::new();
                if TokioReader::new(r.take(MAX_REQUEST)).read_line(&mut line).await.is_err() {
                    return;
                }
                let answer = tauri::async_runtime::spawn_blocking(move || {
                    let req: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                    let name = req.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                    let args = req.get("arguments").cloned().unwrap_or(json!({}));
                    handler(&name, &args)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                let out = match answer {
                    Ok(v) => json!({"ok": v}),
                    Err(e) => json!({"error": e}),
                };
                let _ = w.write_all(format!("{out}\n").as_bytes()).await;
                let _ = w.flush().await;
            });
        }
    });
}

/// Im `--mcp`-Prozess: eine Anfrage an die laufende Helio.
fn ask_helio(name: &str, args: &Value) -> Result<Value, String> {
    let mut pipe = None;
    // belegt (gerade eine andere Anfrage) → kurz warten
    for _ in 0..40 {
        match std::fs::OpenOptions::new().read(true).write(true).open(pipe_name()) {
            Ok(p) => {
                pipe = Some(p);
                break;
            }
            Err(e) if e.raw_os_error() == Some(231) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Err("Helio läuft nicht. Bitte Helio starten (sie läuft sonst im Infobereich der Taskleiste).".into()),
        }
    }
    let mut pipe = pipe.ok_or("Helio ist gerade beschäftigt, bitte gleich noch einmal.")?;
    pipe.write_all(format!("{}\n", json!({"name": name, "arguments": args})).as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(pipe).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    match (v.get("ok"), v.get("error").and_then(Value::as_str)) {
        (Some(ok), _) => Ok(ok.clone()),
        (_, Some(err)) => Err(err.to_string()),
        _ => Err("Leere Antwort von Helio".into()),
    }
}

/// Eine MCP-Nachricht beantworten; `None` bei Benachrichtigungen (ohne `id`).
pub fn handle(msg: &Value, call_tool: &dyn Fn(&str, &Value) -> Result<Value, String>) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let result = match method {
        "initialize" => {
            let v = msg.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL);
            Ok(json!({"protocolVersion": v, "capabilities": {"tools": {}},
                "serverInfo": {"name": "helio", "version": env!("CARGO_PKG_VERSION")}, "instructions": INSTRUCTIONS}))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or_default();
            let args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            // Werkzeugfehler gehören ins Ergebnis (isError), damit Claude sie lesen und anders fragen kann
            Ok(match call_tool(name, &args) {
                Ok(v) => json!({"content": [{"type": "text", "text": v.to_string()}]}),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        _ => Err(json!({"code": -32601, "message": format!("Unbekannte Methode {method}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

/// `helio.exe --mcp`: MCP über stdin/stdout, bis stdin endet.
pub fn run_stdio() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg, &ask_helio),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        health::migrate(&c).unwrap();
        for (d, v) in [("2026-11-16", 55.0), ("2026-11-17", 57.0)] {
            c.execute("INSERT INTO daily VALUES (?1, 'rhr', ?2, 'Zepp', 2)", params![d, v]).unwrap();
        }
        c
    }

    #[test]
    fn mcp_ablauf() {
        let fake = |name: &str, _: &Value| if name == "zeitraum" { Ok(json!({"ok": 1})) } else { Err("nein".to_string()) };
        let init = handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}), &fake).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "helio");
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), &fake).is_none());
        let list = handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), &fake).unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 8);
        let ok = handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "zeitraum", "arguments": {}}}), &fake).unwrap();
        assert_eq!(ok["result"]["content"][0]["text"], r#"{"ok":1}"#);
        let bad = handle(&json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "x"}}), &fake).unwrap();
        assert_eq!(bad["result"]["isError"], true);
        let unknown = handle(&json!({"jsonrpc": "2.0", "id": 5, "method": "resources/list"}), &fake).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }

    #[test]
    fn werkzeuge_lesen_und_pruefen_eingaben() {
        let c = db();
        let v = call(&c, "kennzahl", &json!({"schluessel": "rhr", "von": "2026-11-01", "bis": "2026-11-30"}), "2026-11-18").unwrap();
        assert_eq!(v["werte"], json!([["2026-11-16", 55.0, 2], ["2026-11-17", 57.0, 2]]));
        assert!(call(&c, "kennzahl", &json!({"schluessel": "rhr", "von": "2020-01-01", "bis": "2026-11-30"}), "2026-11-18").is_err());
        assert!(call(&c, "tag", &json!({"datum": "18.11.2026"}), "2026-11-18").is_err());
        assert!(call(&c, "tag", &json!({}), "2026-11-18").is_err());
        let z = call(&c, "zeitraum", &json!({}), "2026-11-18").unwrap();
        assert_eq!(z["kennzahlen"][0]["tage"], 2);
        // alle Werkzeuge laufen auf einer fast leeren Datenbank durch
        for t in tools().as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            let args = json!({"datum": "2026-11-18", "schluessel": "rhr", "von": "2026-11-01", "bis": "2026-11-18"});
            assert!(call(&c, name, &args, "2026-11-18").is_ok(), "{name}");
        }
    }
}

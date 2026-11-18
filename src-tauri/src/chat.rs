//! Seite „Fragen“: Fragen an Claude über das installierte Claude Code (Anmeldung mit Nojos Claude-Konto, nichts davon in Helio).
//!
//! Helio startet `claude -p` mit Ausgabe als JSON-Strom und gibt Claude nur Helios eigene Werkzeuge (mcp.rs): keine Dateien,
//! keine Shell, kein Web, keine anderen MCP-Server, keine Einstellungen. Was Claude abfragt, geht an Anthropic; darum
//! standardmäßig aus (Einstellung `chat`). Nur für Nojo selbst: Anthropic erlaubt die claude.ai-Anmeldung nicht in
//! weitergegebenen Apps; dafür bräuchte es einen API-Schlüssel.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
static SESSION: Mutex<Option<String>> = Mutex::new(None);

const SYSTEM: &str = "Du bist der Analyse-Assistent in Helio, Nojos lokaler Gesundheits-App. Nutze die Helio-Werkzeuge, \
um Fragen zu Schlaf, Puls, HRV, Atmung, Bewegung, Training, Ernährung und Gewicht zu beantworten. Belege jede Aussage mit Zahlen \
aus den Werkzeugen und nenne den Zeitraum. Keine Diagnosen, keine Therapie-, Medikamenten- oder Dosierungshinweise, keine Risiken; \
Zusammenhänge sind keine Ursachen. Wenn die Daten etwas nicht hergeben, sag das. Antworte auf Deutsch, knapp, mit kurzen Absätzen \
oder Listen.";

/// Claude Code finden: im PATH, sonst die üblichen Installationsorte (npm, nativer Installer).
pub fn find_claude() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut places: Vec<PathBuf> = std::env::split_paths(&path).map(|d| d.join("claude.exe")).collect();
    if let Some(a) = std::env::var_os("APPDATA") {
        places.push(PathBuf::from(a).join(r"npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe"));
    }
    if let Some(h) = std::env::var_os("USERPROFILE") {
        places.push(PathBuf::from(h).join(r".local\bin\claude.exe"));
    }
    places.into_iter().find(|p| p.is_file())
}

#[derive(Serialize)]
pub struct Status {
    enabled: bool,
    claude: bool,
    running: bool,
}

#[tauri::command]
pub fn chat_status(app: AppHandle) -> Status {
    Status {
        enabled: crate::load_config(&app).get("chat").and_then(Value::as_bool).unwrap_or(false),
        claude: find_claude().is_some(),
        running: CHILD.lock().unwrap().is_some(),
    }
}

#[tauri::command]
pub fn chat_enable(app: AppHandle, on: bool) {
    crate::set_config(&app, "chat", on.into());
    if !on {
        chat_stop();
    }
}

#[tauri::command]
pub fn chat_stop() {
    if let Some(mut c) = CHILD.lock().unwrap().take() {
        let _ = c.kill();
    }
}

#[tauri::command]
pub fn chat_reset() {
    chat_stop();
    *SESSION.lock().unwrap() = None;
}

/// Eine Zeile aus Claudes JSON-Strom in ein Ereignis für die Seite übersetzen.
pub fn event_of(m: &Value) -> Option<Value> {
    match m.get("type").and_then(Value::as_str)? {
        "stream_event" => {
            let d = m.pointer("/event/delta")?;
            let text = d.get("text").filter(|_| d.get("type").and_then(Value::as_str) == Some("text_delta"))?;
            Some(json!({"kind": "text", "text": text}))
        }
        "assistant" => {
            let tools: Vec<Value> = m
                .pointer("/message/content")?
                .as_array()?
                .iter()
                .filter(|c| c.get("type").and_then(Value::as_str) == Some("tool_use"))
                .map(|c| json!({"name": c.get("name").and_then(Value::as_str).unwrap_or_default().trim_start_matches("mcp__helio__"), "input": c.get("input")}))
                .collect();
            (!tools.is_empty()).then(|| json!({"kind": "tools", "tools": tools}))
        }
        "result" => {
            let error = m.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let text = m.get("result").and_then(Value::as_str).unwrap_or_default();
            let login = error && (text.contains("authenticate") || text.contains("401") || text.contains("login"));
            Some(json!({"kind": "done", "error": error, "text": if error { text } else { "" }, "login": login,
                "cost": m.get("total_cost_usd"), "session": m.get("session_id")}))
        }
        _ => None,
    }
}

fn args(cfg: &std::path::Path, session: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
        // keine eingebauten Werkzeuge, nur Helios Server, nichts aus Nojos Einstellungen
        "--tools", "", "--strict-mcp-config", "--allowedTools", "mcp__helio", "--permission-mode", "dontAsk", "--setting-sources", "",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.extend(["--mcp-config".into(), cfg.to_string_lossy().into_owned()]);
    a.extend(["--append-system-prompt".into(), format!("{SYSTEM} Heute ist der {} (Ortszeit).", crate::mcp::local_today())]);
    if let Some(s) = session {
        a.extend(["--resume".into(), s.into()]);
    }
    a
}

/// Frage stellen; Antwort kommt als Ereignisse `chat` ({kind: text | tools | done | error}).
#[tauri::command]
pub fn chat_ask(app: AppHandle, question: String) -> Result<(), String> {
    if !chat_status(app.clone()).enabled {
        return Err("Fragen an Claude sind ausgeschaltet.".into());
    }
    let question = question.trim().to_string();
    if question.is_empty() || question.chars().count() > 4000 {
        return Err("Die Frage ist leer oder zu lang.".into());
    }
    let claude = find_claude().ok_or("Claude Code ist nicht installiert.")?;
    let mut slot = CHILD.lock().unwrap();
    if slot.is_some() {
        return Err("Claude antwortet noch.".into());
    }
    // eigener leerer Ordner, damit Claude Code keine Projektdateien findet
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("chat");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cfg = dir.join("mcp.json");
    std::fs::write(&cfg, json!({"mcpServers": {"helio": {"command": exe, "args": ["--mcp"]}}}).to_string()).map_err(|e| e.to_string())?;

    let mut cmd = Command::new(claude);
    cmd.args(args(&cfg, SESSION.lock().unwrap().as_deref())).current_dir(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // aus einer Claude-Sitzung heraus gestartet (Entwicklung) erbt Helio deren Anmeldung; die gilt hier nicht
    for (k, _) in std::env::vars().filter(|(k, _)| k == "CLAUDECODE" || k.starts_with("CLAUDE_CODE_")) {
        cmd.env_remove(k);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: kein Konsolenfenster
    }
    let mut child = cmd.spawn().map_err(|e| format!("Claude Code startet nicht: {e}"))?;
    // Frage über stdin: keine Probleme mit Anführungszeichen in Argumenten
    if let Some(mut i) = child.stdin.take() {
        let _ = i.write_all(question.as_bytes());
    }
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let pid = child.id();
    *slot = Some(child);
    drop(slot);

    let tail = std::thread::spawn(move || {
        let lines: Vec<String> = BufReader::new(err).lines().map_while(Result::ok).collect();
        lines[lines.len().saturating_sub(5)..].join("\n")
    });
    std::thread::spawn(move || {
        let mut done = false;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(ev) = event_of(&m) {
                if ev["kind"] == "done" {
                    done = true;
                    if let Some(s) = ev["session"].as_str() {
                        *SESSION.lock().unwrap() = Some(s.to_string());
                    }
                }
                let _ = app.emit("chat", ev);
            }
        }
        // nur den eigenen Prozess abholen; nach Stopp kann schon die nächste Frage laufen
        let mine = {
            let mut g = CHILD.lock().unwrap();
            if g.as_ref().map(Child::id) == Some(pid) { g.take() } else { None }
        };
        let status = mine.and_then(|mut c| c.wait().ok());
        let tail = tail.join().unwrap_or_default();
        if !done {
            // abgebrochen (Stopp) oder Claude Code beendet sich ohne Ergebnis
            let stopped = status.is_none();
            let _ = app.emit("chat", json!({"kind": "error", "stopped": stopped, "text": if stopped { String::new() } else { tail }}));
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strom_wird_zu_ereignissen() {
        let t = json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "Hallo"}}});
        assert_eq!(event_of(&t).unwrap(), json!({"kind": "text", "text": "Hallo"}));
        let tool = json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "x"}, {"type": "tool_use", "name": "mcp__helio__heute", "input": {}}]}});
        assert_eq!(event_of(&tool).unwrap()["tools"][0]["name"], "heute");
        // ganze Textnachrichten nicht doppelt (kommen schon als Stücke)
        assert!(event_of(&json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "x"}]}})).is_none());
        let fail = json!({"type": "result", "is_error": true, "result": "Failed to authenticate. API Error: 401", "session_id": "s"});
        let ev = event_of(&fail).unwrap();
        assert_eq!((ev["login"].as_bool(), ev["session"].as_str()), (Some(true), Some("s")));
        assert!(event_of(&json!({"type": "system", "subtype": "init"})).is_none());
    }

    #[test]
    fn claude_bekommt_nur_helio() {
        let a = args(std::path::Path::new("C:/x/mcp.json"), Some("abc"));
        let pos = |f: &str| a.iter().position(|x| x == f).unwrap();
        assert_eq!(a[pos("--tools") + 1], "");
        assert_eq!(a[pos("--allowedTools") + 1], "mcp__helio");
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(a[pos("--resume") + 1], "abc");
    }
}

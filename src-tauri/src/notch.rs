//! Puls als Live Activity in Nojos Notch (http://127.0.0.1:47800, siehe D:\Dev\notch\README.md).
//! Eigener Faden, nimmt immer nur den neuesten Wert — haengt die Notch, staut sich nichts.
//! Dazu „Heute“: die auffälligsten Abweichungen des Tages (Auswahl in src/Today.tsx).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

const ID: &str = "helio-hr";
const TODAY_ID: &str = "helio-today";
/// Symbole aus der Bibliothek (src/nojo/icons, Quelle nojo-design/assets/icons.mjs)
const HEART_SVG: &str = include_str!("../../src/nojo/icons/heart.svg");
const HEART_COLOR: &str = "#ff5a6e";
const SUN_SVG: &str = include_str!("../../src/nojo/icons/sun.svg");
/// Helios Akzent (--n-accent), bewusst keine Warnfarbe: „Heute“ beschreibt nur
const TODAY_COLOR: &str = "#75b8ff";

fn icon(svg: &str, color: &str) -> String {
    let svg = svg.trim().replace("currentColor", color);
    let mut out = String::from("data:image/svg+xml;utf8,");
    for c in svg.chars() {
        match c {
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '"' => out.push('\''),
            '\n' | '\r' => {}
            c => out.push(c),
        }
    }
    out
}

pub static ENABLED: AtomicBool = AtomicBool::new(true);
static LATEST: Mutex<Option<u16>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static EXE: Mutex<String> = Mutex::new(String::new());

fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 47800))
}

fn request(method: &str, path: &str, body: &str) -> std::io::Result<()> {
    let t = Duration::from_millis(400);
    let mut s = TcpStream::connect_timeout(&addr(), t)?;
    s.set_read_timeout(Some(t))?;
    s.set_write_timeout(Some(t))?;
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut buf = [0u8; 128];
    let _ = s.read(&mut buf);
    Ok(())
}

pub fn spawn() {
    if let Ok(p) = std::env::current_exe() {
        *EXE.lock().unwrap() = p.to_string_lossy().into_owned();
    }
    std::thread::spawn(|| loop {
        let bpm = {
            let mut g = LATEST.lock().unwrap();
            while g.is_none() {
                g = WAKE.wait(g).unwrap();
            }
            g.take().unwrap()
        };
        if !ENABLED.load(Ordering::Relaxed) {
            continue;
        }
        let body = serde_json::json!({
            "id": ID,
            "app": "Helio",
            "title": "Puls",
            "subtitle": "Helio Strap · live",
            "value": bpm.to_string(),
            "unit": "bpm",
            // Notch laesst das Symbol in diesem Takt schlagen
            "pulse": bpm,
            "icon": icon(HEART_SVG, HEART_COLOR),
            "color": HEART_COLOR,
            "ttl": 15,
            "open": *EXE.lock().unwrap(),
        })
        .to_string();
        let _ = request("POST", "/activity", &body);
    });
}

pub fn push(bpm: u16) {
    *LATEST.lock().unwrap() = Some(bpm);
    WAKE.notify_one();
}

pub fn remove() {
    let _ = request("DELETE", &format!("/activity/{ID}"), "");
    let _ = request("DELETE", &format!("/activity/{TODAY_ID}"), "");
}

/// „Heute“ setzen; `text: None` = nichts auffällig, Eintrag weg. `alert` klappt die Notch kurz auf.
/// Eigener App-Name: `app: "Helio"` ordnet die Notch dem Puls zu (src/compact.ts `sourceOf`).
/// Antwort `false`: Notch im Tray abgeschaltet, nichts geschickt.
#[tauri::command]
pub fn notch_today(text: Option<String>, alert: bool) -> bool {
    if !ENABLED.load(Ordering::Relaxed) {
        return false;
    }
    std::thread::spawn(move || {
        let _ = match text {
            None => request("DELETE", &format!("/activity/{TODAY_ID}"), ""),
            Some(t) => request(
                "POST",
                "/activity",
                &serde_json::json!({
                    "id": TODAY_ID,
                    "app": "Helio Heute",
                    "title": "Heute",
                    "subtitle": t,
                    "icon": icon(SUN_SVG, TODAY_COLOR),
                    "color": TODAY_COLOR,
                    // Helio frischt bei jedem Abgleich (5 Min.) auf; läuft Helio nicht mehr, verschwindet es
                    "ttl": 3600,
                    "alert": alert,
                    "open": *EXE.lock().unwrap(),
                })
                .to_string(),
            ),
        };
    });
    true
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    if !on {
        remove();
    }
}

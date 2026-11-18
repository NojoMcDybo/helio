//! Bluetooth: findet das Helio Strap, abonniert die Standard-Herzfrequenz (0x2A37)
//! und verbindet sich selbst neu, wenn die Verbindung abreisst.
//!
//! Das iPhone bleibt dabei mit dem Band verbunden — "Heart Rate Push" in Zepp muss an sein.
//! Wichtig: auf PC-Seite darf nur EIN Programm verbunden sein, sonst wirbt das Band nicht mehr
//! und ist fuer alle anderen unsichtbar. Darum gibt Helio die Verbindung beim Beenden frei.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const HR_SERVICE: Uuid = Uuid::from_u128(0x0000180d_0000_1000_8000_00805f9b34fb);
const HR_MEASUREMENT: Uuid = Uuid::from_u128(0x00002a37_0000_1000_8000_00805f9b34fb);
const NAME_HINT: &str = "Helio";

/// Kommen so lange keine Werte, gilt die Verbindung als tot.
const STALE: Duration = Duration::from_secs(12);

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Serialize, Clone, Debug)]
pub struct Status {
    /// "start" | "suche" | "verbinde" | "live" | "kein-adapter"
    pub state: String,
    pub device: Option<String>,
    pub detail: Option<String>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
static CURRENT: Mutex<Option<Peripheral>> = Mutex::new(None);

pub fn status() -> Status {
    STATUS.lock().unwrap().clone().unwrap_or(Status { state: "start".into(), device: None, detail: None })
}

fn set_status(app: &AppHandle, state: &str, device: Option<String>, detail: Option<String>) {
    let s = Status { state: state.into(), device, detail };
    *STATUS.lock().unwrap() = Some(s.clone());
    let _ = app.emit("status", s);
}

#[derive(Serialize, Clone)]
pub struct Sample {
    pub ts: i64,
    pub bpm: u16,
}

/// Heart Rate Measurement laut Bluetooth-Spezifikation (nur BPM; das Band schickt keine RR-Intervalle).
fn parse(data: &[u8]) -> Option<u16> {
    let flags = *data.first()?;
    if flags & 0x01 != 0 {
        Some(u16::from_le_bytes([*data.get(1)?, *data.get(2)?]))
    } else {
        data.get(1).map(|b| *b as u16)
    }
}

pub fn spawn(app: AppHandle, on_sample: fn(&AppHandle, Sample)) {
    tauri::async_runtime::spawn(async move {
        let manager = loop {
            match Manager::new().await {
                Ok(m) => break m,
                Err(e) => {
                    set_status(&app, "kein-adapter", None, Some(e.to_string()));
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        };
        loop {
            match adapter(&manager).await {
                Some(a) => {
                    if let Err(e) = session(&app, &a, on_sample).await {
                        eprintln!("[helio] {e}");
                    }
                    *CURRENT.lock().unwrap() = None;
                    set_status(&app, "suche", None, None);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                None => {
                    set_status(&app, "kein-adapter", None, None);
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    });
}

async fn adapter(m: &Manager) -> Option<Adapter> {
    m.adapters().await.ok()?.into_iter().next()
}

/// Sucht, bis das Band auftaucht (Name enthaelt "Helio" oder es wirbt mit dem HR-Service).
async fn find(app: &AppHandle, a: &Adapter) -> Res<Peripheral> {
    set_status(app, "suche", None, None);
    a.start_scan(ScanFilter::default()).await?;
    let found = loop {
        let mut hit = None;
        for p in a.peripherals().await? {
            if let Ok(Some(props)) = p.properties().await {
                let name = props.local_name.unwrap_or_default();
                if name.contains(NAME_HINT) || props.services.contains(&HR_SERVICE) {
                    hit = Some(p);
                    break;
                }
            }
        }
        if let Some(p) = hit {
            break p;
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    };
    let _ = a.stop_scan().await;
    Ok(found)
}

async fn session(app: &AppHandle, a: &Adapter, on_sample: fn(&AppHandle, Sample)) -> Res<()> {
    let p = find(app, a).await?;
    let name = p.properties().await?.and_then(|x| x.local_name);
    set_status(app, "verbinde", name.clone(), None);
    if !p.is_connected().await? {
        tokio::time::timeout(Duration::from_secs(20), p.connect()).await??;
    }
    p.discover_services().await?;
    let ch = p
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == HR_MEASUREMENT)
        .ok_or("kein Herzfrequenz-Service – ist Heart Rate Push in Zepp an?")?;
    p.subscribe(&ch).await?;
    let mut notes = p.notifications().await?;
    *CURRENT.lock().unwrap() = Some(p.clone());
    set_status(app, "live", name, None);

    let mut last = Instant::now();
    loop {
        match tokio::time::timeout(Duration::from_secs(2), notes.next()).await {
            Ok(Some(n)) if n.uuid == HR_MEASUREMENT => {
                if let Some(bpm) = parse(&n.value) {
                    last = Instant::now();
                    if bpm > 0 {
                        on_sample(app, Sample { ts: crate::store::now_ms(), bpm });
                    }
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {}
        }
        if last.elapsed() > STALE {
            break;
        }
    }
    let _ = p.disconnect().await;
    Ok(())
}

/// Beim Beenden: Verbindung freigeben, damit das Band sofort wieder fuer andere sichtbar ist.
pub fn shutdown() {
    let p = CURRENT.lock().unwrap().take();
    if let Some(p) = p {
        let _ = tauri::async_runtime::block_on(async {
            tokio::time::timeout(Duration::from_secs(2), p.disconnect()).await
        });
    }
}

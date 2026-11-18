mod ble;
mod chat;
mod daily;
mod forecast;
mod health;
mod load;
mod longterm;
mod mcp;
mod normal;
mod notch;
mod rhythm;
mod signal;
mod stats;
mod store;
mod update;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

static CLEANED: AtomicBool = AtomicBool::new(false);

fn config_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("config.json"))
}

pub(crate) fn load_config(app: &AppHandle) -> serde_json::Value {
    config_file(app)
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn save_config(app: &AppHandle, v: &serde_json::Value) {
    if let Some(f) = config_file(app) {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, v.to_string());
    }
}

pub(crate) fn set_config(app: &AppHandle, key: &str, val: serde_json::Value) {
    let mut v = load_config(app);
    v[key] = val;
    save_config(app, &v);
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn on_sample(app: &AppHandle, s: ble::Sample) {
    store::insert(s.ts, s.bpm);
    notch::push(s.bpm);
    let _ = app.emit("hr", s);
}

#[tauri::command]
fn snapshot() -> serde_json::Value {
    serde_json::json!({
        "status": ble::status(),
        "notch": notch::ENABLED.load(Ordering::Relaxed),
    })
}

fn cleanup() {
    if CLEANED.swap(true, Ordering::Relaxed) {
        return;
    }
    notch::remove();
    chat::chat_stop();
    ble::shutdown();
}

/// `helio.exe --mcp` (main.rs)
pub fn mcp_stdio() {
    mcp::run_stdio()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hidden = std::env::args().any(|a| a == "--hidden");
    let app = tauri::Builder::default()
        // Nur eine Helio darf laufen: zwei Programme am Band = Band unsichtbar
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            update::update_state,
            update::update_check,
            update::update_install,
            snapshot,
            store::series,
            store::stats,
            store::range,
            health::health_status,
            health::health_sync,
            daily::daily_day,
            daily::daily_range,
            normal::today_view,
            load::load_view,
            rhythm::rhythm_view,
            longterm::longterm_view,
            notch::notch_today,
            forecast::forecast_view,
            signal::marks,
            signal::mark_sick,
            chat::chat_status,
            chat::chat_enable,
            chat::chat_ask,
            chat::chat_stop,
            chat::chat_reset
        ])
        .setup(move |app| {
            let h = app.handle().clone();
            let dir = app.path().app_data_dir()?;
            store::open(&dir)?;
            update::spawn(app.handle().clone());

            let cfg = load_config(&h);
            let notch_on = cfg.get("notch").and_then(|v| v.as_bool()).unwrap_or(true);
            notch::ENABLED.store(notch_on, Ordering::Relaxed);

            // Autostart beim ersten Start des installierten Programms einschalten (nicht im Dev-Build)
            if !cfg!(debug_assertions) && cfg.get("autostart_init").is_none() {
                let _ = app.autolaunch().enable();
                set_config(&h, "autostart_init", true.into());
            }
            let auto_on = app.autolaunch().is_enabled().unwrap_or(false);

            let open = MenuItem::with_id(app, "open", "Helio öffnen", true, None::<&str>)?;
            let notch_item = CheckMenuItem::with_id(app, "notch", "In der Notch zeigen", true, notch_on, None::<&str>)?;
            let auto_item = CheckMenuItem::with_id(app, "auto", "Mit Windows starten", true, auto_on, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Helio beenden", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &notch_item, &auto_item, &sep, &quit])?;
            let notch_item2 = notch_item.clone();
            let auto_item2 = auto_item.clone();
            TrayIconBuilder::with_id("helio")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Helio")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, e| match e.id().as_ref() {
                    "open" => show(app),
                    "notch" => {
                        let on = notch_item2.is_checked().unwrap_or(true);
                        notch::set_enabled(on);
                        set_config(app, "notch", on.into());
                        let _ = app.emit("notch", on);
                    }
                    "auto" => {
                        let on = auto_item2.is_checked().unwrap_or(false);
                        let al = app.autolaunch();
                        let _ = if on { al.enable() } else { al.disable() };
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;

            // Schliessen = in den Tray, die Verbindung zum Band laeuft weiter
            if let Some(w) = app.get_webview_window("main") {
                let w2 = w.clone();
                w.on_window_event(move |ev| {
                    if let WindowEvent::CloseRequested { api, .. } = ev {
                        api.prevent_close();
                        let _ = w2.hide();
                    }
                });
                if !hidden {
                    let _ = w.show();
                }
            }

            #[cfg(windows)]
            mcp::serve(h.clone());
            notch::spawn();
            health::spawn(h.clone());
            ble::spawn(h, on_sample);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Helio konnte nicht starten");

    app.run(|_app, ev| {
        if let RunEvent::Exit = ev {
            cleanup();
        }
    });
}

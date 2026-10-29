#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use hub_core::{domain::*, telegram, Hub};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};
use tauri::{Emitter, Manager, State};
type H<'a> = State<'a, Arc<Hub>>;
type R<T> = Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
#[tauri::command]
fn set_window_theme(window: tauri::WebviewWindow, dark: bool) -> R<()> {
    if window.label() != "main" { return Err("Unknown window".into()); }
    window.set_theme(Some(if dark { tauri::Theme::Dark } else { tauri::Theme::Light })).map_err(err)?;
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};
        let hwnd = window.hwnd().map_err(err)?.0 as windows_sys::Win32::Foundation::HWND;
        // COLORREF is 0x00BBGGRR. DWM owns the real caption buttons and hit testing.
        let caption: u32 = if dark { 0x0018140d } else { 0x00f8f8f5 };
        let text: u32 = if dark { 0x00f1efe9 } else { 0x00352b17 };
        unsafe {
            // Older Windows versions may not support custom caption colours;
            // they retain the native light/dark titlebar set above.
            DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR as u32, &caption as *const _ as _, 4);
            DwmSetWindowAttribute(hwnd, DWMWA_TEXT_COLOR as u32, &text as *const _ as _, 4);
        }
    }
    Ok(())
}
#[tauri::command]
async fn snapshot(h: H<'_>) -> R<Snapshot> {
    h.snapshot().await.map_err(err)
}
#[tauri::command]
async fn discover(h: H<'_>) -> R<Vec<Installation>> {
    h.discover().await.map_err(err)
}
#[tauri::command]
async fn enable_agent(
    h: H<'_>,
    provider: Provider,
    enabled: bool,
    accept_unsandboxed: bool,
) -> R<()> {
    h.enable(provider, enabled, accept_unsandboxed)
        .await
        .map_err(err)
}
#[tauri::command]
async fn add_project(h: H<'_>, name: String, root: PathBuf) -> R<Project> {
    h.add_project(name, root).await.map_err(err)
}
#[tauri::command]
async fn create_session(
    h: H<'_>,
    project_id: String,
    provider: Provider,
    title: String,
    model: Option<String>,
) -> R<Session> {
    h.create_session(project_id, provider, title, model)
        .await
        .map_err(err)
}
#[tauri::command]
async fn import_native(h: H<'_>, session_id: String, native_id: String) -> R<()> {
    h.import_native(session_id, native_id).await.map_err(err)
}
#[tauri::command]
async fn submit_turn(h: H<'_>, session_id: String, text: String, assets: Vec<Asset>) -> R<Turn> {
    h.inner()
        .submit(session_id, text, assets, None)
        .await
        .map_err(err)
}
#[tauri::command]
async fn cancel_turn(h: H<'_>, turn_id: String) -> R<()> {
    h.cancel(&turn_id).await.map_err(err)
}
#[tauri::command]
async fn steer_turn(h: H<'_>, turn_id: String, text: String) -> R<()> {
    h.steer(&turn_id, text).await.map_err(err)
}
#[tauri::command]
async fn answer_request(h: H<'_>, request_id: String, decision: String, answers: Value) -> R<()> {
    h.answer(request_id, decision, answers, None)
        .await
        .map_err(err)
}
#[tauri::command]
async fn history(h: H<'_>, session_id: String, after: i64) -> R<Vec<Event>> {
    h.history(session_id, after).await.map_err(err)
}
#[tauri::command]
async fn export_log(h: H<'_>, session_id: String) -> R<PathBuf> {
    h.export_log(session_id).await.map_err(err)
}
#[tauri::command]
async fn audit(h: H<'_>) -> R<Vec<Value>> {
    h.audit_rows().await.map_err(err)
}
#[tauri::command]
async fn configure_telegram(h: H<'_>, token: String, user_id: i64, pin: String) -> R<()> {
    h.setup_telegram(token, user_id, pin).await.map_err(err)
}
#[tauri::command]
async fn connect_telegram(h: H<'_>) -> R<()> {
    telegram::start(h.inner().clone()).await.map_err(err)
}
#[tauri::command]
async fn disconnect_telegram(h: H<'_>) -> R<()> {
    telegram::disconnect(h.inner()).await.map_err(err)
}
#[tauri::command]
async fn revoke_access(h: H<'_>) -> R<()> {
    h.revoke().await.map_err(err)
}
#[tauri::command]
async fn emergency_stop(h: H<'_>) -> R<()> {
    h.emergency_stop().await.map_err(err)
}
#[tauri::command]
async fn stage_file(h: H<'_>, session_id: String, path: PathBuf) -> R<Asset> {
    hub_core::security::reject_links(&path).map_err(err)?;
    let meta = tokio::fs::metadata(&path).await.map_err(err)?;
    if !meta.is_file() || meta.len() > 20 * 1024 * 1024 {
        return Err("Лимит вложения 20 MiB".into());
    }
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let bytes = tokio::fs::read(path).await.map_err(err)?;
    h.stage(session_id, name, bytes).await.map_err(err)
}
#[tauri::command]
fn capabilities(provider: Provider) -> Value {
    hub_core::adapter::capabilities(&provider)
}
#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> R<()> {
    use tauri_plugin_autostart::ManagerExt;
    let a = app.autolaunch();
    if enabled { a.enable() } else { a.disable() }.map_err(err)
}
#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> R<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(err)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--background"])
                .build(),
        )
        .setup(|app| {
            let dir = PathBuf::from(std::env::var("LOCALAPPDATA")?).join("AgentHub");
            let hub = tauri::async_runtime::block_on(Hub::open(dir))?;
            app.manage(hub.clone());
            let reconnect = hub.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = telegram::restore(reconnect.clone()).await {
                    *reconnect.telegram_status.write().await = e.to_string();
                }
            });
            let handle = app.handle().clone();
            let mut events = hub.events.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(e) => {
                            let _ = handle.emit("hub-event", e);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let _ = handle.emit("hub-resync", ());
                        }
                        Err(_) => break,
                    }
                }
            });
            use tauri::menu::{Menu, MenuItem};
            let open = MenuItem::with_id(app, "open", "Открыть Agent Hub", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Остановить все задачи", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Выйти", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &stop, &quit])?;
            let icon = tauri::image::Image::new_owned(
                include_bytes!("../icons/tray-face.rgba").to_vec(), 32, 32,
            );
            tauri::tray::TrayIconBuilder::new()
                .icon(icon)
                .tooltip("Agent Hub")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "stop" => {
                        let h = app.state::<Arc<Hub>>().inner().clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = h.emergency_stop().await;
                        });
                    }
                    "quit" => {
                        let h = app.state::<Arc<Hub>>().inner().clone();
                        let a = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = telegram::stop(&h).await;
                            let _ = h.emergency_stop().await;
                            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                            a.exit(0);
                        });
                    }
                    _ => {}
                })
                .build(app)?;
            if std::env::args().any(|a| a == "--background") {
                if let Some(w) = app.get_webview_window("main") {
                    w.hide()?;
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_window_theme,
            snapshot,
            discover,
            enable_agent,
            add_project,
            create_session,
            import_native,
            submit_turn,
            cancel_turn,
            steer_turn,
            answer_request,
            history,
            export_log,
            audit,
            configure_telegram,
            connect_telegram,
            disconnect_telegram,
            revoke_access,
            emergency_stop,
            stage_file,
            capabilities,
            set_autostart,
            get_autostart
        ])
        .run(tauri::generate_context!())
        .expect("Agent Hub failed to start");
}

//! "Why am I blocked" app page: when the engine hides a blocked
//! app, it records a `blocked_app_notice`; this module shows it in
//! a fullscreen, always-on-top Tauri window (label `blocked-app`)
//! styled like the website blocked page, instead of the app just
//! silently vanishing.
//!
//! SAFETY: unlike the frozen overlay, this page is purely
//! informational and trivially dismissible — "Back to work" and
//! Escape both clear the notice, and the next tick destroys the
//! window. The sync NEVER creates the window without a fresh
//! notice, and a stale notice (older than 2 minutes) never shows
//! the page. The window belongs to Preelock's own process, and
//! `focus::protected` exempts Preelock in every form, so the block
//! logic can never hide this page itself. While a frozen session
//! runs, the frozen overlay owns the screen.

use tauri::Manager;

/// Tick sync for the blocked-app page, run on every 1-second tick
/// from the tray thread (see `run` in `lib.rs`), right after
/// `sync_frozen_overlay`.
pub fn sync_overlay(app: &tauri::AppHandle, snapshot: &crate::models::Snapshot) {
    let now = crate::clock::now();
    let fresh = snapshot
        .blocked_app_notice
        .as_ref()
        .is_some_and(|n| now.saturating_sub(n.at_ms) < 120_000);
    let frozen_running = snapshot.sessions.iter().any(|s| {
        crate::frozen::is_enforced(
            s.focus.frozen.enabled,
            s.status == crate::models::Status::Running,
        ) && !crate::notifications::grace_active(s.grace_until_ms, now)
    });
    let window = app.get_webview_window("blocked-app");
    if fresh && !frozen_running {
        match window {
            None => {
                let result = tauri::WebviewWindowBuilder::new(
                    app,
                    "blocked-app",
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .title("Preelock — Blocked")
                .fullscreen(true)
                .always_on_top(true)
                .decorations(false)
                .skip_taskbar(true)
                .build();
                if let Err(e) = result {
                    log::warn!("Could not open the blocked-app page: {e}");
                }
            }
            Some(w) => {
                let hidden = !w.is_visible().unwrap_or(true);
                let minimized = w.is_minimized().unwrap_or(false);
                if hidden || minimized {
                    let _ = w.unminimize();
                    let _ = w.show();
                }
            }
        }
    } else if let Some(w) = window {
        let _ = w.destroy();
        if app.get_webview_window("blocked-app").is_some() {
            let _ = w.hide();
        }
    }
}

/// Dismiss the "why am I blocked" app page. The page's "Back to
/// work" button and the Escape key both call this; it clears the
/// notice so the tick sync destroys the window. The block itself
/// is untouched.
#[tauri::command]
pub async fn dismiss_blocked_app_page(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        app.state::<crate::ipc::IpcClient>()
            .dismiss_blocked_app_page()
    })
    .await
    .map_err(|e| e.to_string())?
}

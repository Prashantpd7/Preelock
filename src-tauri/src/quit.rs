//! M3: state-dependent quit rules.
//!
//! One decision table, shared by every quit path so they can never
//! drift apart:
//! - GUI tray menu "Quit Preelock" (`lib.rs` menu event),
//! - Cmd+Q / application-menu Quit / Dock Quit, on macOS via both the
//!   native quit guard (`desktop_settings.rs` → `crate::quit`) and
//!   Tauri's `ExitRequested` (`lib.rs` run loop),
//! - the main window's close request (`lib.rs` `on_window_event`).
//!   The `--tray` process menu needs no decision: it has no engine, so
//!   its idle-only Quit already exits just that process (`tray.rs`).
//!
//! The decision is driven by authoritative engine state, never by
//! UI-local state: a live session is Running, Paused, or Preparing in
//! the engine's snapshot (see `Engine::quit_decision`, which tries
//! the cross-process engine lock first and falls back to a fresh
//! read-only database load when another engine owns the lock).
//!
//! ```text
//! live session            -> CloseUiOnly: hide the main window only.
//!                            The GUI process, its tray icon, the
//!                            background engine, the browser bridge
//!                            and the camera agent all keep running.
//! no session, lock owner  -> ShutdownAndExit: shut the engine down
//!                            and exit the UI + tray. The separate
//!                            `--background` process is NOT stopped:
//!                            screen-time tracking and scheduled
//!                            sessions keep running there.
//! no session, observer    -> ExitUiOnly: exit only the GUI process.
//!                            Never shut the engine down — the owner
//!                            keeps enforcing and tracking. This is
//!                            the M1 transient fix: an observer that
//!                            used to report "not idle" (and show the
//!                            "Finish your focus session" notice for
//!                            a session that was not running) now
//!                            quits cleanly with no notice.
//! ```
//!
//! HONEST LIMIT: nothing here stops an administrator (or the user
//! via Activity Monitor / Task Manager / `kill`) from deliberately
//! terminating any Preelock process, including the independent
//! `--background` service. App-level code cannot intercept SIGKILL
//! or an admin kill, and claiming otherwise would be false. What
//! survives a kill is recovery, not prevention: the engine lock is
//! released by the OS on process death, the background engine takes
//! over on its next tick (M1), the tray reappears (M2), and the
//! session resumes from its checkpoint. A same-boot kill is NOT
//! recorded as a force quit: the desktop app is only a view of a
//! session the engine owns, so closing or quitting it never ends the
//! block and is not tampering. The OS
//! service registrations (LaunchAgent `KeepAlive` on macOS,
//! Run-key + `RegisterApplicationRestart` on Windows) restart the
//! background tracker after a kill; they do not — and cannot —
//! forbid the kill itself.

/// What a quit request should do. Decided by [`decide`] (pure) and
/// by `Engine::quit_decision` (which gathers the authoritative
/// inputs, then applies the shutdown side effect for one outcome).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitDecision {
    /// A session is live: close/hide only the main UI. Nothing
    /// exits, nothing is released, nothing is saved.
    CloseUiOnly,
    /// No live session and this engine owns the lock: shut the
    /// engine down, then exit the UI + tray.
    ShutdownAndExit,
    /// No live session but another engine owns the lock: exit only
    /// the GUI process. The engine is never shut down from here.
    ExitUiOnly,
}

/// The state-dependent quit table. `owns_engine_lock` is whether
/// this engine holds (or just acquired) the cross-process engine
/// lock; `has_active` is whether a session is live in the
/// authoritative snapshot for that ownership (reloaded on lock
/// acquisition, or a fresh database read for an observer).
pub fn decide(has_active: bool, owns_engine_lock: bool) -> QuitDecision {
    if has_active {
        QuitDecision::CloseUiOnly
    } else if owns_engine_lock {
        QuitDecision::ShutdownAndExit
    } else {
        QuitDecision::ExitUiOnly
    }
}

/// Authoritative GUI quit rule when `--background` is the sole session engine
/// owner: Cmd+Q / Quit exits ONLY the desktop UI process (`ExitUiOnly`),
/// leaving `--background` running. Wait before exiting only if background
/// is unreachable while a strict session is live (refuse fail-open exit).
/// Note: `--background` itself is responsible for relaunching the UI (~5s)
/// strictly when `has_active_strict` is true, and never when false.
pub fn gui_quit_action(background_reachable: bool, has_active_strict: bool) -> QuitDecision {
    if !background_reachable && has_active_strict {
        QuitDecision::CloseUiOnly
    } else {
        QuitDecision::ExitUiOnly
    }
}

/// M6: whether the main window is shown at GUI startup after an OS
/// shutdown / logout / restart.
///
/// macOS offers no API that tells an app it was launched as a login
/// item rather than by hand (`SMAppService` registers the launch but
/// reports no launch source, and a Finder/Dock launch has the same
/// parent as a login launch), so the decision uses the signal that
/// does exist: `recovered_after_os_shutdown` — true only when the
/// previous run ended via the OS shutdown path or the boot id
/// changed, never after an ordinary quit. That is the login-launch
/// case in practice: the login item fired after the machine came
/// back. The residual false positive is a manual launch as the
/// very first launch after a reboot — the window starts hidden
/// once, and a Dock click (macOS `Reopen`) or the tray's
/// "Show Preelock" item restores it.
///
/// - A live session always shows the window: the user returns to
///   their block, and the tray timer needs a window to open.
/// - A user who enabled the normal "Start Preelock at login"
///   preference (the Settings toggle, recorded separately from the
///   automatic session/schedule registration) always sees the
///   window: they asked for the app at login.
/// - Anything else after an OS restart with no live session starts
///   hidden: the background engine still tracks and fires
///   schedules, and the tray icon stays available. The window is
///   hidden, never destroyed, so it can be reopened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginWindowAction {
    /// Show the main window at startup (the normal case).
    ShowWindow,
    /// Start with the main window hidden (login launch, idle, and
    /// the user did not ask for the app at login).
    StartHidden,
}

/// The login-window table. `recovered_after_os_shutdown` is
/// `Engine::recovered_after_os_shutdown`; `user_login_pref` is
/// whether the user enabled the "Start Preelock at login"
/// preference themselves (see `settings::login_user_pref_enabled`),
/// not the automatic session/schedule registration.
pub fn login_window_action(
    has_active: bool,
    user_login_pref: bool,
    recovered_after_os_shutdown: bool,
) -> LoginWindowAction {
    if has_active || user_login_pref || !recovered_after_os_shutdown {
        LoginWindowAction::ShowWindow
    } else {
        LoginWindowAction::StartHidden
    }
}

/// What closing the main window should do. Closing the window
/// closes only the main UI in every case — it never exits the
/// process, the engine, or the tray — and the window is hidden
/// (not destroyed) so the tray timer click can reopen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCloseAction {
    /// Hide the main window; the process and tray keep running.
    HideWindow,
    /// The engine lock is poisoned and no session state could be
    /// read: keep the window visible and tell the user the session
    /// must finish first (safe direction — never hide the UI of a
    /// possibly-broken engine).
    ShowBlockedNotice,
}

/// The main-window close table. `engine_lock_available` is whether
/// the engine mutex could be locked to read session state at all.
pub fn close_action(has_active: bool, engine_lock_available: bool) -> WindowCloseAction {
    if has_active || engine_lock_available {
        WindowCloseAction::HideWindow
    } else {
        WindowCloseAction::ShowBlockedNotice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_window_action_matrix() {
        // Manual launch (not after an OS shutdown) always shows.
        assert_eq!(
            login_window_action(false, false, false),
            LoginWindowAction::ShowWindow
        );
        // After an OS restart with a live session: show.
        assert_eq!(
            login_window_action(true, false, true),
            LoginWindowAction::ShowWindow
        );
        // After an OS restart, idle, but the user enabled the
        // "Start Preelock at login" preference: show.
        assert_eq!(
            login_window_action(false, true, true),
            LoginWindowAction::ShowWindow
        );
        // After an OS restart, idle, no user preference: hidden.
        // The background engine still runs; the tray can reopen it.
        assert_eq!(
            login_window_action(false, false, true),
            LoginWindowAction::StartHidden
        );
    }

    #[test]
    fn quit_decision_matrix() {
        // Live session: UI only, whoever owns the engine.
        assert_eq!(decide(true, true), QuitDecision::CloseUiOnly);
        assert_eq!(decide(true, false), QuitDecision::CloseUiOnly);
        // Idle owner: shut down and exit.
        assert_eq!(decide(false, true), QuitDecision::ShutdownAndExit);
        // Idle observer (the M1 transient): exit the UI only — no
        // shutdown, no "finish your session" notice.
        assert_eq!(decide(false, false), QuitDecision::ExitUiOnly);
    }

    #[test]
    fn gui_quit_exits_ui_when_background_owns_session() {
        assert_eq!(gui_quit_action(true, true), QuitDecision::ExitUiOnly);
        assert_eq!(gui_quit_action(true, false), QuitDecision::ExitUiOnly);
        assert_eq!(gui_quit_action(false, false), QuitDecision::ExitUiOnly);
        assert_eq!(gui_quit_action(false, true), QuitDecision::CloseUiOnly);
    }

    #[test]
    fn window_close_hides_in_every_known_state() {
        // Live session, owner or observer: hide.
        assert_eq!(close_action(true, true), WindowCloseAction::HideWindow);
        assert_eq!(close_action(true, false), WindowCloseAction::HideWindow);
        // Idle with a readable engine: hide (close-to-tray).
        assert_eq!(close_action(false, true), WindowCloseAction::HideWindow);
        // Idle but the engine lock is poisoned: stay visible and
        // explain, rather than hiding a broken engine's window.
        assert_eq!(
            close_action(false, false),
            WindowCloseAction::ShowBlockedNotice
        );
    }
}

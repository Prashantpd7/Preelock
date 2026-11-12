//! M2: persistent menu-bar / system-tray process (`--tray`).
//!
//! The GUI owns the `focus-controller` tray icon while it is alive
//! (see `lib.rs`), but that icon dies with the GUI process. This
//! module is the second, independent owner: a `--tray` process of the
//! same binary that shows exactly the same icon and menu while the
//! GUI is down, and hands the icon back when the GUI returns.
//!
//! Ownership rules (exactly one icon at all times):
//! - One `tray.lock` flock (same pattern as `background.lock` /
//!   `engine.lock`) guards the tray process: a second `--tray`
//!   exits immediately, and the GUI waits for the lock to free
//!   before building its own tray on relaunch.
//! - The tray process never owns a session timer. It re-reads the
//!   live session from the engine's database every second (the
//!   engine is the single source of truth; `read_live_session`
//!   is a read-only `Database::load`, the same pattern the
//!   background process uses to decide whether attention is
//!   needed) and only renders what it finds. When the GUI is back
//!   (fresh `desktop-heartbeat`) it exits, so the GUI tray takes
//!   over with no duplicate icon.
//! - Spawned by the background process while ticking with an active
//!   session (covers force-quit / crash, where no exit handler runs,
//!   and schedules firing while the GUI is closed). It is never
//!   spawned while the GUI is alive. (M3 removed the second spawn
//!   path: Cmd+Q / Dock Quit during a session no longer exits the
//!   GUI process — it only hides the main window — so the GUI's own
//!   tray icon survives and no handover is needed. See `quit.rs`.)
//! - Menu behaviour is identical to the GUI tray: a live session
//!   shows only the timer item (clicking it reopens the GUI —
//!   debounced, and only while the GUI is actually down); idle
//!   shows only "Quit Preelock", which exits just this process.
//!   Quit rules while a session runs are the M3 rules (no Quit item
//!   then, in either tray); the `--tray` process itself never owns
//!   an engine, so its idle Quit can only ever exit this process.
//! - The process is menu-bar only: on macOS it hides from the Dock
//!   / Cmd-Tab (`platform::hide_from_dock`) and destroys any window
//!   the shared Tauri config created before the run loop starts,
//!   so no window ever appears.
//!
//! What this module deliberately does NOT do: tick the engine,
//! save the database, drive the attention agent, publish browser
//! policies, or own the bridge. All enforcement stays with the
//! background engine; the tray only displays its state.
use crate::{
    background,
    database::Database,
    models::{Session, Snapshot, Status},
};
use fs2::FileExt;
use std::{
    fs::File,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use tauri::Manager;

/// `--tray` mode: the persistent tray process. See `main.rs`.
pub const ARG: &str = "--tray";
const LOCK_FILE: &str = "tray.lock";
const TRAY_ID: &str = "focus-controller";
/// Ignore a second timer click within this long after spawning the
/// GUI, so a double-click cannot launch two GUI processes while the
/// first is still starting (and the desktop heartbeat is stale).
const SHOW_GUI_DEBOUNCE_MS: i64 = 5_000;
/// How long the GUI waits for a tray process to exit (and release
/// the lock) before building its own tray on relaunch.
const GUI_TAKEOVER_WAIT: Duration = Duration::from_secs(2);

pub fn is_invocation(args: &[String]) -> bool {
    args.iter().any(|arg| arg == ARG)
}

/// The held cross-process tray lock. Dropping it releases the flock,
/// so a force-killed tray hands the icon over with no cleanup step.
/// The lock file itself is never deleted: the flock, not the file's
/// existence, is the mutex.
pub struct TrayLock {
    _file: File,
}

impl TrayLock {
    /// Try to take the tray lock in `data_dir`. Returns `None` when
    /// another tray process already holds it. Never blocks.
    pub fn try_acquire(data_dir: &Path) -> Option<Self> {
        let file = File::create(data_dir.join(LOCK_FILE)).ok()?;
        if file.try_lock_exclusive().is_ok() {
            Some(Self { _file: file })
        } else {
            None
        }
    }
}

/// True while a tray process holds the lock (i.e. a tray icon already
/// exists somewhere). Probe-only: production paths use
/// `TrayLock::try_acquire` / `ensure_running`.
pub fn lock_held(data_dir: &Path) -> bool {
    let Ok(lock) = File::create(data_dir.join(LOCK_FILE)) else {
        return true;
    };
    if lock.try_lock_exclusive().is_ok() {
        let _ = lock.unlock();
        false
    } else {
        true
    }
}

/// Spawn the tray process if none is running. Fire-and-forget: the
/// child is detached (stdio null) and survives the spawner. A second
/// concurrent spawn is harmless — the loser exits on the lock.
/// M7: spawned from the GUI executable, not necessarily this
/// process's own executable — the background process that calls this
/// is the `PreelockBackground.exe` copy in the MSIX build, and
/// spawning that copy with `--tray` used to dispatch back into the
/// background (see `launch`); the GUI copy runs the tray from
/// either build.
pub fn ensure_running(data_dir: &Path) -> Result<(), String> {
    if lock_held(data_dir) {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = crate::launch::gui_executable(&exe);
    Command::new(&exe)
        .arg(ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start tray process: {e}"))
}

/// Wait up to `timeout` for a tray process to release the lock.
/// Returns true when the lock is free. Used by the GUI on relaunch:
/// the tray process exits within a second of the desktop heartbeat
/// going fresh, so a short wait avoids a duplicate icon.
pub fn wait_for_release(data_dir: &Path, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while lock_held(data_dir) {
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// The live session, if any — Running, Paused, or Preparing.
/// Same predicate as the engine's `has_active` and the GUI tray.
pub fn live_session(snapshot: &Snapshot) -> Option<&Session> {
    snapshot.sessions.iter().find(|s| {
        matches!(
            s.status,
            Status::Running | Status::Paused | Status::Preparing
        )
    })
}

/// Read the live session from the engine's database. Read-only: a
/// fresh `Database` connection + `load`, no writes, no engine. The
/// tray renders exactly what the engine last saved, so its countdown
/// is the engine's countdown, never its own. Returns `None` when
/// there is no database yet or no session is live.
pub fn read_live_session(data_dir: &Path) -> Option<Session> {
    let db = Database::open(&data_dir.join("preelock.sqlite")).ok()?;
    let snapshot = db.load().ok()?;
    live_session(&snapshot).cloned()
}

/// The tray timer line for one session, at wall-clock `now_ms`.
/// Step 14: a grace session shows its countdown, not its full
/// productive target — the block has not started enforcing yet.
/// (Moved unchanged from `lib.rs` so the GUI tray and the tray
/// process can never drift apart.)
pub fn timer_text(session: &Session, now_ms: i64) -> String {
    if session.status == Status::Running
        && crate::notifications::grace_active(session.grace_until_ms, now_ms)
    {
        let secs = crate::notifications::grace_remaining_secs(session.grace_until_ms, now_ms);
        return format!("Starting in {secs}s");
    }
    let secs = session.remaining_ms / 1000;
    let clock = format!("{:02}:{:02}", secs / 60, secs % 60);
    match session.status {
        Status::Paused => format!("Paused - {clock}"),
        Status::Preparing => "Starting session...".to_string(),
        _ => format!("{clock} left"),
    }
}

/// The tray tooltip text for Windows System Tray parity (and general tooltip representation):
/// During an active session: "Preelock · <timer_text>" (e.g. "Preelock · 25:00 left",
/// "Preelock · Paused - 25:00", "Preelock · Starting in 45s", "Preelock · Starting session...").
/// When idle: "Preelock".
pub fn tooltip_text(live: Option<&Session>, now_ms: i64) -> String {
    match live {
        Some(session) => format!("Preelock · {}", timer_text(session, now_ms)),
        None => "Preelock".to_string(),
    }
}

/// The tray menu: a live session shows only the timer item (no Quit
/// — same rule as the GUI tray); idle shows only "Quit Preelock".
/// (Moved unchanged from `lib.rs`.)
pub fn build_menu(
    app: &tauri::AppHandle,
    live: Option<&Session>,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    match live {
        Some(session) => {
            let timer = tauri::menu::MenuItem::with_id(
                app,
                "timer",
                timer_text(session, crate::clock::now()),
                true,
                None::<&str>,
            )?;
            tauri::menu::Menu::with_items(app, &[&timer])
        }
        None => {
            let quit =
                tauri::menu::MenuItem::with_id(app, "quit", "Quit Preelock", true, None::<&str>)?;
            tauri::menu::Menu::with_items(app, &[&quit])
        }
    }
}

/// Whether a timer click may (re)open the GUI: only while the GUI is
/// actually down, and not within the debounce window of a previous
/// spawn. `last_spawn_ms` is 0 when the GUI has never been spawned
/// from this tray process.
pub fn should_show_gui(desktop_alive: bool, last_spawn_ms: i64, now_ms: i64) -> bool {
    !desktop_alive && (last_spawn_ms == 0 || now_ms - last_spawn_ms >= SHOW_GUI_DEBOUNCE_MS)
}

/// Reopen the GUI (plain launch, no arguments). The GUI touches the
/// desktop heartbeat at startup, which is what makes this tray
/// process exit and hand the icon back.
/// M7: launched from the GUI executable — this tray process may
/// itself be running from the MSIX background copy, whose bare
/// launch runs the background, not the GUI.
fn show_gui() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = crate::launch::gui_executable(&exe);
    #[cfg(target_os = "macos")]
    if let Some(bundle) = crate::background::app_bundle_root(&exe) {
        return Command::new("/usr/bin/open")
            .arg("-n")
            .arg("-a")
            .arg(&bundle)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not reopen Preelock bundle: {e}"));
    }
    Command::new(&exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not reopen Preelock: {e}"))
}

/// The `--tray` process: one menu-bar / system-tray icon, no windows,
/// driven by the engine's database. Returns the process exit code.
/// A second tray process exits 0 on the lock; a process that cannot
/// find the data dir exits 1.
pub fn run() -> i32 {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let Ok(dir) = background::data_dir() else {
        return 1;
    };
    let Some(_lock) = TrayLock::try_acquire(&dir) else {
        log::warn!("Another tray process already owns the tray icon.");
        return 0;
    };
    let setup_dir = dir.clone();
    let result = tauri::Builder::default()
        .setup(move |app| {
            // Menu-bar only: no Dock tile / Cmd-Tab entry on macOS.
            crate::platform::hide_from_dock();
            // Destroy any window the shared Tauri config created:
            // this process shows no window at all. Setup runs before
            // the run loop starts, so no window is ever presented.
            for window in app.webview_windows().values() {
                let _ = window.destroy();
            }
            let initial_live = read_live_session(&setup_dir);
            let initial_menu = build_menu(app.handle(), initial_live.as_ref())?;
            let mut tray = tauri::tray::TrayIconBuilder::with_id(TRAY_ID)
                .tooltip("Preelock focus controller")
                .menu(&initial_menu)
                .on_menu_event({
                    let dir = setup_dir.clone();
                    let last_show = std::sync::Arc::new(std::sync::Mutex::new(0i64));
                    move |app, event| match event.id.as_ref() {
                        "timer" => {
                            let now = crate::clock::now();
                            let mut last = last_show.lock().unwrap_or_else(|e| e.into_inner());
                            if should_show_gui(background::desktop_alive(&dir), *last, now) {
                                *last = now;
                                drop(last);
                                if let Err(error) = show_gui() {
                                    log::warn!("{error}");
                                }
                            }
                        }
                        // Idle only: the menu has no Quit item while a
                        // session is live. Quitting here exits just
                        // this process; the background engine keeps
                        // enforcing (same as quitting the GUI idle).
                        "quit" => app.exit(0),
                        _ => {}
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            // 1-second refresh, mirroring the GUI tray thread: re-read
            // the engine's state, update the menu, and exit as soon
            // as the GUI is back so there is never a second icon.
            let handle = app.handle().clone();
            let thread_dir = setup_dir.clone();
            let mut tray_menu = initial_menu;
            let mut showing_timer = initial_live.is_some();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                if background::desktop_alive(&thread_dir) {
                    handle.exit(0);
                    return;
                }
                let live = read_live_session(&thread_dir);
                let active = live.is_some();
                if active != showing_timer {
                    if let Ok(next) = build_menu(&handle, live.as_ref()) {
                        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
                            if tray.set_menu(Some(next.clone())).is_ok() {
                                tray_menu = next;
                                showing_timer = active;
                            }
                        }
                    }
                } else if active {
                    if let (Some(session), Some(kind)) = (live.as_ref(), tray_menu.get("timer")) {
                        if let Some(item) = kind.as_menuitem() {
                            let _ = item.set_text(timer_text(session, crate::clock::now()));
                        }
                    }
                }
                #[cfg(target_os = "windows")]
                if let Some(tray) = handle.tray_by_id(TRAY_ID) {
                    let _ =
                        tray.set_tooltip(Some(&tooltip_text(live.as_ref(), crate::clock::now())));
                }
            });
            Ok(())
        })
        .build(crate::context());
    match result {
        Ok(app) => {
            app.run(|_app, _event| {});
            0
        }
        Err(error) => {
            log::error!("Tray process startup failed: {error}");
            1
        }
    }
}

/// How long the GUI waits for a tray process to hand the icon back.
/// Re-exported for the GUI setup; kept here so the timeout lives
/// with the lock it waits on.
pub fn gui_takeover_wait() -> Duration {
    GUI_TAKEOVER_WAIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Session, Status};

    fn test_session(status: Status, remaining_ms: u64) -> Session {
        Session {
            id: "test-session".into(),
            title: "Deep work".into(),
            duration_minutes: 25,
            remaining_ms,
            focused_ms: 0,
            allow_pause: true,
            status,
            started_at: None,
            ended_at: None,
            deadline_ms: None,
            focus: Default::default(),
            report: Default::default(),
            attention: Default::default(),
            force_quit: false,
            schedule_id: None,
            kind: crate::models::SessionKind::Focus,
            pomodoro: Default::default(),
            reward: Default::default(),
            lock: Default::default(),
            lock_state: Default::default(),
            policy_applied: false,
            browser_fail_closed: false,
            grace_until_ms: None,
            grace_notified: false,
            durability: Default::default(),
        }
    }

    #[test]
    fn lock_is_singleton_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = TrayLock::try_acquire(dir.path()).unwrap();
        assert!(TrayLock::try_acquire(dir.path()).is_none());
        assert!(lock_held(dir.path()));
        drop(first);
        assert!(!lock_held(dir.path()));
        assert!(TrayLock::try_acquire(dir.path()).is_some());
    }

    #[test]
    fn timer_text_counts_down() {
        let now = 1_000_000i64;
        assert_eq!(
            timer_text(&test_session(Status::Running, 90_000), now),
            "01:30 left"
        );
        assert_eq!(
            timer_text(&test_session(Status::Running, 5_000), now),
            "00:05 left"
        );
        assert_eq!(
            timer_text(&test_session(Status::Paused, 90_000), now),
            "Paused - 01:30"
        );
        assert_eq!(
            timer_text(&test_session(Status::Preparing, 90_000), now),
            "Starting session..."
        );
        let mut grace = test_session(Status::Running, 90_000);
        grace.grace_until_ms = Some(now + 42_000);
        assert_eq!(timer_text(&grace, now), "Starting in 42s");
    }

    #[test]
    fn live_session_ignores_finished_sessions() {
        let empty = Snapshot::default();
        assert!(live_session(&empty).is_none());
        let mut snapshot = Snapshot::default();
        snapshot.sessions.push(test_session(Status::Completed, 0));
        snapshot.sessions.push(test_session(Status::Ready, 90_000));
        assert!(live_session(&snapshot).is_none());
        snapshot
            .sessions
            .push(test_session(Status::Running, 90_000));
        assert_eq!(live_session(&snapshot).unwrap().status, Status::Running);
        snapshot.sessions.push(test_session(Status::Paused, 60_000));
        // First live session wins — the engine never has two live.
        assert_eq!(live_session(&snapshot).unwrap().remaining_ms, 90_000);
    }

    #[test]
    fn read_live_session_reads_the_engine_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preelock.sqlite");
        let mut db = Database::open(&path).unwrap();
        let mut snapshot = Snapshot::default();
        snapshot
            .sessions
            .push(test_session(Status::Running, 90_000));
        db.save(&snapshot).unwrap();
        let live = read_live_session(dir.path()).unwrap();
        assert_eq!(live.title, "Deep work");
        assert_eq!(live.remaining_ms, 90_000);
        // Session ended in the DB: the tray goes idle.
        let mut ended = Snapshot::default();
        ended.sessions.push(test_session(Status::Ended, 0));
        db.save(&ended).unwrap();
        assert!(read_live_session(dir.path()).is_none());
    }

    #[test]
    fn read_live_session_is_none_without_a_database() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_live_session(dir.path()).is_none());
        assert!(read_live_session(Path::new("/nonexistent-preelock-dir")).is_none());
    }

    #[test]
    fn should_show_gui_matrix() {
        let now = 1_000_000i64;
        // GUI down, never spawned: open it.
        assert!(should_show_gui(false, 0, now));
        // GUI alive: the GUI's own tray handles the click.
        assert!(!should_show_gui(true, 0, now));
        // Spawned 1 s ago: debounced.
        assert!(!should_show_gui(false, now - 1_000, now));
        // Spawned 6 s ago: allowed again.
        assert!(should_show_gui(false, now - 6_000, now));
    }

    #[test]
    fn wait_for_release_waits_for_the_holder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(wait_for_release(dir.path(), Duration::from_millis(100)));
        let lock = TrayLock::try_acquire(dir.path()).unwrap();
        assert!(!wait_for_release(dir.path(), Duration::from_millis(150)));
        let path = dir.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(lock);
        });
        assert!(wait_for_release(&path, Duration::from_secs(2)));
        handle.join().unwrap();
    }

    #[test]
    fn tray_arg_is_detected() {
        assert!(is_invocation(&["--tray".to_string()]));
        assert!(!is_invocation(&["--background".to_string()]));
        assert!(!is_invocation(&[]));
    }

    #[test]
    fn windows_tooltip_parity_matrix() {
        let now = 1_000_000i64;
        // 1. Idle tooltip
        assert_eq!(tooltip_text(None, now), "Preelock");

        // 2. Running active session countdown
        let running_session = test_session(Status::Running, 90_000);
        assert_eq!(
            tooltip_text(Some(&running_session), now),
            "Preelock · 01:30 left"
        );
        let running_session_countdown = test_session(Status::Running, 89_000);
        assert_eq!(
            tooltip_text(Some(&running_session_countdown), now),
            "Preelock · 01:29 left"
        );

        // 3. Paused session
        let paused_session = test_session(Status::Paused, 90_000);
        assert_eq!(
            tooltip_text(Some(&paused_session), now),
            "Preelock · Paused - 01:30"
        );

        // 4. Preparing session
        let preparing_session = test_session(Status::Preparing, 90_000);
        assert_eq!(
            tooltip_text(Some(&preparing_session), now),
            "Preelock · Starting session..."
        );

        // 5. Grace starting session
        let mut grace_session = test_session(Status::Running, 90_000);
        grace_session.grace_until_ms = Some(now + 30_000);
        assert_eq!(
            tooltip_text(Some(&grace_session), now),
            "Preelock · Starting in 30s"
        );
    }
}

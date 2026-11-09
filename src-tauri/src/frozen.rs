//! Step 12 Frozen Turkey: a full-computer lock for one session.
//!
//! A frozen session is an ordinary session whose timer is the whole
//! lock: it carries two allowlists and nothing else is open while it
//! runs. The enforcement itself is NOT new machinery — it reuses the
//! paths earlier steps built, so there is no parallel blocking system
//! to drift out of sync:
//!
//! * App blocking: `FocusConfig::blocked_sample` (step 4) treats a
//!   frozen session as a strict allowlist: every app except the
//!   frozen allowed-apps list is blocked through the same sampler
//!   path (hide, distracted time, interruption count). The paired
//!   browser is blocked too unless it is explicitly in the allowed
//!   apps list — there is no automatic browser exemption for frozen
//!   sessions, unlike ordinary sessions. Preelock itself and the
//!   essential system processes stay open because `focus::protected`
//!   exempts them before any block rule runs (step 4); the default
//!   frozen allowed-apps list is therefore empty, which already
//!   means "only Preelock plus system processes".
//! * Website blocking: `browser::Policy::new` turns a frozen config
//!   into an allowlist policy whose domains are exactly the frozen
//!   allowed websites (default: none, so every site is blocked).
//!   The extension already enforces allowlist policies, so no
//!   extension change is needed and it stays at version 6. The
//!   adult overlay (step 1) still outranks the allowlist, exactly
//!   as in ordinary allowlist sessions: a frozen session that
//!   allowlists an adult-category host with "Block adult content"
//!   on still blocks that host.
//! * The lock overlay is the visible lock only. It is a fullscreen,
//!   always-on-top Tauri window (`frozen` label, see `lib.rs`) that
//!   shows the frozen timer and reason, with no close affordance:
//!   the main window's close handler prevents closing it, and the
//!   1-second sync in `lib.rs` recreates it on the next tick if it
//!   was closed or unminimizes it if it was minimized. If overlay
//!   creation fails, enforcement continues (app + website blocking
//!   are engine-side) and the failure is recorded as a
//!   `frozen_overlay_failed` report event — the block never
//!   fail-opens because the window failed.
//!
//! NO-OVERRIDE, WITH THE SAFETY BOUND (non-negotiable):
//! While a frozen session is live (Running / Paused / Preparing),
//! ending it early, pausing it, deleting it, and editing /
//! disabling / deleting a schedule whose frozen session is live
//! are rejected in `sessions::apply` — a frozen session is
//! implicitly maximally locked, on top of any step-10 lock it also
//! carries, and there is NO emergency unlock button. The bound is
//! the timer itself: natural expiry always lifts the lock. When
//! the session completes, or its schedule window ends, the session
//! ends, the overlay is destroyed by the sync (see `OverlayAction`
//! below), and the policy is restored (publish finds no live
//! session). On startup recovery (`engine::open_recovery`), a
//! frozen session whose remaining time is already zero is
//! completed, never resumed; a frozen session with time left
//! resumes exactly like any other live session (step 11), and the
//! overlay re-appears on the first sync tick. The timer itself is
//! wall-clock time (`wall_expired`): a frozen session ends after
//! its duration in real minutes even if no productive time was
//! recorded, unlike ordinary attention sessions, which complete on
//! productive time. Without this, a frozen session whose allowed
//! apps never earned productive time could never expire — the
//! wall-clock bound is what makes "never a permanent lockout"
//! literally true. OS shutdown/restart
//! is never intercepted: the app cannot and does not block power
//! actions, and a killed app can never produce a permanent lockout
//! because the timer is the only lock and it always expires.
//!
//! BREAKS AND ALLOWANCES (decided once, step 12):
//! Frozen sessions get NO Pomodoro or reward breaks — a break would
//! relax the very lock the user asked for, so `focus::validate`
//! rejects a frozen config with either break system enabled and
//! the wizard disables both break options while Frozen is checked
//! (with a hint saying why). Allowances (step 8) keep accruing
//! normally during frozen Focus phases: accrual already happens
//! only during Running Focus phases, and a frozen session has no
//! breaks, so it is always in a Focus phase while Running.
//! Frozen schedule sessions follow exactly the same rules as
//! frozen manual sessions, because the frozen config rides the
//! schedule's focus snapshot (step 5) and the same `sessions::apply`
//! gates see the schedule session's own frozen flag.
//!
//! HONEST PLATFORM LIMITS — do not overclaim these:
//! * macOS: a fullscreen window lives on its own Space, and the
//!   user can switch Spaces away from it. The overlay is a strong
//!   deterrent, but the REAL enforcement is the engine's app +
//!   website blocking, which Space-switching does not defeat:
//!   switching to another Space still shows only allowed apps,
//!   and every other app is hidden by the sampler the moment it
//!   becomes foreground.
//! * Windows: Task Manager can kill the app process — the OS owns
//!   process lifetime, and no ordinary app can forbid that. Killing
//!   does NOT end the block: step-11 resume re-asserts it on
//!   relaunch with its remaining time preserved, and the overlay
//!   re-appears. The Store build has no privileged helper, so
//!   there is no claim of an unescapable overlay on either
//!   platform; the guarantee is the outcome (the block outlasts
//!   the window, the kill, and the restart) until its timer ends.
//!
//! All helpers here are pure and shared across macOS and Windows;
//! there is no platform `cfg` in this module.

use crate::focus::{ids_match, rule_matches, website_rules};
use serde::{Deserialize, Serialize};

/// Frozen configuration, stored on [`crate::focus::FocusConfig`].
/// Serde-default: sessions and schedules saved before this step
/// load unfrozen, and no database migration is needed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FrozenConfig {
    /// Whether this session freezes the whole computer.
    pub enabled: bool,
    /// App ids allowed while frozen (bundle ids on macOS, exe
    /// paths or `store:` ids on Windows, matched with
    /// `focus::ids_match`). Empty means only Preelock itself plus
    /// the protected system processes (see `focus::protected`).
    /// The paired browser must be listed here to be usable.
    pub allowed_apps: Vec<String>,
    /// Website rules allowed while frozen, in the same rule
    /// language as `FocusConfig::domains` (domains, `*.example.com`
    /// wildcards, domain/path rules) but never `search:` rules:
    /// a frozen allowlist is about hosts, not search queries.
    /// Empty means no website is allowed.
    pub allowed_websites: Vec<String>,
}

impl FrozenConfig {
    /// Validate and normalize. Disabled configs are never
    /// rejected: their values are ignored, like Pomodoro and
    /// reward configs. An enabled config rejects `search:` website
    /// rules (they cannot be allowlisted hosts) and caps both
    /// lists at 100 entries.
    pub fn validate(mut self) -> Result<Self, String> {
        if !self.enabled {
            return Ok(self);
        }
        if self.allowed_apps.len() > 100 {
            return Err("Use at most 100 allowed apps for a frozen session.".into());
        }
        let mut apps = vec![];
        for value in self.allowed_apps {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            if value.len() > 1024 || value.chars().any(char::is_control) {
                return Err(format!("Invalid allowed app: {value}"));
            }
            apps.push(value.to_string());
        }
        apps.sort();
        apps.dedup();
        self.allowed_apps = apps;
        let websites = website_rules(std::mem::take(&mut self.allowed_websites))?;
        if websites.iter().any(|r| crate::focus::is_keyword_rule(r)) {
            return Err(
                "Frozen allowed websites cannot use search: rules; use a domain, *.example.com, or a domain with a path instead."
                    .into(),
            );
        }
        self.allowed_websites = websites;
        Ok(self)
    }

    /// True when this app id is on the frozen allowed-apps list.
    /// Protected apps (Preelock, system processes) are handled by
    /// the caller (`blocked_sample` checks `protected` first), so
    /// they never need to be listed here.
    pub fn app_allowed(&self, id: &str) -> bool {
        self.allowed_apps.iter().any(|a| ids_match(a, id))
    }

    /// True when this host is on the frozen allowed-websites list.
    /// Path rules are evaluated with an empty path here (the
    /// engine only sees a host); the extension enforces the full
    /// path rule, exactly as for ordinary domain rules.
    pub fn website_allowed(&self, host: &str) -> bool {
        self.allowed_websites
            .iter()
            .any(|r| rule_matches(r, host, ""))
    }

    /// Host rules for the allowlist policy: keyword rules are
    /// rejected by `validate`, so this is normally the whole list.
    pub fn website_host_rules(&self) -> Vec<String> {
        self.allowed_websites
            .iter()
            .filter(|r| !crate::focus::is_keyword_rule(r))
            .cloned()
            .collect()
    }
}

/// What the overlay sync in `lib.rs` must do on one tick. Pure, so
/// the transitions are unit-testable without a Tauri handle:
/// a frozen Running session needs the window (create it when it
/// does not exist, re-assert it when it was minimized); anything
/// else — expiry, natural completion, a schedule window ending —
/// needs it gone. A window that exists and is neither closed nor
/// minimized needs nothing: re-focusing it every second would
/// steal focus from the one allowed app the user is working in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayAction {
    /// No frozen session is running: destroy the window if present.
    Destroy,
    /// A frozen session is running but the window does not exist:
    /// create it (this is also the recovery path — a resumed
    /// frozen session re-appears on the first sync tick — and the
    /// re-assert path when the user closed the window).
    Create,
    /// A frozen session is running and the window exists but is
    /// minimized: unminimize and show it again.
    Reassert,
    /// No change needed.
    None,
}

/// Decide one overlay sync step. `window_exists` is false when the
/// window was closed (or never created); `window_minimized` only
/// matters when it exists.
pub fn overlay_action(
    window_exists: bool,
    window_minimized: bool,
    frozen_running: bool,
) -> OverlayAction {
    if !frozen_running {
        if window_exists {
            OverlayAction::Destroy
        } else {
            OverlayAction::None
        }
    } else if !window_exists {
        OverlayAction::Create
    } else if window_minimized {
        OverlayAction::Reassert
    } else {
        OverlayAction::None
    }
}

/// Wall-clock expiry for frozen sessions (the failsafe that makes
/// the safety bound real): a frozen session ends `duration_minutes`
/// of wall-clock time after it started, even if no productive time
/// was recorded. Ordinary attention sessions complete on productive
/// time, which could strand a frozen session forever if the user
/// never did productive work in an allowed app — a frozen lock
/// must never depend on productive accounting to lift. `None`
/// (never started) never expires; a session that has not started
/// cannot lock anything. Schedule sessions start at window entry,
/// so their wall expiry coincides with their window end.
pub fn wall_expired(started_at: Option<i64>, duration_minutes: u32, now_ms: i64) -> bool {
    match started_at {
        None => false,
        Some(started) => now_ms.saturating_sub(started) >= i64::from(duration_minutes) * 60_000,
    }
}

/// True when a live session must be treated as frozen for
/// enforcement: the config is frozen AND the session is Running.
/// Preparing sessions are not yet enforced (their browser policy
/// is still being acknowledged) and Paused sessions get no ticks;
/// a frozen session is never pausable anyway, so Paused frozen
/// sessions only arise from a pre-step save and are not enforced
/// until resumed.
pub fn is_enforced(frozen_enabled: bool, running: bool) -> bool {
    frozen_enabled && running
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frozen(apps: &[&str], sites: &[&str]) -> FrozenConfig {
        FrozenConfig {
            enabled: true,
            allowed_apps: apps.iter().map(|s| s.to_string()).collect(),
            allowed_websites: sites.iter().map(|s| s.to_string()).collect(),
        }
        .validate()
        .unwrap()
    }

    #[test]
    fn frozen_app_allowlist_defaults_to_nothing_but_matches_listed_apps() {
        let config = frozen(&[], &[]);
        assert!(config.allowed_apps.is_empty());
        assert!(!config.app_allowed("com.google.Chrome"));
        assert!(!config.app_allowed("com.microsoft.VSCode"));
        let config = frozen(&["com.microsoft.VSCode"], &[]);
        assert!(config.app_allowed("com.microsoft.VSCode"));
        assert!(!config.app_allowed("com.google.Chrome"));
        // Windows exe-path matching reuses ids_match: same file
        // under a different spelling still counts as allowed.
        let config = frozen(&[r"C:\Work\Code.exe"], &[]);
        assert!(config.app_allowed(r"C:\Work\Code.exe"));
        assert!(!config.app_allowed(r"C:\Games\Game.exe"));
    }

    #[test]
    fn frozen_website_allowlist_allows_only_listed_hosts() {
        let config = frozen(&[], &[]);
        assert!(!config.website_allowed("example.com"));
        let config = frozen(&[], &["docs.example.com", "*.wikipedia.org"]);
        assert!(config.website_allowed("docs.example.com"));
        assert!(config.website_allowed("en.wikipedia.org"));
        assert!(!config.website_allowed("wikipedia.org"));
        assert!(!config.website_allowed("youtube.com"));
    }

    #[test]
    fn frozen_validation_rejects_search_rules_and_caps_lists() {
        let bad = FrozenConfig {
            enabled: true,
            allowed_apps: vec![],
            allowed_websites: vec!["search:chess".into()],
        }
        .validate();
        assert!(bad.is_err());
        let many: Vec<String> = (0..101).map(|i| format!("app{i}")).collect();
        let bad = FrozenConfig {
            enabled: true,
            allowed_apps: many,
            allowed_websites: vec![],
        }
        .validate();
        assert!(bad.is_err());
        // Disabled configs are never rejected, whatever they hold.
        let disabled = FrozenConfig {
            enabled: false,
            allowed_apps: vec![],
            allowed_websites: vec!["search:chess".into()],
        }
        .validate();
        assert!(disabled.is_ok());
    }

    #[test]
    fn overlay_transitions_cover_create_reassert_destroy_and_expiry() {
        use OverlayAction::*;
        // No frozen session, no window: nothing to do.
        assert_eq!(overlay_action(false, false, false), None);
        // Frozen session starts: create.
        assert_eq!(overlay_action(false, false, true), Create);
        // Window present and visible: leave it alone (no focus steal).
        assert_eq!(overlay_action(true, false, true), None);
        // Window minimized while frozen: re-assert it.
        assert_eq!(overlay_action(true, true, true), Reassert);
        // Window closed while frozen: re-create on the next tick.
        assert_eq!(overlay_action(false, false, true), Create);
        // Expiry / natural completion: destroy a present window.
        assert_eq!(overlay_action(true, false, false), Destroy);
        // Expiry with no window: nothing to do.
        assert_eq!(overlay_action(true, true, false), Destroy);
    }

    #[test]
    fn frozen_wall_expiry_is_wall_clock_not_productive_time() {
        // 25-minute frozen session started 24 minutes ago: not expired.
        assert!(!wall_expired(Some(1_000_000), 25, 1_000_000 + 24 * 60_000));
        // Exactly at the duration: expired.
        assert!(wall_expired(Some(1_000_000), 25, 1_000_000 + 25 * 60_000));
        // Past it: expired, however much productive time is left.
        assert!(wall_expired(Some(1_000_000), 25, 1_000_000 + 90 * 60_000));
        // Never started: never expires.
        assert!(!wall_expired(None, 25, 1_000_000 + 90 * 60_000));
    }

    #[test]
    fn frozen_enforcement_only_while_running() {
        assert!(is_enforced(true, true));
        assert!(!is_enforced(true, false));
        assert!(!is_enforced(false, true));
    }
}

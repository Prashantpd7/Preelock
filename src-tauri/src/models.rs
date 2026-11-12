use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ready,
    Preparing,
    Running,
    Paused,
    Completed,
    Ended,
    Interrupted,
}

/// How a session was created. Set at creation and persisted, so
/// a one-shot session keeps its kind after its timer record is
/// purged when it fires, and a scheduled session keeps its kind
/// even if its schedule is later deleted. Serde-default: sessions
/// saved before this field load as Focus; the frontend still falls
/// back to `schedule_id` for those, so old scheduled sessions
/// still show as scheduled.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    #[default]
    Focus,
    Scheduled,
    OneShot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub duration_minutes: u32,
    pub remaining_ms: u64,
    pub focused_ms: u64,
    pub allow_pause: bool,
    pub status: Status,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub deadline_ms: Option<i64>,
    #[serde(default)]
    pub focus: crate::focus::FocusConfig,
    #[serde(default)]
    pub report: crate::focus::FocusReport,
    #[serde(default)]
    pub attention: crate::attention::Tracking,
    /// Legacy visibility flag from when a same-boot process loss meant the
    /// block had stopped. Sessions are now owned by the engine, so normal app
    /// closure, a UI crash or a handover to the background engine resume the
    /// session without setting it. Kept serialized so historical records still
    /// load and display truthfully; new sessions always load it `false`.
    /// Reset by `RestartSession`.
    #[serde(default)]
    pub force_quit: bool,
    /// Schedule that auto-created this session, if any. Manual
    /// sessions have `None`; schedule sessions are never resumed as
    /// manual sessions and are ended at their window end.
    #[serde(default)]
    pub schedule_id: Option<String>,
    /// How this session was created: an instant focus session, a
    /// scheduled block, or a one-shot timer. Set at creation (see
    /// `SessionKind`); a restarted session resets to Focus, like
    /// `schedule_id` resets to `None`.
    #[serde(default)]
    pub kind: SessionKind,
    /// Live Pomodoro phase state (step 7). Serde-default: sessions
    /// saved before this step load as Focus with no time left, which
    /// the phase machine heals to a full phase on first advance.
    #[serde(default)]
    pub pomodoro: crate::pomodoro::PomodoroState,
    /// Live reward-break state (step 9). Serde-default: sessions
    /// saved before this step load with no credit and no pending or
    /// active break.
    #[serde(default)]
    pub reward: crate::rewards::RewardState,
    /// Lock configuration (step 10). Serde-default: sessions
    /// saved before this step load unlocked.
    #[serde(default)]
    pub lock: crate::locks::LockConfig,
    /// Live lock state (step 10): the challenge to type and any
    /// pending delay request. Serde-default, cleared on end.
    #[serde(default)]
    pub lock_state: crate::locks::LockState,
    /// Step 11 anti-tamper: true once any extension has applied
    /// this session's policy. Persisted in the session JSON, so
    /// it survives a kill/restart: a resumed session whose policy
    /// was applied before the kill is a transient disconnect,
    /// not a never-applied session. Reset by RestartSession.
    #[serde(default)]
    pub policy_applied: bool,
    /// Step 11 anti-tamper: live fail-closed flag, recomputed on
    /// every tick (never-applied session + Running + website
    /// blocking required = browser app blocked itself). Cleared
    /// when the session ends or the extension applies the policy.
    #[serde(default)]
    pub browser_fail_closed: bool,
    /// Step 14 grace period: wall-clock instant (epoch ms) until
    /// which this auto-started session does not enforce yet.
    /// `None` means no grace (manual sessions, grace off, or grace
    /// already expired and cleared). Persisted, so a restart
    /// resumes with the remaining grace time instead of losing
    /// it or stranding the session. Cleared when the session ends.
    #[serde(default)]
    pub grace_until_ms: Option<i64>,
    /// Step 14 grace-start notification: true once the engine has
    /// told the user (one OS notification) that this auto-started
    /// session is in its grace period and blocking begins when the
    /// countdown ends. Persisted with the session, so a later
    /// engine — GUI takeover after the background engine fired,
    /// or a restart resuming grace — never notifies twice, while
    /// an undelivered tick (no channel yet, Windows background
    /// engine) leaves it false so a later tick retries with the
    /// true remaining seconds. Cleared by RestartSession, like
    /// the grace itself.
    #[serde(default)]
    pub grace_notified: bool,
    /// Durable persistence metadata: intended end wall-clock timestamp,
    /// per-occurrence schedule key (anti-duplicate restart guard),
    /// last durable checkpoint & enforcement confirmation timestamps,
    /// and recovery status across crashes/reboots.
    #[serde(default)]
    pub durability: SessionDurability,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDurability {
    #[serde(default)]
    pub intended_end_at_ms: Option<i64>,
    #[serde(default)]
    pub schedule_window_key: Option<String>,
    #[serde(default)]
    pub last_checkpoint_at_ms: Option<i64>,
    #[serde(default)]
    pub last_enforcement_at_ms: Option<i64>,
    #[serde(default)]
    pub recovery_status: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub completed: bool,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSetup {
    pub id: String,
    pub name: String,
    pub minutes: u32,
    pub focus: crate::focus::FocusConfig,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: String,
    pub default_minutes: u32,
    pub large_text: bool,
    pub menu_timer: bool,
    pub daily_goal: u32,
    pub session_alert: bool,
    pub sound: bool,
    pub session_start_alert: bool,
    pub session_start_sound: bool,
    pub volume: u32,
    pub reminder: bool,
    pub reminder_time: String,
    pub reminder_days: Vec<u32>,
    pub offer_break: bool,
    pub break_minutes: u32,
    pub break_alert: bool,
    pub notifications_enabled: bool,
    pub notifications_sound: bool,
    /// Step 14 advance warnings: comma-separated minutes before a
    /// scheduled or one-shot block starts, e.g. `"2"` or `"5, 15"`.
    /// Default `"2"`; empty means no warnings. Normalized on save
    /// (deduped, sorted) by `validate_preferences`.
    pub warning_minutes: String,
    /// Step 14 grace period in seconds after a scheduled or
    /// one-shot block starts before blocking begins. Default 60,
    /// range 0..=300, 0 = off. Manual sessions never get grace.
    pub grace_seconds: u32,
    pub saved_setups: Vec<SavedSetup>,
    /// Screen Time category corrections, keyed by app bundle id/path
    /// or website domain. The frontend checks these before its
    /// built-in mapping, so a correction re-colors past days too.
    /// Serde-default: saves from before this field load as empty.
    pub category_overrides: std::collections::HashMap<String, String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            default_minutes: 25,
            large_text: false,
            menu_timer: true,
            daily_goal: 0,
            session_alert: true,
            sound: true,
            session_start_alert: true,
            session_start_sound: true,
            volume: 50,
            reminder: false,
            reminder_time: "09:00".into(),
            reminder_days: vec![1, 2, 3, 4, 5],
            offer_break: true,
            break_minutes: 5,
            break_alert: true,
            notifications_enabled: true,
            notifications_sound: true,
            warning_minutes: "2".into(),
            grace_seconds: 60,
            saved_setups: vec![],
            category_overrides: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScreenTimeBucket {
    /// Local calendar-day start in epoch milliseconds.
    pub day_ms: i64,
    pub kind: String,
    pub target: String,
    pub name: String,
    pub ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub sessions: Vec<Session>,
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub schedules: Vec<crate::schedules::Schedule>,
    #[serde(default)]
    pub one_shots: Vec<crate::schedules::OneShot>,
    /// Time allowances (step 8). Serde-default: saves from before
    /// this step load as an empty list.
    #[serde(default)]
    pub allowances: Vec<crate::allowances::Allowance>,
    /// App locks: independent timed / daily-limit blocks, enforced
    /// whether or not a session runs. Serde-default: saves from
    /// before this feature load as an empty list.
    #[serde(default)]
    pub app_locks: Vec<crate::applocks::AppLock>,
    /// Locally collected Screen Time totals, bucketed per day and target.
    #[serde(default)]
    pub screen_time_usage: Vec<ScreenTimeBucket>,
    pub preferences: Preferences,
    #[serde(default)]
    pub browser: Option<crate::browser::BrowserStatus>,
    /// "Why am I blocked" app page: the last blocked app + its true
    /// reason, shown in the fullscreen `blocked-app` overlay window
    /// instead of the app just silently hiding. Set by the engine at
    /// the hide points (throttled); cleared on dismiss. Never
    /// persisted: `Database::write` is per-table, so a notice never
    /// reaches the DB, and old saves load with no notice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_app_notice: Option<BlockedAppNotice>,
}

/// One blocked-app page notice: which app was hidden and why, in
/// plain words precomputed by the engine (`blockreasons::cause_text`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BlockedAppNotice {
    /// Display name of the blocked app.
    pub app_name: String,
    /// Plain-words reason, e.g. "Safari is blocked — your “Deep
    /// work” session is on."
    pub reason: String,
    /// Timed locks: wall-clock end (epoch ms) for the live
    /// countdown; None for other causes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at_ms: Option<i64>,
    /// When the notice was set (wall-clock epoch ms); the overlay
    /// sync only shows the page while the notice is fresh.
    pub at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    CreateSession {
        title: String,
        minutes: u32,
        allow_pause: bool,
        #[serde(default)]
        focus: crate::focus::FocusConfig,
        /// Lock configuration (step 10). The plaintext password,
        /// if any, arrives separately as `lock_password` and is
        /// hashed immediately; it is never stored.
        #[serde(default)]
        lock: crate::locks::LockConfig,
        #[serde(default)]
        lock_password: Option<String>,
    },
    StartSession {
        id: String,
    },
    PauseSession {
        id: String,
        /// Proofs for a locked session (step 10); absent when the
        /// session is unlocked or the caller has not supplied them.
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        challenge_answer: Option<String>,
    },
    ResumeSession {
        id: String,
    },
    EndSession {
        id: String,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        challenge_answer: Option<String>,
    },
    DeleteSession {
        id: String,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        challenge_answer: Option<String>,
    },
    /// Start a delay-lock request for one guarded action
    /// (`"end"`, `"pause"`, `"delete"`). The block stays active
    /// during the delay; see `crate::locks`.
    RequestLockDelay {
        id: String,
        for_action: String,
    },
    /// Withdraw a pending delay-lock request (step 10).
    CancelLockDelay {
        id: String,
    },
    RestartSession {
        id: String,
    },
    /// Take a reward break (step 9): starts at once, or after the
    /// configured delay when one is set.
    TakeRewardBreak {
        id: String,
    },
    /// Cancel a pending delayed break before it starts (step 9).
    CancelDelayedBreak {
        id: String,
    },
    EmergencyExit {
        id: String,
        confirmation: String,
    },
    CorrectInterval {
        id: String,
        interval_id: String,
        note: String,
    },
    DisableCamera {
        id: String,
    },
    EnableCamera {
        id: String,
    },
    CreateSchedule {
        title: String,
        days: Vec<u32>,
        start_time: String,
        end_time: String,
        focus: crate::focus::FocusConfig,
        enabled: bool,
        lock_changes: bool,
        /// Step 10 lock (in addition to `lock_changes`). The
        /// plaintext password, if any, arrives as `lock_password`.
        #[serde(default)]
        lock: crate::locks::LockConfig,
        #[serde(default)]
        lock_password: Option<String>,
    },
    UpdateSchedule {
        id: String,
        title: String,
        days: Vec<u32>,
        start_time: String,
        end_time: String,
        focus: crate::focus::FocusConfig,
        enabled: bool,
        lock_changes: bool,
        /// `None` (the quick enabled-toggle path) keeps the
        /// existing lock untouched; `Some` replaces it wholesale.
        /// The frontend sends the stored hash back when the
        /// password lock stays on, and omits it when it is turned
        /// off; a new `lock_password` replaces the hash.
        #[serde(default)]
        lock: Option<crate::locks::LockConfig>,
        #[serde(default)]
        lock_password: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        challenge_answer: Option<String>,
    },
    DeleteSchedule {
        id: String,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        challenge_answer: Option<String>,
    },
    /// Start a delay-lock request for a schedule change
    /// (edit / disable / delete) while its window is active.
    RequestScheduleLockDelay {
        id: String,
    },
    /// Withdraw a pending schedule delay-lock request.
    CancelScheduleLockDelay {
        id: String,
    },
    CreateOneShot {
        title: String,
        minutes: u32,
        allow_pause: bool,
        #[serde(default)]
        focus: crate::focus::FocusConfig,
        fire_at_ms: i64,
    },
    DeleteOneShot {
        id: String,
    },
    CreateAllowance {
        title: String,
        kind: crate::allowances::TargetKind,
        target: String,
        budget_minutes: u32,
        period: crate::allowances::Period,
        mode: crate::allowances::ResetMode,
        enabled: bool,
    },
    UpdateAllowance {
        allowance: crate::allowances::Allowance,
    },
    DeleteAllowance {
        id: String,
    },
    /// App lock: a timed block (minutes from now) or a daily-limit
    /// block (minutes of Screen Time usage per local day) for one
    /// site or app, enforced independently of sessions. Deleting
    /// lifts enforcement at once, since it is computed, never stored.
    CreateAppLock {
        title: String,
        kind: crate::allowances::TargetKind,
        target: String,
        rule: crate::applocks::AppLockRuleInput,
    },
    DeleteAppLock {
        id: String,
    },
    CreateTask {
        title: String,
    },
    UpdateTask {
        id: String,
        title: String,
        completed: bool,
    },
    DeleteTask {
        id: String,
    },
    UpdatePreferences {
        preferences: Preferences,
    },
    SetPreferences {
        theme: String,
        default_minutes: u32,
    },
}

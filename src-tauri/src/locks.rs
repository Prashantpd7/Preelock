//! Lock framework (step 10): a general lock for sessions and
//! scheduled blocks, enforced in the backend and shown in the UI.
//!
//! Four lock types, all optional and combinable. When several are
//! set, ALL of them must be satisfied before the guarded action
//! (ending early, pausing, deleting a live session, or editing /
//! disabling / deleting a locked schedule in its active window)
//! is allowed:
//!
//! * Password lock — the action needs the password set when the
//!   lock was created. Only a salted, iterated hash is stored,
//!   never the password itself (see `hash_password`).
//! * Typed-text challenge — the action needs an exact copy of a
//!   long random string generated server-side (in Rust) when the
//!   lock was created. The string lives in the lock state and is
//!   shown in the UI, Cold Turkey style; typing it proves the
//!   user really meant to break the block, it is not a secret.
//! * Delay lock — the action needs a delay request first (see
//!   `request_delay`): the block stays fully active during the
//!   delay, the request can be cancelled, and the action is
//!   allowed only once the delay has reached zero. The delay is
//!   wall-clock time, like schedule windows: it is not paused by
//!   pausing the session, because a delay that could be frozen
//!   forever would be a lock that never lifts on its own terms.
//!   (The delay request itself is a separate action because a
//!   rejected action is never persisted by the engine commit —
//!   see `engine::commit` — so "first attempt starts the delay
//!   and fails" could never store the pending delay.)
//! * Restart lock — a persistence property, not an exit gate on
//!   its own: a session with only this lock can still be ended
//!   normally, but if the app is closed mid-session it resumes
//!   locked on restart (see `engine::open_recovery`) instead of
//!   being marked interrupted. Combined with any gate above, the
//!   gate still applies after the restart.
//!
//! Safety, non-negotiable: a lock NEVER extends a block's
//! productive target and NEVER blocks natural expiry. When the
//! timer ends, or a schedule window ends, the session ends and
//! its lock state is cleared. A forgotten password therefore
//! never means a permanent lockout: full control returns when
//! the block's timer expires. The gate only applies while the
//! session is live (Running / Paused / Preparing) or, for a
//! schedule, while its window is active; a not-yet-started
//! session and an out-of-window schedule can always be changed.
//!
//! Persistence: lock config and state live in the session /
//! schedule JSON with serde defaults, like step 7/9 state, so
//! pre-step saves load unlocked and no database migration is
//! needed. Locks compose with Pomodoro (step 7), allowances
//! (step 8), reward breaks (step 9) and schedule windows (step
//! 5): a locked session still follows all of them, and a lock
//! rejection is a plain error string the UI can display.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Iterated SHA-256 rounds for a stored password hash. This is a
/// deliberate, documented tradeoff (see below), not Argon2: no
/// password-hashing crate is a direct dependency, and SHA-256 is
/// already in the lockfile (via Tauri/Wry), so adding it as a
/// direct dependency changes no lockfile entries beyond the
/// preelock package itself. The lock's threat model is
/// self-restraint — stopping the user from casually breaking
/// their own block — not protecting a secret against an attacker
/// who stole the database; and no lock survives its timer
/// anyway. A stolen database could still be brute-forced faster
/// than an Argon2 hash would allow, so the password must not be
/// reused from anywhere else. If a future step wants a
/// memory-hard hash, `hash_password` / `verify_password` are
/// the only two functions to replace (the stored format is
/// versioned with a `v1:` prefix for exactly that).
pub const PASSWORD_ITERATIONS: u32 = 100_000;
/// A challenge shorter than this is too easy to type casually;
/// longer than the max is unusable. 0 disables the challenge.
pub const MIN_CHALLENGE_LENGTH: u32 = 8;
pub const MAX_CHALLENGE_LENGTH: u32 = 64;
/// Delay bounds in minutes (task range 1-60); 0 disables it.
pub const MAX_DELAY_MINUTES: u32 = 60;

/// Challenge alphabet: letters and digits without the easily
/// confused `0`, `O`, `1`, `l`, `I`, so a displayed challenge is
/// unambiguous to retype.
const CHALLENGE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";

/// Lock configuration, stored on a session or a schedule.
/// Serde-default: saves from before this step load unlocked.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockConfig {
    /// `v1:{iterations}:{salt_hex}:{hash_hex}`, or `None` when
    /// no password lock is set. The plaintext password is never
    /// stored anywhere; it only crosses the action boundary once,
    /// as `lock_password`, and is hashed immediately.
    #[serde(default)]
    pub password_hash: Option<String>,
    /// 0 = no challenge; otherwise the length of the random
    /// string that must be typed exactly (8-64).
    #[serde(default)]
    pub challenge_length: u32,
    /// 0 = no delay; otherwise the delay in minutes (1-60)
    /// between requesting the guarded action and being allowed
    /// to take it.
    #[serde(default)]
    pub delay_minutes: u32,
    /// Restart persistence (see the module docs): the lock
    /// survives an app restart instead of being dropped.
    #[serde(default)]
    pub restart_lock: bool,
}

impl LockConfig {
    /// True when any gate can actually block an action. A
    /// restart-only lock is not a gate (see the module docs).
    pub fn has_gate(&self) -> bool {
        self.password_hash.is_some() || self.challenge_length > 0 || self.delay_minutes > 0
    }
    /// True when any lock component is set, gate or not.
    pub fn is_configured(&self) -> bool {
        self.has_gate() || self.restart_lock
    }
    /// Range check. A stored hash, if present, must carry the
    /// versioned format; anything else is rejected rather than
    /// trusted, so a corrupt or hand-edited hash can never
    /// silently become "no password".
    pub fn validate(&self) -> Result<(), String> {
        if self.challenge_length != 0
            && !(MIN_CHALLENGE_LENGTH..=MAX_CHALLENGE_LENGTH).contains(&self.challenge_length)
        {
            return Err(format!(
                "Challenge length must be {MIN_CHALLENGE_LENGTH}-{MAX_CHALLENGE_LENGTH} characters, or 0 for no challenge."
            ));
        }
        if self.delay_minutes > MAX_DELAY_MINUTES {
            return Err(format!(
                "Delay must be 0-{MAX_DELAY_MINUTES} minutes (0 means no delay)."
            ));
        }
        if let Some(hash) = &self.password_hash {
            if !hash.starts_with("v1:") {
                return Err("The stored password lock is invalid.".into());
            }
        }
        Ok(())
    }
    /// Human-readable lock summary for the UI badge, or `None`
    /// when nothing is locked.
    pub fn label(&self) -> Option<String> {
        let mut parts: Vec<String> = vec![];
        if self.password_hash.is_some() {
            parts.push("password".into());
        }
        if self.challenge_length > 0 {
            parts.push(format!("challenge ({} chars)", self.challenge_length));
        }
        if self.delay_minutes > 0 {
            parts.push(format!("delay {} min", self.delay_minutes));
        }
        if self.restart_lock {
            parts.push("survives restart".into());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" · "))
        }
    }
}

/// Live lock state, stored alongside the config and cleared
/// when the block ends (naturally or by an allowed early exit),
/// so no lock state ever outlives its block.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LockState {
    /// The current challenge string to type, generated when the
    /// lock was created (or the session restarted). `None` when
    /// no challenge lock is set.
    #[serde(default)]
    pub challenge: Option<String>,
    /// Wall-clock time (epoch ms) from which the pending delay
    /// request allows its action, or `None` when no delay has
    /// been requested. A request for one action never unlocks a
    /// different action.
    #[serde(default)]
    pub delay_until_ms: Option<i64>,
    /// Which action the pending delay was requested for:
    /// `"end"`, `"pause"`, `"delete"` for sessions, `"change"`
    /// for schedule edits / disables / deletes.
    #[serde(default)]
    pub delay_for: Option<String>,
}

/// Fresh lock state for a newly created lock: a challenge when
/// one is configured, no pending delay.
pub fn new_state(config: &LockConfig, seed: &str) -> LockState {
    LockState {
        challenge: if config.challenge_length > 0 {
            Some(generate_challenge(config.challenge_length, seed))
        } else {
            None
        },
        delay_until_ms: None,
        delay_for: None,
    }
}

/// Clear all live lock state. Called when a block ends for any
/// reason, so a finished session never carries a stale challenge
/// or delay into a later run (a restart generates fresh state).
pub fn clear_state(state: &mut LockState) {
    *state = LockState::default();
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

fn stretch(salt: &str, password: &str) -> [u8; 32] {
    let mut digest = Sha256::digest([salt.as_bytes(), password.as_bytes()].concat());
    for _ in 1..PASSWORD_ITERATIONS {
        digest = Sha256::digest(digest);
    }
    digest.into()
}

/// Hash a plaintext password with a fresh random salt. The salt
/// is a v4 UUID (random, 16 bytes as 32 hex chars): the
/// password is never stored, and two locks with the same
/// password get different stored hashes.
pub fn hash_password(password: &str) -> String {
    let salt = uuid::Uuid::new_v4().simple().to_string();
    format!(
        "v1:{PASSWORD_ITERATIONS}:{salt}:{}",
        to_hex(&stretch(&salt, password))
    )
}

/// Verify a plaintext password against a stored hash, in
/// constant time over the hash bytes (no early exit on the
/// first differing byte). An unparseable stored hash never
/// verifies — a corrupt hash fails closed.
pub fn verify_password(stored: &str, password: &str) -> bool {
    let mut parts = stored.split(':');
    let (Some(version), Some(iterations), Some(salt), Some(hash_hex)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if version != "v1" || parts.next().is_some() {
        return false;
    }
    let Ok(iterations) = iterations.parse::<u32>() else {
        return false;
    };
    let Some(expected) = from_hex(hash_hex) else {
        return false;
    };
    let mut digest = Sha256::digest([salt.as_bytes(), password.as_bytes()].concat());
    for _ in 1..iterations {
        digest = Sha256::digest(digest);
    }
    if expected.len() != digest.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.iter().zip(digest.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Generate a random-looking challenge string of `length`
/// characters from a seed (the caller's session/schedule id
/// plus creation time, which includes a random UUID, so two
/// locks never share a challenge). SHA-256 counter mode: the
/// challenge is deterministic for a given seed, which is what
/// lets a restart regenerate the same string if ever needed,
/// while still being unpredictable without the seed.
pub fn generate_challenge(length: u32, seed: &str) -> String {
    let mut out = String::new();
    let mut counter = 0u32;
    while out.len() < length as usize {
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        hasher.update(counter.to_be_bytes());
        let digest = hasher.finalize();
        for b in digest {
            if out.len() >= length as usize {
                break;
            }
            out.push(CHALLENGE_CHARS[(b as usize) % CHALLENGE_CHARS.len()] as char);
        }
        counter += 1;
    }
    out
}

/// Apply a transient plaintext password to a config: validate
/// the ranges, then replace the stored hash when a password was
/// supplied. `None` leaves the config's hash untouched (create
/// paths pass a fresh config, so that means no password lock);
/// an empty supplied password is rejected rather than silently
/// clearing the lock. Schedule edits use
/// `merge_schedule_lock`, not this function.
pub fn apply_password(
    mut config: LockConfig,
    lock_password: Option<String>,
) -> Result<LockConfig, String> {
    config.validate()?;
    if let Some(password) = lock_password {
        if password.trim().is_empty() {
            return Err(
                "Enter a password for the password lock, or turn the password lock off.".into(),
            );
        }
        config.password_hash = Some(hash_password(&password));
    }
    Ok(config)
}

/// Merge a schedule edit's lock over the stored one. `None`
/// (the quick enabled-toggle path) keeps the stored lock.
/// `Some` replaces it wholesale: the caller sends the stored
/// hash back when the password lock stays on, and omits it
/// when it is turned off, so turning a lock off really removes
/// it. A new plaintext password, when supplied, replaces the
/// hash; an empty supplied password is rejected.
pub fn merge_schedule_lock(
    existing: &LockConfig,
    incoming: Option<LockConfig>,
    lock_password: Option<String>,
) -> Result<LockConfig, String> {
    let mut merged = match incoming {
        None => existing.clone(),
        Some(cfg) => {
            cfg.validate()?;
            cfg
        }
    };
    if let Some(supplied) = lock_password {
        if supplied.trim().is_empty() {
            return Err(
                "Enter a password for the password lock, or turn the password lock off.".into(),
            );
        }
        merged.password_hash = Some(hash_password(&supplied));
    }
    Ok(merged)
}

/// The gate check shared by sessions and schedules. `thing` is
/// `"session"` or `"schedule"` for the error text; `verb` is
/// the infinitive (`"end it early"`, `"pause it"`, `"delete
/// it"`, `"change it"`). On success with an elapsed delay, the
/// pending delay is cleared so it cannot be reused for a later
/// action. On failure nothing is mutated: the caller must not
/// persist a failed check (the engine commit discards rejected
/// actions anyway).
/// The eight parameters are the gate's full input (config,
/// mutable state, two label strings, the action key, the two
/// proofs, and the clock); bundling them would only obscure
/// the call sites, so the arity lint is allowed here.
#[allow(clippy::too_many_arguments)]
pub fn check_gate(
    config: &LockConfig,
    state: &mut LockState,
    thing: &str,
    verb: &str,
    for_action: &str,
    password: Option<&str>,
    challenge_answer: Option<&str>,
    now: i64,
) -> Result<(), String> {
    if !config.has_gate() {
        return Ok(());
    }
    if let Some(hash) = &config.password_hash {
        let ok = password.is_some_and(|p| verify_password(hash, p));
        if !ok {
            return Err(format!(
                "This {thing} is password-locked. Enter its password to {verb}."
            ));
        }
    }
    if config.challenge_length > 0 {
        let ok = match (&state.challenge, challenge_answer) {
            (Some(expected), Some(answer)) => answer == expected,
            _ => false,
        };
        if !ok {
            return Err(format!(
                "This {thing} is challenge-locked. Type the challenge text exactly to {verb}."
            ));
        }
    }
    if config.delay_minutes > 0 {
        match (state.delay_until_ms, &state.delay_for) {
            (Some(until), Some(action)) if action == for_action => {
                if now >= until {
                    state.delay_until_ms = None;
                    state.delay_for = None;
                } else {
                    let left = ((until - now + 59_999) / 60_000).max(1);
                    return Err(format!(
                        "This {thing} is delay-locked: {left} minute{} left before it can be {verb}. The block stays active during the delay, and you can cancel the delay request.",
                        if left == 1 { "" } else { "s" }
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "This {thing} is delay-locked. Request the delay first: the block stays active during the delay, and it can be {verb} once the delay reaches zero."
                ));
            }
        }
    }
    Ok(())
}

/// Start a delay request for `for_action`. One pending request
/// at a time: requesting again for the same action while it is
/// still running is rejected (it must not silently extend the
/// wait), while requesting a different action replaces the old
/// one, since only one guarded action can be pending anyway.
pub fn request_delay(
    config: &LockConfig,
    state: &mut LockState,
    for_action: &str,
    now: i64,
) -> Result<(), String> {
    if config.delay_minutes == 0 {
        return Err("This lock has no delay. No delay request is needed.".into());
    }
    if let (Some(until), Some(action)) = (state.delay_until_ms, &state.delay_for) {
        if action == for_action && now < until {
            let left = ((until - now + 59_999) / 60_000).max(1);
            return Err(format!(
                "A delay request is already running: {left} minute{} left.",
                if left == 1 { "" } else { "s" }
            ));
        }
    }
    state.delay_until_ms = Some(now + i64::from(config.delay_minutes) * 60_000);
    state.delay_for = Some(for_action.to_string());
    Ok(())
}

/// Cancel a pending delay request. Cancelling is always free:
/// it never ends the block, it only withdraws the request to
/// end or change it.
pub fn cancel_delay(state: &mut LockState) -> Result<(), String> {
    if state.delay_until_ms.is_none() {
        return Err("No delay request is pending.".into());
    }
    state.delay_until_ms = None;
    state.delay_for = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_verifies_and_salts() {
        let a = hash_password("correct horse");
        let b = hash_password("correct horse");
        assert_ne!(a, b, "same password must get a different salt");
        assert!(verify_password(&a, "correct horse"));
        assert!(verify_password(&b, "correct horse"));
        assert!(!verify_password(&a, "wrong horse"));
        assert!(!verify_password(&a, ""));
        assert!(!verify_password("garbage", "correct horse"));
        assert!(!verify_password("v1:notanumber:ab:cd", "correct horse"));
    }

    #[test]
    fn lock_config_ranges_are_validated() {
        let mut config = LockConfig::default();
        assert!(config.validate().is_ok());
        config.challenge_length = 4;
        assert!(config.validate().is_err());
        config.challenge_length = 32;
        config.delay_minutes = 61;
        assert!(config.validate().is_err());
        config.delay_minutes = 60;
        assert!(config.validate().is_ok());
        config.password_hash = Some("unversioned".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn challenge_has_requested_length_and_alphabet() {
        let challenge = generate_challenge(32, "session-1:1000");
        assert_eq!(challenge.len(), 32);
        assert!(challenge.bytes().all(|b| CHALLENGE_CHARS.contains(&b)));
        assert_eq!(generate_challenge(32, "session-1:1000"), challenge);
        assert_ne!(generate_challenge(32, "session-2:1000"), challenge);
        assert_eq!(generate_challenge(8, "x").len(), 8);
        assert_eq!(generate_challenge(64, "x").len(), 64);
    }

    #[test]
    fn gate_requires_every_configured_lock() {
        let config = LockConfig {
            password_hash: Some(hash_password("secret")),
            challenge_length: 16,
            delay_minutes: 0,
            restart_lock: false,
        };
        let mut state = new_state(&config, "s:1");
        let challenge = state.challenge.clone().unwrap();
        // No proofs at all.
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            None,
            None,
            1000
        )
        .is_err());
        // Password only: challenge still blocks.
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            Some("secret"),
            None,
            1000
        )
        .is_err());
        // Wrong challenge still blocks.
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            Some("secret"),
            Some("nope"),
            1000
        )
        .is_err());
        // Both proofs pass.
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            Some("secret"),
            Some(&challenge),
            1000
        )
        .is_ok());
    }

    #[test]
    fn delay_counts_down_and_cancels() {
        let config = LockConfig {
            delay_minutes: 5,
            ..Default::default()
        };
        let mut state = LockState::default();
        // No request yet: the action is refused, with guidance.
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            None,
            None,
            1000
        )
        .is_err());
        request_delay(&config, &mut state, "end", 1000).unwrap();
        // A second request for the same action does not extend it.
        assert!(request_delay(&config, &mut state, "end", 2000).is_err());
        assert_eq!(state.delay_until_ms, Some(1000 + 5 * 60_000));
        // Still waiting one minute before expiry.
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            None,
            None,
            1000 + 4 * 60_000
        )
        .is_err());
        // A pending delay for another action does not unlock this one.
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "pause it",
            "pause",
            None,
            None,
            1000 + 6 * 60_000
        )
        .is_err());
        // At zero the action passes and the pending request clears.
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            None,
            None,
            1000 + 5 * 60_000
        )
        .is_ok());
        assert_eq!(state.delay_until_ms, None);
        // Cancel: a fresh request can be withdrawn, and the
        // action is refused again afterwards.
        request_delay(&config, &mut state, "end", 2000).unwrap();
        cancel_delay(&mut state).unwrap();
        assert!(cancel_delay(&mut state).is_err());
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            None,
            None,
            2000
        )
        .is_err());
    }

    #[test]
    fn schedule_lock_merge_preserves_replaces_and_clears() {
        let existing = LockConfig {
            password_hash: Some(hash_password("old")),
            challenge_length: 16,
            delay_minutes: 5,
            restart_lock: true,
        };
        // Toggle path: untouched.
        assert_eq!(
            merge_schedule_lock(&existing, None, None).unwrap(),
            existing
        );
        // Edit path with the hash sent back: kept.
        let kept = merge_schedule_lock(
            &existing,
            Some(LockConfig {
                password_hash: existing.password_hash.clone(),
                challenge_length: 0,
                delay_minutes: 0,
                restart_lock: false,
            }),
            None,
        )
        .unwrap();
        assert!(kept.password_hash.is_some());
        assert_eq!(kept.challenge_length, 0);
        // Edit path with the hash omitted: the password lock is gone.
        let cleared = merge_schedule_lock(&existing, Some(LockConfig::default()), None).unwrap();
        assert_eq!(cleared.password_hash, None);
        // A new password replaces the old hash.
        let replaced =
            merge_schedule_lock(&existing, Some(LockConfig::default()), Some("new".into()))
                .unwrap();
        assert!(verify_password(
            replaced.password_hash.as_ref().unwrap(),
            "new"
        ));
        assert!(!verify_password(
            replaced.password_hash.as_ref().unwrap(),
            "old"
        ));
        // An empty supplied password is rejected, not a silent clear.
        assert!(
            merge_schedule_lock(&existing, Some(LockConfig::default()), Some("  ".into())).is_err()
        );
    }

    #[test]
    fn restart_only_lock_is_not_an_exit_gate() {
        let config = LockConfig {
            restart_lock: true,
            ..Default::default()
        };
        let mut state = LockState::default();
        assert!(config.is_configured());
        assert!(!config.has_gate());
        assert!(check_gate(
            &config,
            &mut state,
            "session",
            "end it early",
            "end",
            None,
            None,
            1000
        )
        .is_ok());
    }

    #[test]
    fn lock_state_roundtrips_through_json() {
        let config = LockConfig {
            password_hash: Some(hash_password("secret")),
            challenge_length: 32,
            delay_minutes: 5,
            restart_lock: true,
        };
        let state = new_state(&config, "s:9");
        let json = serde_json::to_string(&(&config, &state)).unwrap();
        let (back_config, back_state): (LockConfig, LockState) =
            serde_json::from_str(&json).unwrap();
        assert_eq!(back_config, config);
        assert_eq!(back_state.challenge, state.challenge);
        assert!(back_config.label().unwrap().contains("password"));
    }
}

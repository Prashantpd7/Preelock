use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::LazyLock;

/// Maximum duration permitted for adult website blocking: 7 days.
pub const MAX_DURATION_MINUTES: u32 = 10_080;

/// Persistent configuration and state of System-Wide Adult Website Blocking.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdultBlock {
    pub enabled: bool,
    pub strict: bool,
    #[serde(default)]
    pub ends_at_ms: Option<i64>,
    #[serde(default)]
    pub activating_until_ms: Option<i64>,
    pub created_at_ms: i64,
}

static ADULT_DOMAINS_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let raw = include_str!("../resources/adult-domains.txt");
    raw.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
});

/// Fast check whether a given hostname or any of its parent domain suffixes
/// matches a known adult domain in the maintained blocklist (47,664 domains).
pub fn is_adult_domain(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if ADULT_DOMAINS_SET.contains(host.as_str()) {
        return true;
    }
    // Check parent domains (e.g. "sub.example.xxx" -> "example.xxx")
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() > 2 {
        for i in 1..labels.len() - 1 {
            let parent = labels[i..].join(".");
            if ADULT_DOMAINS_SET.contains(parent.as_str()) {
                return true;
            }
        }
    }
    false
}

/// Applies user action to AdultBlock state, enforcing strict non-cancellability
/// and maximum duration limits.
pub fn apply_adult_block_action(
    current: &AdultBlock,
    enabled: bool,
    strict: bool,
    minutes: Option<u32>,
    now_ms: i64,
) -> Result<AdultBlock, String> {
    let is_currently_strict_active =
        current.enabled && current.strict && current.ends_at_ms.is_some_and(|end| now_ms < end);

    if is_currently_strict_active {
        if !enabled || !strict {
            return Err("Strict adult website block is active and cannot be cancelled or disabled until its configured expiry.".into());
        }
        if let (Some(cur_end), Some(new_mins)) = (current.ends_at_ms, minutes) {
            let proposed_end = now_ms.saturating_add((new_mins as i64) * 60_000);
            if proposed_end < cur_end {
                return Err("Cannot shorten the duration of an active strict adult block.".into());
            }
        }
    }

    if !enabled {
        return Ok(AdultBlock {
            enabled: false,
            strict: false,
            ends_at_ms: None,
            activating_until_ms: None,
            created_at_ms: current.created_at_ms,
        });
    }

    if strict {
        let mins = minutes.ok_or("A duration up to 7 days is required for Strict adult block.")?;
        if mins == 0 || mins > MAX_DURATION_MINUTES {
            return Err(format!(
                "Duration must be between 1 minute and 7 days ({MAX_DURATION_MINUTES} minutes)."
            ));
        }
        let ends_at_ms = now_ms.saturating_add((mins as i64) * 60_000);
        Ok(AdultBlock {
            enabled: true,
            strict: true,
            ends_at_ms: Some(ends_at_ms),
            activating_until_ms: None,
            created_at_ms: if current.enabled {
                current.created_at_ms
            } else {
                now_ms
            },
        })
    } else {
        let ends_at_ms = match minutes {
            Some(m) if m > 0 && m <= MAX_DURATION_MINUTES => {
                Some(now_ms.saturating_add((m as i64) * 60_000))
            }
            Some(m) if m > MAX_DURATION_MINUTES => {
                return Err(format!(
                    "Duration cannot exceed 7 days ({MAX_DURATION_MINUTES} minutes)."
                ));
            }
            _ => None,
        };
        Ok(AdultBlock {
            enabled: true,
            strict: false,
            ends_at_ms,
            activating_until_ms: None,
            created_at_ms: if current.enabled {
                current.created_at_ms
            } else {
                now_ms
            },
        })
    }
}

/// Begins a 20-second countdown for strict mode activation.
pub fn start_strict_countdown(
    strict: bool,
    ends_at_ms: &mut Option<i64>,
    activating_until_ms: &mut Option<i64>,
    days: u32,
    now_ms: i64,
) -> Result<(), String> {
    if strict && ends_at_ms.is_some_and(|end| now_ms < end) {
        return Err("Strict block is already active and cannot be modified until expired.".into());
    }
    if !(1..=7).contains(&days) {
        return Err("Strict block duration must be between 1 and 7 days.".into());
    }
    let countdown_ms = 20_000;
    let duration_ms = (days as i64) * 86_400_000;
    *activating_until_ms = Some(now_ms.saturating_add(countdown_ms));
    *ends_at_ms = Some(
        now_ms
            .saturating_add(countdown_ms)
            .saturating_add(duration_ms),
    );
    Ok(())
}

/// Cancels a pending 20-second countdown before strict mode activates.
pub fn cancel_strict_countdown(
    strict: bool,
    ends_at_ms: &mut Option<i64>,
    activating_until_ms: &mut Option<i64>,
    now_ms: i64,
) -> Result<(), String> {
    if strict && ends_at_ms.is_some_and(|end| now_ms < end) {
        return Err(
            "Strict block is active and cannot be cancelled or disabled until expiry.".into(),
        );
    }
    *activating_until_ms = None;
    *ends_at_ms = None;
    Ok(())
}

/// Transitions state from 20-second countdown to locked strict mode,
/// or from expired strict mode to deactivated. Returns true if state changed.
pub fn check_strict_transition(
    enabled: &mut bool,
    strict: &mut bool,
    ends_at_ms: &mut Option<i64>,
    activating_until_ms: &mut Option<i64>,
    now_ms: i64,
) -> bool {
    let mut changed = false;
    if let Some(act) = *activating_until_ms {
        if now_ms >= act {
            *strict = true;
            *enabled = true;
            *activating_until_ms = None;
            changed = true;
        }
    }
    if *strict && ends_at_ms.is_some_and(|end| now_ms >= end) {
        *strict = false;
        *enabled = false;
        *ends_at_ms = None;
        *activating_until_ms = None;
        changed = true;
    }
    changed
}

/// Checks if an active adult block with a deadline has expired or activated.
pub fn check_expiry(block: &mut AdultBlock, now_ms: i64) -> bool {
    check_strict_transition(
        &mut block.enabled,
        &mut block.strict,
        &mut block.ends_at_ms,
        &mut block.activating_until_ms,
        now_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adult_domain_detection_matches_exact_and_subdomains() {
        assert!(is_adult_domain("pornhub.com"));
        assert!(is_adult_domain("www.pornhub.com"));
        assert!(is_adult_domain("cdn.xvideos.com"));
        assert!(is_adult_domain("chaturbate.com"));
        assert!(!is_adult_domain("google.com"));
        assert!(!is_adult_domain("github.com"));
        assert!(!is_adult_domain("apple.com"));
        assert!(!is_adult_domain("preelock.com"));
        assert!(!is_adult_domain(""));
    }

    #[test]
    fn strict_adult_block_rejects_cancellation_before_expiry() {
        let now = 1_000_000;
        let active_strict = AdultBlock {
            enabled: true,
            strict: true,
            ends_at_ms: Some(now + 60_000),
            activating_until_ms: None,
            created_at_ms: now,
        };

        // Attempt disable while active
        let res = apply_adult_block_action(&active_strict, false, false, None, now + 1000);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("cannot be cancelled"));

        // Attempt un-strict while active
        let res = apply_adult_block_action(&active_strict, true, false, None, now + 1000);
        assert!(res.is_err());

        // Attempt duration shortening while active
        let res = apply_adult_block_action(&active_strict, true, true, Some(0), now + 1000);
        assert!(res.is_err());
    }

    #[test]
    fn duration_caps_at_seven_days() {
        let now = 1_000_000;
        let blank = AdultBlock::default();

        let over =
            apply_adult_block_action(&blank, true, true, Some(MAX_DURATION_MINUTES + 1), now);
        assert!(over.is_err());

        let valid = apply_adult_block_action(&blank, true, true, Some(MAX_DURATION_MINUTES), now);
        assert!(valid.is_ok());
        assert_eq!(
            valid.unwrap().ends_at_ms,
            Some(now + (MAX_DURATION_MINUTES as i64) * 60_000)
        );
    }

    #[test]
    fn automatic_expiry_lifts_enforcement() {
        let now = 1_000_000;
        let mut block = AdultBlock {
            enabled: true,
            strict: true,
            ends_at_ms: Some(now + 60_000),
            activating_until_ms: None,
            created_at_ms: now,
        };

        // Before expiry: no change
        assert!(!check_expiry(&mut block, now + 30_000));
        assert!(block.enabled);

        // At expiry: expires
        assert!(check_expiry(&mut block, now + 60_000));
        assert!(!block.enabled);
        assert!(!block.strict);
        assert_eq!(block.ends_at_ms, None);
    }
}

use crate::models::ScreenTimeBucket;

pub fn local_day_start_ms(now_ms: i64) -> i64 {
    let (_, minutes) = crate::schedules::local_weekday_and_minutes(now_ms);
    now_ms
        .saturating_sub(i64::from(minutes) * 60_000)
        .saturating_sub(now_ms.rem_euclid(60_000))
}

pub fn add_usage(
    buckets: &mut Vec<ScreenTimeBucket>,
    day_ms: i64,
    kind: &str,
    target: &str,
    name: &str,
    ms: u64,
) {
    if target.is_empty() || ms == 0 {
        return;
    }
    if let Some(bucket) = buckets
        .iter_mut()
        .find(|b| b.day_ms == day_ms && b.kind == kind && b.target == target)
    {
        bucket.ms = bucket.ms.saturating_add(ms);
        if !name.is_empty() {
            bucket.name = name.into();
        }
    } else {
        buckets.push(ScreenTimeBucket {
            day_ms,
            kind: kind.into(),
            target: target.into(),
            name: name.into(),
            ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_per_target_and_separates_days() {
        let mut buckets = Vec::new();
        add_usage(&mut buckets, 1, "app", "com.example", "Example", 1000);
        add_usage(&mut buckets, 1, "app", "com.example", "Example App", 2000);
        add_usage(&mut buckets, 2, "app", "com.example", "Example", 3000);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].ms, 3000);
        assert_eq!(buckets[0].name, "Example App");
    }

    #[test]
    fn ignores_empty_and_zero_usage() {
        let mut buckets = Vec::new();
        add_usage(&mut buckets, 1, "site", "", "", 1000);
        add_usage(&mut buckets, 1, "site", "example.com", "example.com", 0);
        assert!(buckets.is_empty());
    }
}

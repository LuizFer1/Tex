pub const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    Time { days: u32 },
    Storage { limit_bytes: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowFacts {
    pub id: i64,
    pub copied_at: i64,
    pub size: u64,
    pub favorite: bool,
}

fn period(days: u32) -> i64 {
    i64::from(days) * DAY_MS
}

/// Non-favorites copied before `now - days`.
pub fn expired(rows: &[RowFacts], days: u32, now: i64) -> Vec<i64> {
    let cutoff = now.saturating_sub(period(days));
    rows.iter()
        .filter(|row| !row.favorite && row.copied_at < cutoff)
        .map(|row| row.id)
        .collect()
}

/// When non-favorites reach 90% of `limit`, the oldest of them until the total is at most 80%.
/// `rows` in any order.
pub fn storage_victims(rows: &[RowFacts], limit: u64) -> Vec<i64> {
    let mut candidates: Vec<&RowFacts> = rows.iter().filter(|row| !row.favorite).collect();
    let mut total: u128 = candidates.iter().map(|row| u128::from(row.size)).sum();
    let limit = u128::from(limit);
    if total * 10 < limit * 9 {
        return Vec::new();
    }
    candidates.sort_by_key(|row| (row.copied_at, row.id));
    let mut victims = Vec::new();
    for row in candidates {
        if total * 10 <= limit * 8 {
            break;
        }
        total -= u128::from(row.size);
        victims.push(row.id);
    }
    victims
}

/// When the oldest non-favorite expires, for the time policy.
pub fn next_expiry(rows: &[RowFacts], days: u32) -> Option<i64> {
    rows.iter()
        .filter(|row| !row.favorite)
        .map(|row| row.copied_at)
        .min()
        .map(|oldest| oldest.saturating_add(period(days)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, copied_at: i64, size: u64, favorite: bool) -> RowFacts {
        RowFacts {
            id,
            copied_at,
            size,
            favorite,
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn expires_strictly_after_the_period() {
        let limit = NOW - 7 * DAY_MS;
        let rows = [row(1, limit, 1, false), row(2, limit - 1, 1, false)];
        assert_eq!(expired(&rows, 7, NOW), [2]);
    }

    #[test]
    fn favorites_never_expire() {
        let rows = [
            row(1, 0, 1, true),
            row(2, 0, 1, false),
            row(3, NOW, 1, false),
        ];
        assert_eq!(expired(&rows, 3, NOW), [2]);
    }

    #[test]
    fn below_90_percent_removes_nothing() {
        let rows = [row(1, 1, 50, false), row(2, 2, 39, false)];
        assert!(storage_victims(&rows, 100).is_empty());
    }

    #[test]
    fn at_90_percent_removes_oldest_until_80_percent() {
        let rows = [
            row(1, 1, 5, false),
            row(2, 2, 5, false),
            row(3, 3, 5, false),
            row(4, 4, 75, false),
        ];
        assert_eq!(storage_victims(&rows, 100), [1, 2]);
    }

    #[test]
    fn large_favorites_do_not_count() {
        let rows = [
            row(1, 1, 1_000, true),
            row(2, 2, 10, false),
            row(3, 3, 79, false),
        ];
        assert!(storage_victims(&rows, 100).is_empty());
        let rows = [
            row(1, 1, 1_000, true),
            row(2, 2, 11, false),
            row(3, 3, 79, false),
        ];
        assert_eq!(storage_victims(&rows, 100), [2]);
    }

    #[test]
    fn storage_order_is_by_age_then_id() {
        let rows = [
            row(1, 30, 30, false),
            row(2, 10, 30, false),
            row(4, 20, 30, false),
            row(3, 20, 30, false),
        ];
        assert_eq!(storage_victims(&rows, 100), [2, 3]);
    }

    #[test]
    fn next_expiry_ignores_favorites() {
        let rows = [
            row(1, 5, 1, true),
            row(2, 20, 1, false),
            row(3, 10, 1, false),
        ];
        assert_eq!(next_expiry(&rows, 3), Some(10 + 3 * DAY_MS));
        assert_eq!(next_expiry(&[row(1, 5, 1, true)], 3), None);
        assert_eq!(next_expiry(&[], 3), None);
    }
}

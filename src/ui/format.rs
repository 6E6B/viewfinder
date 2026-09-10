use std::time::{SystemTime, UNIX_EPOCH};

const COUNT_UNITS: [&str; 7] = ["", "K", "M", "B", "T", "Q", "Qi"];

pub fn compact_count(number: u64) -> String {
    if number < 1_000 {
        return number.to_string();
    }

    let mut unit = 0;
    let mut divisor = 1_u64;
    while unit + 1 < COUNT_UNITS.len() && divisor <= u64::MAX / 1_000 && number >= divisor * 1_000 {
        divisor *= 1_000;
        unit += 1;
    }

    loop {
        let whole = number / divisor;
        if whole < 100 {
            let tenths = ((number as u128 * 10 + divisor as u128 / 2) / divisor as u128) as u64;
            if tenths >= 10_000 && unit + 1 < COUNT_UNITS.len() {
                divisor *= 1_000;
                unit += 1;
                continue;
            }
            if tenths.is_multiple_of(10) {
                return format!("{}{}", tenths / 10, COUNT_UNITS[unit]);
            }
            return format!("{}.{}{}", tenths / 10, tenths % 10, COUNT_UNITS[unit]);
        }

        let rounded = ((number as u128 + divisor as u128 / 2) / divisor as u128) as u64;
        if rounded >= 1_000 && unit + 1 < COUNT_UNITS.len() {
            divisor *= 1_000;
            unit += 1;
            continue;
        }
        return format!("{}{}", rounded, COUNT_UNITS[unit]);
    }
}

pub fn relative_time(timestamp: i64) -> Option<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    relative_time_at(timestamp, now)
}

fn relative_time_at(timestamp: i64, now: i64) -> Option<String> {
    if timestamp <= 0 {
        return None;
    }

    let timestamp = normalize_timestamp(timestamp);
    let age = now.saturating_sub(timestamp).max(0);
    let (value, unit) = match age {
        0..=9 => return Some("now".into()),
        10..=59 => (age, "s"),
        60..=3_599 => (age / 60, "m"),
        3_600..=86_399 => (age / 3_600, "h"),
        86_400..=604_799 => (age / 86_400, "d"),
        604_800..=2_591_999 => (age / 604_800, "w"),
        2_592_000..=31_535_999 => (age / 2_592_000, "mo"),
        _ => (age / 31_536_000, "y"),
    };
    Some(format!("{value}{unit} ago"))
}

fn normalize_timestamp(timestamp: i64) -> i64 {
    match timestamp.unsigned_abs() {
        1_000_000_000_000_000_000.. => timestamp / 1_000_000_000,
        1_000_000_000_000_000.. => timestamp / 1_000_000,
        1_000_000_000_000.. => timestamp / 1_000,
        _ => timestamp,
    }
}

/// DM timestamps are microseconds; tolerate other API precisions too.
pub fn message_time(timestamp: i64) -> Option<gtk::glib::DateTime> {
    if timestamp <= 0 {
        return None;
    }
    gtk::glib::DateTime::from_unix_local(normalize_timestamp(timestamp)).ok()
}

pub fn same_message_group(previous: i64, current: i64) -> bool {
    let (Some(a), Some(b)) = (message_time(previous), message_time(current)) else {
        return false;
    };
    let gap = normalize_timestamp(current).saturating_sub(normalize_timestamp(previous));
    (0..900).contains(&gap) && a.year() == b.year() && a.day_of_year() == b.day_of_year()
}

pub fn message_timestamp(timestamp: i64) -> Option<String> {
    message_time(timestamp)?
        .format("%b %e, %Y · %H:%M")
        .ok()
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_groups_respect_gaps_dates_and_missing_times() {
        let noon = gtk::glib::DateTime::from_local(2026, 9, 9, 12, 0, 0.0)
            .unwrap()
            .to_unix();
        assert!(same_message_group(
            noon * 1_000_000,
            (noon + 899) * 1_000_000
        ));
        assert!(!same_message_group(noon, noon + 900));
        assert!(!same_message_group(noon, noon - 1));
        assert!(!same_message_group(0, noon));
        let midnight = gtk::glib::DateTime::from_local(2026, 9, 10, 0, 0, 0.0)
            .unwrap()
            .to_unix();
        assert!(!same_message_group(midnight - 60, midnight));
    }

    #[test]
    fn compacts_counts_and_carries_rounded_units() {
        for (number, expected) in [
            (0, "0"),
            (999, "999"),
            (1_000, "1K"),
            (1_049, "1K"),
            (1_050, "1.1K"),
            (26_200, "26.2K"),
            (123_456, "123K"),
            (999_499, "999K"),
            (999_500, "1M"),
            (1_250_000, "1.3M"),
        ] {
            assert_eq!(compact_count(number), expected);
        }
    }

    #[test]
    fn formats_relative_time_for_each_unit() {
        let now = 2_000_000_000;
        for (age, expected) in [
            (4, "now"),
            (12, "12s ago"),
            (6 * 60, "6m ago"),
            (5 * 3_600, "5h ago"),
            (6 * 86_400, "6d ago"),
            (3 * 604_800, "3w ago"),
            (5 * 2_592_000, "5mo ago"),
            (2 * 31_536_000, "2y ago"),
        ] {
            assert_eq!(relative_time_at(now - age, now).as_deref(), Some(expected));
        }
    }

    #[test]
    fn accepts_api_timestamp_precisions() {
        let now = 2_000_000_000;
        assert_eq!(
            relative_time_at(1_999_913_600, now).as_deref(),
            Some("1d ago")
        );
        assert_eq!(
            relative_time_at(1_999_913_600_000, now).as_deref(),
            Some("1d ago")
        );
        assert_eq!(
            relative_time_at(1_999_913_600_000_000, now).as_deref(),
            Some("1d ago")
        );
        assert_eq!(
            relative_time_at(1_999_913_600_000_000_000, now).as_deref(),
            Some("1d ago")
        );
        assert_eq!(relative_time_at(0, now), None);
        assert_eq!(relative_time_at(now + 60, now).as_deref(), Some("now"));
    }
}

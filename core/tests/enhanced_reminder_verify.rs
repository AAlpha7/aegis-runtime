//! Standalone tests for reminder schedule calculation logic.
//!
//! Tests the time-generation algorithms that enhanced/recurring reminders
//! depend on. (A former Wasm sandbox test against a
//! `smart_recurring_reminder_calc` skill was removed on 2026-09-16: no such
//! skill exists under `skills/` and nothing builds its `.wasm`, so the test
//! could never run.)

use std::collections::HashSet;

// ── Pure logic: reminder schedule generation ─────────────────────────────

fn days_in_month(y: i32, m: i32) -> i32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn next_day(y: i32, m: i32, d: i32) -> (i32, i32, i32) {
    let dim = days_in_month(y, m);
    if d < dim {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    }
}

fn format_iso(y: i32, m: i32, d: i32, hh: i32, mm: i32, ss: i32) -> String {
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hh, mm, ss)
}

fn parse_hhmm(s: &str) -> Option<(i32, i32)> {
    if s.len() < 5 {
        return None;
    }
    let hh = s[0..2].parse::<i32>().ok()?;
    let mm = s[3..5].parse::<i32>().ok()?;
    Some((hh, mm))
}

/// Generate a one-time reminder at the given datetime.
fn generate_one_time(start: &str, message: &str) -> Vec<(String, String)> {
    vec![(start.to_string(), message.to_string())]
}

/// Generate recurring reminders with multiple daily times, distributing `count` across days.
fn generate_recurring_multi_time(
    start_y: i32,
    start_m: i32,
    start_d: i32,
    times: &[&str],
    count: usize,
    message: &str,
) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut cy = start_y;
    let mut cm = start_m;
    let mut cd = start_d;
    let mut remaining = count;

    while remaining > 0 {
        for t in times {
            if remaining == 0 {
                break;
            }
            if let Some((th, tm)) = parse_hhmm(t) {
                result.push((format_iso(cy, cm, cd, th, tm, 0), message.to_string()));
                remaining -= 1;
            }
        }
        let n = next_day(cy, cm, cd);
        cy = n.0;
        cm = n.1;
        cd = n.2;
    }
    result
}

// ── Test: one-time reminder produces exactly 1 entry ─────────────────────

#[test]
fn one_time_single_reminder() {
    let reminders = generate_one_time("2025-03-01T10:30:00Z", "开会");
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].0, "2025-03-01T10:30:00Z");
    assert_eq!(reminders[0].1, "开会");
}

// ── Test: daily with 3 times, count=3 → 3 unique timestamps on day 1 ────

#[test]
fn daily_with_three_times_count_3_no_duplicates() {
    let reminders =
        generate_recurring_multi_time(2025, 1, 15, &["09:00", "14:00", "21:00"], 3, "喝水");

    assert_eq!(reminders.len(), 3, "should generate exactly 3 reminders");

    let times: Vec<&str> = reminders.iter().map(|r| r.0.as_str()).collect();
    let unique: HashSet<&str> = times.iter().copied().collect();
    assert_eq!(
        unique.len(),
        3,
        "all 3 timestamps must be UNIQUE, got: {:?}",
        times
    );

    assert!(
        times.contains(&"2025-01-15T09:00:00Z"),
        "missing 09:00, got {:?}",
        times
    );
    assert!(
        times.contains(&"2025-01-15T14:00:00Z"),
        "missing 14:00, got {:?}",
        times
    );
    assert!(
        times.contains(&"2025-01-15T21:00:00Z"),
        "missing 21:00, got {:?}",
        times
    );
}

// ── Test: daily with 3 times, count=9 → 9 unique timestamps across 3 days ─

#[test]
fn daily_with_three_times_count_9_spans_three_days() {
    let reminders =
        generate_recurring_multi_time(2025, 2, 1, &["08:00", "12:00", "20:00"], 9, "吃药");

    assert_eq!(
        reminders.len(),
        9,
        "should generate 9 reminders across 3 days"
    );

    let times: Vec<&str> = reminders.iter().map(|r| r.0.as_str()).collect();
    let unique: HashSet<&str> = times.iter().copied().collect();
    assert_eq!(
        unique.len(),
        9,
        "all 9 timestamps must be unique, got: {:?}",
        times
    );

    // Day 1: Feb 1
    assert!(times.contains(&"2025-02-01T08:00:00Z"));
    assert!(times.contains(&"2025-02-01T12:00:00Z"));
    assert!(times.contains(&"2025-02-01T20:00:00Z"));
    // Day 2: Feb 2
    assert!(times.contains(&"2025-02-02T08:00:00Z"));
    assert!(times.contains(&"2025-02-02T12:00:00Z"));
    assert!(times.contains(&"2025-02-02T20:00:00Z"));
    // Day 3: Feb 3
    assert!(times.contains(&"2025-02-03T08:00:00Z"));
    assert!(times.contains(&"2025-02-03T12:00:00Z"));
    assert!(times.contains(&"2025-02-03T20:00:00Z"));
}

// ── Test: month boundary crossing ────────────────────────────────────────

#[test]
fn recurring_crosses_month_boundary() {
    let reminders = generate_recurring_multi_time(2025, 1, 31, &["10:00"], 3, "月末跨月");

    assert_eq!(reminders.len(), 3);
    assert_eq!(reminders[0].0, "2025-01-31T10:00:00Z");
    assert_eq!(reminders[1].0, "2025-02-01T10:00:00Z");
    assert_eq!(reminders[2].0, "2025-02-02T10:00:00Z");
}

// ── Test: leap year Feb 29 ───────────────────────────────────────────────

#[test]
fn recurring_crosses_leap_year_feb() {
    let reminders = generate_recurring_multi_time(2024, 2, 28, &["09:00"], 3, "闰年");

    assert_eq!(reminders.len(), 3);
    assert_eq!(reminders[0].0, "2024-02-28T09:00:00Z");
    assert_eq!(reminders[1].0, "2024-02-29T09:00:00Z"); // leap day
    assert_eq!(reminders[2].0, "2024-03-01T09:00:00Z");
}

// ── Test: year boundary crossing ─────────────────────────────────────────

#[test]
fn recurring_crosses_year_boundary() {
    let reminders = generate_recurring_multi_time(2025, 12, 31, &["23:00"], 2, "跨年");

    assert_eq!(reminders.len(), 2);
    assert_eq!(reminders[0].0, "2025-12-31T23:00:00Z");
    assert_eq!(reminders[1].0, "2026-01-01T23:00:00Z");
}

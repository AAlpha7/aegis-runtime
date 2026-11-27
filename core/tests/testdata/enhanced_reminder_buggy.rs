// === MANDATORY: these imports and boilerplate must appear exactly as shown ===
use std::cell::RefCell;

#[link(wasm_import_module = "aegis")]
extern "C" {
    fn host_fetch(url_ptr: i32, url_len: i32) -> i32;
    fn host_write_output(ptr: i32, len: i32);
    fn host_get_payload_len() -> i32;
    fn host_get_payload(dest_ptr: i32);
    fn host_get_fetch_response_len() -> i32;
    fn host_get_fetch_response(dest_ptr: i32) -> i32;
}

thread_local! {
    static OUTPUT_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn emit_output<T: serde::Serialize>(output: &T) {
    let json = serde_json::to_vec(output).unwrap_or_default();
    OUTPUT_BUF.with(|buf| {
        *buf.borrow_mut() = json;
        let b = buf.borrow();
        unsafe { host_write_output(b.as_ptr() as i32, b.len() as i32) };
    });
}
// === END MANDATORY ===

// Payload structure matching the skill design
#[derive(serde::Deserialize)]
struct Payload {
    #[serde(default)]
    reminder_type: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    start_datetime: Option<String>,
    #[serde(default)]
    recurrence: Option<String>,
    #[serde(default)]
    times: Option<Vec<String>>,
    #[serde(default)]
    day_of_week: Option<u32>,
    #[serde(default)]
    day_of_month: Option<u32>,
    #[serde(default)]
    custom_interval_minutes: Option<u32>,
    #[serde(default)]
    end_datetime: Option<String>,
    #[serde(default)]
    count: Option<u32>,
}

#[derive(serde::Serialize)]
struct Reminder {
    index: u32,
    scheduled_time: String,
    message: String,
    recurrence_info: String,
}

#[derive(serde::Serialize)]
struct Output {
    status: String,
    message: String,
    reminders: Vec<Reminder>,
}

// Date/time utilities - parse ISO 8601 datetime
// Format: "2024-01-15T09:00:00" or "2024-01-15T09:00:00Z"
fn parse_datetime(s: &str) -> Option<(u32, u32, u32, u32, u32)> {
    // Simple ISO 8601 parsing: YYYY-MM-DDTHH:MM:SS
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return None;
    }

    let date_parts: Vec<&str> = parts[0].split('-').collect();
    let time_parts: Vec<&str> = parts[1].trim_end_matches('Z').split(':').collect();

    if date_parts.len() != 3 || time_parts.len() < 2 {
        return None;
    }

    let year = date_parts[0].parse().ok()?;
    let month = date_parts[1].parse().ok()?;
    let day = date_parts[2].parse().ok()?;
    let hour = time_parts[0].parse().ok()?;
    let minute = time_parts[1].parse().ok()?;

    Some((year, month, day, hour, minute))
}

fn format_datetime((year, month, day, hour, minute): (u32, u32, u32, u32, u32)) -> String {
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:00Z", year, month, day, hour, minute)
}

fn is_valid_date(year: u32, month: u32, day: u32) -> bool {
    if month < 1 || month > 12 || day < 1 {
        return false;
    }
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    day <= days_in_month
}

fn get_days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

// Add days to a date
fn add_days(date: (u32, u32, u32, u32, u32), days: u32) -> (u32, u32, u32, u32, u32) {
    let (year, month, day, hour, minute) = date;
    let mut new_day = day + days;
    let mut new_month = month;
    let mut new_year = year;

    while new_day > get_days_in_month(new_year, new_month) {
        new_day -= get_days_in_month(new_year, new_month);
        new_month += 1;
        if new_month > 12 {
            new_month = 1;
            new_year += 1;
        }
    }
    (new_year, new_month, new_day, hour, minute)
}

// Add months to a date
fn add_months(date: (u32, u32, u32, u32, u32), months: u32) -> (u32, u32, u32, u32, u32) {
    let (year, month, day, hour, minute) = date;
    let total_months = (year * 12 + month - 1) + months;
    let new_year = total_months / 12;
    let new_month = (total_months % 12) + 1;
    let new_day = day.min(get_days_in_month(new_year, new_month));
    (new_year, new_month, new_day, hour, minute)
}

// Get day of week (0=Sunday, 1=Monday, ..., 6=Saturday)
fn get_day_of_week(year: u32, month: u32, day: u32) -> u32 {
    // Zeller's congruence adaptation
    let m = if month < 3 { month + 12 } else { month };
    let y = if month < 3 { year - 1 } else { year };
    let k = y % 100;
    let j = y / 100;
    let h = (day + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 - 2 * j) % 7;
    // Convert to 0=Sunday
    (h + 6) % 7
}

fn get_next_matching_day(
    current: (u32, u32, u32, u32, u32),
    target_day_of_week: u32,
) -> (u32, u32, u32, u32, u32) {
    let (year, month, day, hour, minute) = current;
    for i in 0..7 {
        let check_date = add_days((year, month, day, hour, minute), i);
        let dow = get_day_of_week(check_date.0, check_date.1, check_date.2);
        if dow == target_day_of_week {
            return check_date;
        }
    }
    current
}

fn parse_time(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let hour: u32 = parts[0].parse().ok()?;
    let minute: u32 = parts[1].parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

fn apply_time(date: (u32, u32, u32, u32, u32), time: (u32, u32)) -> (u32, u32, u32, u32, u32) {
    (date.0, date.1, date.2, time.0, time.1)
}

#[no_mangle]
pub extern "C" fn run() {
    // Get payload
    let payload_len = unsafe { host_get_payload_len() } as usize;
    if payload_len == 0 {
        emit_output(&Output {
            status: "error".to_string(),
            message: "Empty payload".to_string(),
            reminders: vec![],
        });
        return;
    }

    let mut payload_buf = vec![0u8; payload_len];
    unsafe { host_get_payload(payload_buf.as_mut_ptr() as i32) };

    let payload: Payload = match serde_json::from_slice(&payload_buf) {
        Ok(p) => p,
        Err(e) => {
            emit_output(&Output {
                status: "error".to_string(),
                message: format!("Invalid payload: {}", e),
                reminders: vec![],
            });
            return;
        }
    };

    // Validate required fields
    let reminder_type = payload.reminder_type.as_deref().unwrap_or("one_time");
    let message = match &payload.message {
        Some(m) if !m.is_empty() => m.clone(),
        _ => {
            emit_output(&Output {
                status: "error".to_string(),
                message: "Missing or empty message".to_string(),
                reminders: vec![],
            });
            return;
        }
    };

    let start_datetime_str = match &payload.start_datetime {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            emit_output(&Output {
                status: "error".to_string(),
                message: "Missing start_datetime".to_string(),
                reminders: vec![],
            });
            return;
        }
    };

    let start_date = match parse_datetime(&start_datetime_str) {
        Some(d) => d,
        None => {
            emit_output(&Output {
                status: "error".to_string(),
                message: "Invalid start_datetime format (expected ISO 8601: YYYY-MM-DDTHH:MM:SS)".to_string(),
                reminders: vec![],
            });
            return;
        }
    };

    // Parse times
    let times: Vec<(u32, u32)> = match &payload.times {
        Some(times_list) if !times_list.is_empty() => {
            let mut parsed = Vec::new();
            for t in times_list {
                match parse_time(t) {
                    Some(time) => parsed.push(time),
                    None => {
                        emit_output(&Output {
                            status: "error".to_string(),
                            message: format!("Invalid time format: {}", t),
                            reminders: vec![],
                        });
                        return;
                    }
                }
            }
            parsed
        }
        _ => {
            // Default to start time's time
            vec![(start_date.3, start_date.4)]
        }
    };

    // Parse end datetime if provided
    let end_date = payload.end_datetime.as_ref().and_then(|s| {
        if s.is_empty() {
            None
        } else {
            parse_datetime(s)
        }
    });

    // Parse recurrence
    let recurrence = payload.recurrence.as_deref().unwrap_or("none");

    // Get max count
    let max_count = payload.count.unwrap_or(100) as usize;

    // Generate reminders
    let mut reminders: Vec<Reminder> = Vec::new();
    let mut current = start_date;
    let mut index: u32 = 0;

    if reminder_type == "one_time" {
        // One-time reminder: use all specified times on the start date
        for time in &times {
            let scheduled = apply_time(current, *time);
            reminders.push(Reminder {
                index,
                scheduled_time: format_datetime(scheduled),
                message: message.clone(),
                recurrence_info: "One-time".to_string(),
            });
            index += 1;
        }
    } else {
        // Recurring reminder
        let recurrence_info = match recurrence {
            "daily" => "Daily".to_string(),
            "weekly" => {
                let dow = payload.day_of_week.unwrap_or(0);
                format!("Weekly on day {}", dow)
            }
            "monthly" => {
                let dom = payload.day_of_month.unwrap_or(1);
                format!("Monthly on day {}", dom)
            }
            "custom" => {
                let interval = payload.custom_interval_minutes.unwrap_or(60);
                format!("Every {} minutes", interval)
            }
            _ => "Custom".to_string(),
        };

        // Handle day-of-week constraint
        if recurrence == "weekly" {
            if let Some(target_dow) = payload.day_of_week {
                current = get_next_matching_day(current, target_dow);
            }
        }

        // For custom interval, generate directly without the times array
        if recurrence == "custom" {
            let interval = payload.custom_interval_minutes.unwrap_or(60) as u64;
            while (index as usize) < max_count {
                if let Some(end) = end_date {
                    if current.0 > end.0
                        || (current.0 == end.0 && current.1 > end.1)
                        || (current.0 == end.0 && current.1 == end.1 && current.2 > end.2)
                        || (current.0 == end.0 && current.1 == end.1 && current.2 == end.2 && current.3 > end.3)
                        || (current.0 == end.0 && current.1 == end.1 && current.2 == end.2 && current.3 == end.3 && current.4 > end.4)
                    {
                        break;
                    }
                }
                reminders.push(Reminder {
                    index,
                    scheduled_time: format_datetime(current),
                    message: message.clone(),
                    recurrence_info: recurrence_info.clone(),
                });
                index += 1;
                // Advance by interval minutes
                let total_minutes = (current.3 as u64 * 60 + current.4 as u64) + interval;
                let new_hour = (total_minutes / 60) % 24;
                let new_day_offset = total_minutes / 1440;
                let new_minute = total_minutes % 60;
                current = add_days(current, new_day_offset as u32);
                current = (current.0, current.1, current.2, new_hour as u32, new_minute as u32);
                if index > 1000 { break; }
            }
        } else {
            // daily / weekly / monthly: iterate dates, apply times array
            while (index as usize) < max_count {
                if let Some(end) = end_date {
                    if current.0 > end.0
                        || (current.0 == end.0 && current.1 > end.1)
                        || (current.0 == end.0 && current.1 == end.1 && current.2 > end.2)
                    {
                        break;
                    }
                }
                for time in &times {
                    if index as usize >= max_count { break; }
                    let _ = time;
                    reminders.push(Reminder {
                        index,
                        scheduled_time: format_datetime(current),
                        message: message.clone(),
                        recurrence_info: recurrence_info.clone(),
                    });
                    index += 1;
                }
                // Advance to next occurrence
                match recurrence {
                    "daily" => { current = add_days(current, 1); }
                    "weekly" => {
                        if let Some(target_dow) = payload.day_of_week {
                            current = get_next_matching_day(add_days(current, 1), target_dow);
                        } else {
                            current = add_days(current, 7);
                        }
                    }
                    "monthly" => {
                        current = add_months(current, 1);
                        if let Some(dom) = payload.day_of_month {
                            current = (current.0, current.1, dom.min(get_days_in_month(current.0, current.1)), current.3, current.4);
                        }
                    }
                    _ => { break; }
                }
                if index > 1000 { break; }
            }
        }
    }

    emit_output(&Output {
        status: "success".to_string(),
        message: format!("Generated {} reminders", reminders.len()),
        reminders,
    });
}

#[no_mangle]
pub extern "C" fn alloc(size: i32) -> i32 {
    if size <= 0 || size > 4 * 1024 * 1024 { return 0; }
    let Ok(layout) = std::alloc::Layout::from_size_align(size as usize, 1) else { return 0; };
    unsafe { std::alloc::alloc(layout) as i32 }
}

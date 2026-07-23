//! Formatting utilities - size, time, mode display

/// Format file size compactly (e.g., "1.2K", "3.4M", "5.6G")
pub(super) fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}M", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}K", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Format time in local timezone
/// Today: "01:25 pm", Other days: "23-02-25" (YY-MM-DD)
pub(super) fn format_time(time: std::time::SystemTime) -> String {
    use std::time::UNIX_EPOCH;

    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let timestamp = duration.as_secs() as i64;

    // Get local time components using libc
    #[cfg(unix)]
    let (year, month, day, hour, min, local_days) = {
        use std::mem::MaybeUninit;

        let mut tm = MaybeUninit::<libc::tm>::uninit();
        let time_t = timestamp as libc::time_t;

        // SAFETY: localtime_r is thread-safe and writes to our buffer
        let result = unsafe { libc::localtime_r(&time_t, tm.as_mut_ptr()) };

        if result.is_null() {
            return "???".to_string();
        }

        let tm = unsafe { tm.assume_init() };
        let year = tm.tm_year + 1900;
        let month = tm.tm_mon + 1;
        let day = tm.tm_mday;
        let hour = tm.tm_hour;
        let min = tm.tm_min;
        // Days since epoch in local time (tm_yday + years worth of days)
        let local_days =
            tm.tm_yday as i64 + (year as i64 - 1970) * 365 + ((year as i64 - 1969) / 4);

        (year, month, day, hour, min, local_days)
    };

    #[cfg(windows)]
    let (year, month, day, hour, min, local_days) = {
        use std::mem::MaybeUninit;

        let mut tm = MaybeUninit::<libc::tm>::uninit();
        let time_t = timestamp as libc::time_t;
        let result = unsafe { libc::localtime_s(tm.as_mut_ptr(), &time_t) };

        if result != 0 {
            // Fallback to UTC
            let secs_per_day = 86400i64;
            let days = timestamp / secs_per_day;
            let h = ((timestamp % secs_per_day) / 3600) as i32;
            let m = ((timestamp % 3600) / 60) as i32;
            (1970i32, 1i32, 1i32, h, m, days)
        } else {
            let tm = unsafe { tm.assume_init() };
            let year = tm.tm_year + 1900;
            let month = tm.tm_mon + 1;
            let day = tm.tm_mday;
            let hour = tm.tm_hour;
            let min = tm.tm_min;

            // Calculate days since epoch for "today" comparison
            let mut days_since_epoch = 0i64;
            for y in 1970..year {
                days_since_epoch += if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                    366
                } else {
                    365
                };
            }
            let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            let days_in_month = [
                31,
                if is_leap { 29 } else { 28 },
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ];
            for &d in &days_in_month[..month as usize - 1] {
                days_since_epoch += d as i64;
            }
            days_since_epoch += (day - 1) as i64;

            (year, month, day, hour, min, days_since_epoch)
        }
    };

    // Get current local time for "today" comparison
    let now_timestamp = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    #[cfg(unix)]
    let current_local_days = {
        use std::mem::MaybeUninit;

        let mut tm = MaybeUninit::<libc::tm>::uninit();
        let time_t = now_timestamp as libc::time_t;
        let result = unsafe { libc::localtime_r(&time_t, tm.as_mut_ptr()) };

        if result.is_null() {
            0i64
        } else {
            let tm = unsafe { tm.assume_init() };
            let now_year = (tm.tm_year + 1900) as i64;
            tm.tm_yday as i64 + (now_year - 1970) * 365 + ((now_year - 1969) / 4)
        }
    };

    #[cfg(windows)]
    let current_local_days = {
        use std::mem::MaybeUninit;

        let mut tm = MaybeUninit::<libc::tm>::uninit();
        let time_t = now_timestamp as libc::time_t;
        let result = unsafe { libc::localtime_s(tm.as_mut_ptr(), &time_t) };

        if result != 0 {
            0i64
        } else {
            let tm = unsafe { tm.assume_init() };
            let year = tm.tm_year + 1900;
            let month = tm.tm_mon + 1;
            let day = tm.tm_mday;

            // Calculate days since epoch
            let mut days = 0i64;
            for y in 1970..year {
                days += if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                    366
                } else {
                    365
                };
            }
            let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            let days_in_month = [
                31,
                if is_leap { 29 } else { 28 },
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ];
            for &d in &days_in_month[..month as usize - 1] {
                days += d as i64;
            }
            days += (day - 1) as i64;

            days
        }
    };

    let is_today = local_days == current_local_days;

    if is_today {
        // Today - show time like "01:25 pm"
        let (hour12, ampm) = if hour == 0 {
            (12, "am")
        } else if hour < 12 {
            (hour, "am")
        } else if hour == 12 {
            (12, "pm")
        } else {
            (hour - 12, "pm")
        };
        format!("{:02}:{:02} {}", hour12, min, ampm)
    } else {
        // Different day - show date like "23-02-25" (YY-MM-DD)
        format!("{:02}-{:02}-{:02}", year % 100, month, day)
    }
}

/// Format file mode/permissions as octal (e.g., "755", "644")
pub(super) fn format_mode(mode: u32) -> String {
    // On Unix, mode contains full permission bits
    // On Windows, we use a simplified mode
    #[cfg(unix)]
    {
        // Extract last 3 octal digits (user, group, other permissions)
        format!("{:03o}", mode & 0o777)
    }

    #[cfg(windows)]
    {
        // Simple display for Windows - just show if read-only
        if mode == 0o444 {
            "r--".to_string()
        } else {
            "rw-".to_string()
        }
    }
}

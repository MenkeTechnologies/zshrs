//! Date/time utilities - port of Modules/datetime.c
//!
//! Provides strftime builtin and EPOCHSECONDS/EPOCHREALTIME/epochtime parameters.

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Get current time as epoch seconds
pub fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64
}

/// Get current time as high-resolution epoch time (float)
pub fn epoch_realtime() -> f64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    now.as_secs() as f64 + now.subsec_nanos() as f64 * 1e-9
}

/// Get current time as [seconds, nanoseconds] array
pub fn epoch_time() -> (i64, i64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    (now.as_secs() as i64, now.subsec_nanos() as i64)
}

/// Format time using strftime-style format
pub fn strftime(
    format: &str,
    timestamp: Option<i64>,
    nanoseconds: Option<i64>,
) -> Result<String, String> {
    let (secs, nanos) = if let Some(ts) = timestamp {
        (ts, nanoseconds.unwrap_or(0))
    } else {
        let (s, n) = epoch_time();
        (s, n)
    };

    let dt: DateTime<Local> = match Local.timestamp_opt(secs, nanos as u32) {
        chrono::LocalResult::Single(dt) => dt,
        chrono::LocalResult::Ambiguous(dt, _) => dt,
        chrono::LocalResult::None => return Err("unable to convert to time".to_string()),
    };

    let mut result = format.to_string();

    result = result.replace("%%", "\x00");
    result = result.replace("%Y", &dt.format("%Y").to_string());
    result = result.replace("%y", &dt.format("%y").to_string());
    result = result.replace("%m", &dt.format("%m").to_string());
    result = result.replace("%d", &dt.format("%d").to_string());
    result = result.replace("%H", &dt.format("%H").to_string());
    result = result.replace("%M", &dt.format("%M").to_string());
    result = result.replace("%S", &dt.format("%S").to_string());
    result = result.replace("%j", &dt.format("%j").to_string());
    result = result.replace("%w", &dt.format("%w").to_string());
    result = result.replace("%u", &dt.format("%u").to_string());
    result = result.replace("%U", &dt.format("%U").to_string());
    result = result.replace("%W", &dt.format("%W").to_string());
    result = result.replace("%a", &dt.format("%a").to_string());
    result = result.replace("%A", &dt.format("%A").to_string());
    result = result.replace("%b", &dt.format("%b").to_string());
    result = result.replace("%B", &dt.format("%B").to_string());
    result = result.replace("%c", &dt.format("%c").to_string());
    result = result.replace("%x", &dt.format("%x").to_string());
    result = result.replace("%X", &dt.format("%X").to_string());
    result = result.replace("%p", &dt.format("%p").to_string());
    result = result.replace("%P", &dt.format("%P").to_string());
    result = result.replace("%Z", &dt.format("%Z").to_string());
    result = result.replace("%z", &dt.format("%z").to_string());
    result = result.replace("%e", &dt.format("%e").to_string());
    result = result.replace("%k", &dt.format("%k").to_string());
    result = result.replace("%l", &dt.format("%l").to_string());
    result = result.replace("%n", "\n");
    result = result.replace("%t", "\t");
    result = result.replace("%s", &secs.to_string());

    result = result.replace("%N", &format!("{:09}", nanos));
    result = result.replace("%.N", &format!(".{:09}", nanos));
    result = result.replace("%3N", &format!("{:03}", nanos / 1_000_000));
    result = result.replace("%6N", &format!("{:06}", nanos / 1_000));
    result = result.replace("%9N", &format!("{:09}", nanos));

    result = result.replace('\x00', "%");

    Ok(result)
}

/// Parse a time string using strptime-style format
pub fn strptime(format: &str, input: &str) -> Result<i64, String> {
    let dt = NaiveDateTime::parse_from_str(input, format)
        .map_err(|e| format!("format not matched: {}", e))?;

    let local = Local.from_local_datetime(&dt);
    match local {
        chrono::LocalResult::Single(dt) => Ok(dt.timestamp()),
        chrono::LocalResult::Ambiguous(dt, _) => Ok(dt.timestamp()),
        chrono::LocalResult::None => Err("unable to convert to time".to_string()),
    }
}

/// Options for strftime builtin
#[derive(Debug, Default)]
pub struct StrftimeOptions {
    pub no_newline: bool,
    pub quiet: bool,
    pub reverse: bool,
    pub scalar: Option<String>,
}

/// Execute the strftime builtin
pub fn builtin_strftime(args: &[&str], options: &StrftimeOptions) -> (i32, String) {
    if args.is_empty() {
        return (1, "strftime: format expected\n".to_string());
    }

    let format = args[0];

    if options.reverse {
        if args.len() < 2 {
            return (1, "strftime: timestring expected\n".to_string());
        }

        match strptime(format, args[1]) {
            Ok(timestamp) => {
                if options.scalar.is_some() {
                    (0, timestamp.to_string())
                } else {
                    (0, format!("{}\n", timestamp))
                }
            }
            Err(e) => {
                if options.quiet {
                    (1, String::new())
                } else {
                    (1, format!("strftime: {}\n", e))
                }
            }
        }
    } else {
        let timestamp = if args.len() > 1 {
            match args[1].parse::<i64>() {
                Ok(ts) => Some(ts),
                Err(_) => {
                    return (
                        1,
                        format!("strftime: {}: invalid decimal number\n", args[1]),
                    )
                }
            }
        } else {
            None
        };

        let nanoseconds = if args.len() > 2 {
            match args[2].parse::<i64>() {
                Ok(ns) if ns >= 0 && ns <= 999_999_999 => Some(ns),
                Ok(_) => {
                    return (
                        1,
                        format!("strftime: {}: invalid nanosecond value\n", args[2]),
                    )
                }
                Err(_) => {
                    return (
                        1,
                        format!("strftime: {}: invalid decimal number\n", args[2]),
                    )
                }
            }
        } else {
            None
        };

        match strftime(format, timestamp, nanoseconds) {
            Ok(result) => {
                let output = if options.no_newline || options.scalar.is_some() {
                    result
                } else {
                    format!("{}\n", result)
                };
                (0, output)
            }
            Err(e) => (1, format!("strftime: {}\n", e)),
        }
    }
}

/// Format a duration in human-readable form
pub fn format_duration(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let mins = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{}d {}h {}m {}s", days, hours, mins, secs)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Get current date/time info as a hashmap (for TZ-aware operations)
pub fn get_datetime_info() -> std::collections::HashMap<String, String> {
    let now = Local::now();
    let mut info = std::collections::HashMap::new();

    info.insert("year".to_string(), now.format("%Y").to_string());
    info.insert("month".to_string(), now.format("%m").to_string());
    info.insert("day".to_string(), now.format("%d").to_string());
    info.insert("hour".to_string(), now.format("%H").to_string());
    info.insert("minute".to_string(), now.format("%M").to_string());
    info.insert("second".to_string(), now.format("%S").to_string());
    info.insert("weekday".to_string(), now.format("%A").to_string());
    info.insert("monthname".to_string(), now.format("%B").to_string());
    info.insert("timezone".to_string(), now.format("%Z").to_string());
    info.insert("offset".to_string(), now.format("%z").to_string());
    info.insert("epoch".to_string(), now.timestamp().to_string());
    info.insert(
        "iso8601".to_string(),
        now.format("%Y-%m-%dT%H:%M:%S%z").to_string(),
    );

    info
}

/// Convert between timezones
pub fn convert_timezone(timestamp: i64, to_utc: bool) -> i64 {
    if to_utc {
        let dt: DateTime<Local> = Local
            .timestamp_opt(timestamp, 0)
            .single()
            .unwrap_or_else(|| Local::now());
        dt.with_timezone(&Utc).timestamp()
    } else {
        let dt: DateTime<Utc> = Utc
            .timestamp_opt(timestamp, 0)
            .single()
            .unwrap_or_else(Utc::now);
        dt.with_timezone(&Local).timestamp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epoch_seconds() {
        let secs = epoch_seconds();
        assert!(secs > 1700000000);
    }

    #[test]
    fn test_epoch_realtime() {
        let rt = epoch_realtime();
        assert!(rt > 1700000000.0);

        let (secs, _) = epoch_time();
        assert!((rt - secs as f64).abs() < 1.0);
    }

    #[test]
    fn test_epoch_time() {
        let (secs, nanos) = epoch_time();
        assert!(secs > 1700000000);
        assert!(nanos >= 0 && nanos < 1_000_000_000);
    }

    #[test]
    fn test_strftime_basic() {
        let result = strftime("%Y-%m-%d", Some(1700000000), None).unwrap();
        assert!(result.contains("-"));

        let result = strftime("%%", None, None).unwrap();
        assert_eq!(result, "%");
    }

    #[test]
    fn test_strftime_nanoseconds() {
        let result = strftime("%N", Some(1700000000), Some(123456789)).unwrap();
        assert_eq!(result, "123456789");

        let result = strftime("%3N", Some(1700000000), Some(123456789)).unwrap();
        assert_eq!(result, "123");
    }

    #[test]
    fn test_strftime_epoch() {
        let result = strftime("%s", Some(1700000000), None).unwrap();
        assert_eq!(result, "1700000000");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(90), "1m 30s");
        assert_eq!(format_duration(3661), "1h 1m 1s");
        assert_eq!(format_duration(90061), "1d 1h 1m 1s");
    }

    #[test]
    fn test_builtin_strftime() {
        let (status, output) = builtin_strftime(&["%s"], &StrftimeOptions::default());
        assert_eq!(status, 0);
        assert!(!output.is_empty());

        let (status, _) = builtin_strftime(&[], &StrftimeOptions::default());
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_strftime_with_timestamp() {
        let (status, output) = builtin_strftime(&["%s", "1700000000"], &StrftimeOptions::default());
        assert_eq!(status, 0);
        assert!(output.contains("1700000000"));
    }

    #[test]
    fn test_get_datetime_info() {
        let info = get_datetime_info();
        assert!(info.contains_key("year"));
        assert!(info.contains_key("epoch"));
        assert!(info.contains_key("iso8601"));
    }
}

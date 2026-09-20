//! Renders the one-line status header printed above each eval's result in
//! watch mode: script path, bound inputs, run duration, last-run time.

// ponytail: not yet wired into a command (lands with watch mode, #3);
// allowed dead here so this PR can ship the header standalone with full
// test coverage.
#![allow(dead_code)]

use colored::Colorize;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Formats `t` as `HH:MM:SS UTC`, stdlib-only (no calendar dependency).
fn format_hms_utc(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let sod = secs % 86_400;
    format!(
        "{:02}:{:02}:{:02} UTC",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// Renders the status header line for one eval.
pub fn render_header(
    script: &Path,
    inputs: &[(String, PathBuf)],
    duration: Duration,
    at: SystemTime,
) -> String {
    let inputs_str = if inputs.is_empty() {
        "none".to_string()
    } else {
        inputs
            .iter()
            .map(|(name, file)| format!("{name}={}", file.display()))
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        "{} {}  {} {}  {} {}ms  {} {}",
        "script:".dimmed(),
        script.display(),
        "inputs:".dimmed(),
        inputs_str,
        "took:".dimmed(),
        duration.as_millis(),
        "at:".dimmed(),
        format_hms_utc(at),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_hms_utc_at_epoch() {
        assert_eq!(format_hms_utc(UNIX_EPOCH), "00:00:00 UTC");
    }

    #[test]
    fn format_hms_utc_mid_day() {
        // 1h 2m 3s into the epoch day
        let t = UNIX_EPOCH + Duration::from_secs(3600 + 120 + 3);
        assert_eq!(format_hms_utc(t), "01:02:03 UTC");
    }

    #[test]
    fn format_hms_utc_wraps_across_days() {
        // exactly 2 days + 1 second
        let t = UNIX_EPOCH + Duration::from_secs(2 * 86_400 + 1);
        assert_eq!(format_hms_utc(t), "00:00:01 UTC");
    }

    #[test]
    fn header_includes_script_path() {
        let out = render_header(
            Path::new("foo.dwl"),
            &[],
            Duration::from_millis(5),
            UNIX_EPOCH,
        );
        assert!(out.contains("foo.dwl"));
    }

    #[test]
    fn header_reports_no_inputs() {
        let out = render_header(
            Path::new("foo.dwl"),
            &[],
            Duration::from_millis(5),
            UNIX_EPOCH,
        );
        assert!(out.contains("none"));
    }

    #[test]
    fn header_lists_bound_inputs() {
        let inputs = vec![("payload".to_string(), PathBuf::from("p.json"))];
        let out = render_header(
            Path::new("foo.dwl"),
            &inputs,
            Duration::from_millis(5),
            UNIX_EPOCH,
        );
        assert!(out.contains("payload=p.json"));
    }

    #[test]
    fn header_includes_duration_in_millis() {
        let out = render_header(
            Path::new("foo.dwl"),
            &[],
            Duration::from_millis(42),
            UNIX_EPOCH,
        );
        assert!(out.contains("42ms"));
    }

    #[test]
    fn header_includes_timestamp() {
        let out = render_header(
            Path::new("foo.dwl"),
            &[],
            Duration::from_millis(5),
            UNIX_EPOCH,
        );
        assert!(out.contains("00:00:00 UTC"));
    }
}

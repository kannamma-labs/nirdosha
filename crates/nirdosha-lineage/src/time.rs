//! Canonical fixed-width RFC 3339 UTC, millisecond precision. Same width
//! for all inputs ⇒ lexical order == chronological order. No chrono dep
//! (civil-from-days per Hinnant).

/// Format unix-epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_rfc3339_ms(unix_ms: u64) -> String {
    let secs = unix_ms / 1000;
    let ms = (unix_ms % 1000) as u32;
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn epoch() {
        assert_eq!(super::format_rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn next_day() {
        assert_eq!(super::format_rfc3339_ms(86_400_000), "1970-01-02T00:00:00.000Z");
    }

    #[test]
    fn leap_day() {
        assert_eq!(super::format_rfc3339_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn ordering_is_chronological() {
        assert!(super::format_rfc3339_ms(0) < super::format_rfc3339_ms(86_400_000));
        assert!(super::format_rfc3339_ms(951_782_399_999) < super::format_rfc3339_ms(951_782_400_000));
    }
}

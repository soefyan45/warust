//! Local times for messages (stored as Unix seconds). The UTC offset is asked
//! of the OS once (`platform::utc_offset`), so no time-zone crate is needed.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::i18n::tr;

/// Weekday names, Sunday first, in the language that is on.
fn days() -> [&'static str; 7] {
    [
        tr("Sunday", "Minggu"),
        tr("Monday", "Senin"),
        tr("Tuesday", "Selasa"),
        tr("Wednesday", "Rabu"),
        tr("Thursday", "Kamis"),
        tr("Friday", "Jumat"),
        tr("Saturday", "Sabtu"),
    ]
}

fn offset() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| crate::platform::utc_offset().unwrap_or(0))
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn local(ts: i64) -> i64 {
    ts + offset()
}

/// Local day number since 1970-01-01.
pub fn day(ts: i64) -> i64 {
    local(ts).div_euclid(86400)
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Local hour of the day, 0–23.
pub fn hour(ts: i64) -> usize {
    (local(ts).rem_euclid(86400) / 3600) as usize
}

/// `Monday, 9 November 2026`
pub fn long_date(ts: i64) -> String {
    let months = [
        tr("January", "Januari"),
        tr("February", "Februari"),
        tr("March", "Maret"),
        "April",
        tr("May", "Mei"),
        tr("June", "Juni"),
        tr("July", "Juli"),
        tr("August", "Agustus"),
        "September",
        tr("October", "Oktober"),
        "November",
        tr("December", "Desember"),
    ];
    let d = day(ts);
    let (y, m, dd) = civil(d);
    format!("{}, {dd} {} {y}", days()[(d + 4).rem_euclid(7) as usize], months[(m - 1) as usize])
}

/// `09.43`
pub fn clock(ts: i64) -> String {
    let secs = local(ts).rem_euclid(86400);
    format!("{:02}.{:02}", secs / 3600, secs % 3600 / 60)
}

/// `09.43.12`
pub fn clock_secs(ts: i64) -> String {
    let secs = local(ts).rem_euclid(86400);
    format!("{:02}.{:02}.{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// `Today`, `Yesterday`, a weekday within a week, else `12/08/2026`.
pub fn day_label(ts: i64) -> String {
    let d = day(ts);
    let today = day(now());
    match today - d {
        0 => tr("Today", "Hari ini").to_owned(),
        1 => tr("Yesterday", "Kemarin").to_owned(),
        2..=6 => days()[(d + 4).rem_euclid(7) as usize].to_owned(),
        _ => {
            let (y, m, dd) = civil(d);
            format!("{dd:02}/{m:02}/{y}")
        }
    }
}

/// `last seen today at 09.43`, `last seen yesterday at 09.43`, else with the
/// day label (`last seen Monday at 09.43`, `last seen 12/08/2026 at 09.43`).
pub fn last_seen(ts: i64) -> String {
    let at = clock(ts);
    match day(now()) - day(ts) {
        0 => crate::trf!("last seen today at {}", "terakhir dilihat hari ini pukul {}", at),
        1 => crate::trf!("last seen yesterday at {}", "terakhir dilihat kemarin pukul {}", at),
        _ => crate::trf!("last seen {} at {}", "terakhir dilihat {} pukul {}", day_label(ts), at),
    }
}

/// For the chat list: the time today, else the day label.
pub fn short(ts: i64) -> String {
    if ts == 0 {
        return String::new();
    }
    if day(ts) == day(now()) { clock(ts) } else { day_label(ts) }
}

/// `20261109-094312`, for file names.
pub fn stamp(ts: i64) -> String {
    let (y, m, d) = civil(day(ts));
    let secs = local(ts).rem_euclid(86400);
    format!("{y}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// `09/11/2026 09.43.12`
pub fn full(ts: i64) -> String {
    let (y, m, d) = civil(day(ts));
    format!("{d:02}/{m:02}/{y} {}", clock_secs(ts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_and_dates() {
        assert_eq!(crate::platform::parse_offset("+0700"), Some(25200));
        assert_eq!(crate::platform::parse_offset("-0330"), Some(-12600));
        assert_eq!(crate::platform::parse_offset("WIB"), None);
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_401), (2025, 11, 9));
        // 1970-01-01 was a Thursday.
        assert_eq!(days()[(0i64 + 4).rem_euclid(7) as usize], "Thursday");
    }
}

//! Local wall-clock time for the Board clock (contract section 9). No crates: the UTC offset comes from
//! extern FFI on native (kernel32 on Windows, libc on Unix) and falls back to UTC on wasm.

/// Local UTC offset in minutes (east positive), for the current date. 0 when unknown.
pub fn utc_offset_minutes() -> i32 {
    let m = platform_offset_minutes();
    if (-840..=840).contains(&m) {
        m
    } else {
        0
    }
}

/// Local `(year, month, day, hour, minute, second)` right now.
pub fn local_now() -> (i32, u32, u32, u32, u32, u32) {
    local_from_epoch(macroquad::miniquad::date::now(), utc_offset_minutes())
}

/// Break epoch seconds plus an offset in minutes into calendar fields.
pub fn local_from_epoch(epoch_secs: f64, offset_min: i32) -> (i32, u32, u32, u32, u32, u32) {
    let t = epoch_secs.floor() as i64 + offset_min as i64 * 60;
    let days = t.div_euclid(86_400);
    let sod = t.rem_euclid(86_400) as u32;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let mut y = yoe + era * 400;
    if m <= 2 {
        y += 1;
    }
    (y as i32, m, d, sod / 3600, (sod % 3600) / 60, sod % 60)
}

/// `HH:MM` for the Board clock, local time.
pub fn local_hhmm() -> String {
    let (_, _, _, h, m, _) = local_now();
    format_hhmm(h, m)
}

pub fn format_hhmm(h: u32, m: u32) -> String {
    format!("{:02}:{:02}", h, m)
}

#[cfg(windows)]
fn platform_offset_minutes() -> i32 {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SystemTime {
        w: [u16; 8],
    }
    #[repr(C)]
    struct Tzi {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }
    extern "system" {
        fn GetTimeZoneInformation(info: *mut Tzi) -> u32;
    }
    let mut tzi = Tzi {
        bias: 0,
        standard_name: [0; 32],
        standard_date: SystemTime { w: [0; 8] },
        standard_bias: 0,
        daylight_name: [0; 32],
        daylight_date: SystemTime { w: [0; 8] },
        daylight_bias: 0,
    };
    // SAFETY: plain struct matching TIME_ZONE_INFORMATION, valid for writes.
    let kind = unsafe { GetTimeZoneInformation(&mut tzi) };
    let extra = match kind {
        1 => tzi.standard_bias,
        2 => tzi.daylight_bias,
        _ => 0,
    };
    -(tzi.bias + extra)
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn platform_offset_minutes() -> i32 {
    use std::os::raw::{c_char, c_int, c_long};
    #[repr(C)]
    struct Tm {
        tm_sec: c_int,
        tm_min: c_int,
        tm_hour: c_int,
        tm_mday: c_int,
        tm_mon: c_int,
        tm_year: c_int,
        tm_wday: c_int,
        tm_yday: c_int,
        tm_isdst: c_int,
        tm_gmtoff: c_long,
        tm_zone: *const c_char,
    }
    extern "C" {
        fn localtime_r(t: *const c_long, out: *mut Tm) -> *mut Tm;
    }
    let now = macroquad::miniquad::date::now() as c_long;
    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: tm matches the glibc/musl/Apple struct tm layout; localtime_r writes only into it.
    let r = unsafe { localtime_r(&now, &mut tm) };
    if r.is_null() {
        0
    } else {
        (tm.tm_gmtoff / 60) as i32
    }
}

#[cfg(not(any(windows, all(unix, not(target_arch = "wasm32")))))]
fn platform_offset_minutes() -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_in_range() {
        assert!((-840..=840).contains(&utc_offset_minutes()));
    }

    #[test]
    fn epoch_breakdown() {
        assert_eq!(local_from_epoch(0.0, 0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(local_from_epoch(0.0, -90), (1969, 12, 31, 22, 30, 0));
        // 2024-02-29 13:07:05 UTC
        assert_eq!(local_from_epoch(1_709_212_025.0, 0), (2024, 2, 29, 13, 7, 5));
        assert_eq!(local_from_epoch(1_709_212_025.0, 600), (2024, 2, 29, 23, 7, 5));
    }

    #[test]
    fn formatting() {
        assert_eq!(format_hhmm(7, 5), "07:05");
        assert_eq!(format_hhmm(23, 59), "23:59");
    }
}

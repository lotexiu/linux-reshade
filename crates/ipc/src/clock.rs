//! A hora local (`HH:MM:SS.mmm`) para os logs: a mesma do `journalctl`, para casar o que o programa fazia com uma falha do sistema.

#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(time: *const i64, out: *mut Tm) -> *mut Tm;
}

pub fn clock() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() as i64;
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: `tm` é uma `struct tm` do glibc (mesma disposição) e `secs` vive até o fim da chamada.
    if unsafe { localtime_r(&secs, &mut tm) }.is_null() {
        return secs.to_string();
    }
    format!("{:02}:{:02}:{:02}.{:03}", tm.hour, tm.min, tm.sec, now.subsec_millis())
}

/// O instante agora no formato de nome de arquivo (`2026-11-02_18-45-03`), na hora local.
pub fn file_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: como em `clock`.
    if unsafe { localtime_r(&secs, &mut tm) }.is_null() {
        return secs.to_string();
    }
    format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", tm.year + 1900, tm.mon + 1, tm.mday, tm.hour, tm.min, tm.sec)
}

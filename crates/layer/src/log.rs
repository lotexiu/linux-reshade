//! Log em arquivo, ligado só quando `LINUX_RESHADE_LOG` aponta um caminho. Sem a variável a
//! layer não escreve nada (nem em stderr), para não poluir a saída de um jogo.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

static SINK: OnceLock<Option<Mutex<File>>> = OnceLock::new();

fn sink() -> &'static Option<Mutex<File>> {
    SINK.get_or_init(|| {
        let path = std::env::var_os("LINUX_RESHADE_LOG")?;
        OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
    })
}

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

/// A hora local (`HH:MM:SS.mmm`): é a mesma do `journalctl`, para casar uma falha da GPU (Xid) com o que a layer fazia.
fn clock() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() as i64;
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: `tm` é uma `struct tm` do glibc (mesma disposição) e `secs` vive até o fim da chamada.
    if unsafe { localtime_r(&secs, &mut tm) }.is_null() {
        return format!("{secs}");
    }
    format!("{:02}:{:02}:{:02}.{:03}", tm.hour, tm.min, tm.sec, now.subsec_millis())
}

pub fn write(args: fmt::Arguments<'_>) {
    if let Some(file) = sink() {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "[linux_reshade pid={} {}] {args}", std::process::id(), clock());
        }
    }
}

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(format_args!($($arg)*)) };
}
pub(crate) use log;

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

pub fn write(args: fmt::Arguments<'_>) {
    if let Some(file) = sink() {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "[linux_reshade pid={}] {args}", std::process::id());
        }
    }
}

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(format_args!($($arg)*)) };
}
pub(crate) use log;

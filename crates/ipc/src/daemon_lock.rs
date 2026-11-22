//! A trava do daemon: um só daemon por usuário, e o carimbo do executável que o painel confere para trocar um daemon de versão antiga.

use rustix::fs::{FlockOperation, flock};
use std::fs::{File, OpenOptions};

fn open() -> std::io::Result<File> {
    let dir = super::state_dir();
    std::fs::create_dir_all(&dir)?;
    OpenOptions::new().create(true).truncate(false).write(true).read(true).open(dir.join("daemon.lock"))
}

/// Pega a trava do daemon. `None` se outro daemon já a tem. Quem pega deve guardar o `File` enquanto viver.
pub fn acquire() -> Option<File> {
    let file = open().ok()?;
    flock(&file, FlockOperation::NonBlockingLockExclusive).ok().map(|()| file)
}

/// O "carimbo" de um executável (tamanho e data): dois daemons de builds diferentes têm carimbos diferentes.
pub fn exe_stamp(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let secs = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(format!("{}:{secs}", meta.len()))
}

/// O daemon que pegou a trava diz aqui de que build ele é (para o painel notar um daemon antigo sobrevivendo a uma atualização).
pub fn publish(file: &File, stamp: &str) {
    use std::os::unix::fs::FileExt;
    let _ = file.set_len(0);
    let _ = file.write_all_at(stamp.as_bytes(), 0);
}

/// O carimbo que o daemon em execução publicou (vazio: um daemon de antes de existir o carimbo). `None` se não há daemon.
pub fn running_stamp() -> Option<String> {
    if !is_running() {
        return None;
    }
    Some(std::fs::read_to_string(super::state_dir().join("daemon.lock")).unwrap_or_default().trim().to_string())
}

/// O processo que segura a trava (lido de `/proc/locks`: vale também para daemons que não publicaram nada).
pub fn holder_pid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    let inode = std::fs::metadata(super::state_dir().join("daemon.lock")).ok()?.ino();
    let locks = std::fs::read_to_string("/proc/locks").ok()?;
    locks.lines().find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let id = f.iter().find(|w| w.matches(':').count() == 2)?;
        (f.get(1) == Some(&"FLOCK") && id.rsplit(':').next()?.parse::<u64>().ok()? == inode).then(|| f.get(4)?.parse().ok())?
    })
}

/// Há um daemon rodando?
pub fn is_running() -> bool {
    let Ok(file) = open() else { return false };
    // Se a trava compartilhada vem, ninguém a segura de forma exclusiva: não há daemon.
    flock(&file, FlockOperation::NonBlockingLockShared).is_err()
}

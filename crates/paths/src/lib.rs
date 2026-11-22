//! As pastas do usuário (XDG), num lugar só: `~/.config`, `~/.local/share`, `~/.cache` e `~/.local/state`, ou o que as variáveis
//! `XDG_*_HOME` dizem. Antes cada crate repetia a mesma conta; agora todos pedem aqui.
//!
//! Só a biblioteca padrão (vale para a layer, que roda dentro do jogo, e para o resto). As variáveis relativas são ignoradas, como manda a
//! especificação do XDG. Cada função tem uma versão `_with` que recebe o ambiente (para os testes não dependerem da máquina).

use std::ffi::OsString;
use std::path::PathBuf;

/// Como o ambiente é lido: injetado, para os testes não dependerem da máquina.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn process_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// A pasta pessoal (`$HOME`).
pub fn home_with(env: Env) -> Option<PathBuf> {
    env("HOME").map(PathBuf::from)
}

pub fn home() -> Option<PathBuf> {
    home_with(&process_env)
}

/// `$var` (se for um caminho absoluto) ou, sem ela, `$HOME/<fallback>`.
pub fn xdg_with(env: Env, var: &str, fallback: &str) -> Option<PathBuf> {
    env(var).map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home_with(env).map(|h| h.join(fallback)))
}

pub fn config_home_with(env: Env) -> Option<PathBuf> {
    xdg_with(env, "XDG_CONFIG_HOME", ".config")
}

pub fn data_home_with(env: Env) -> Option<PathBuf> {
    xdg_with(env, "XDG_DATA_HOME", ".local/share")
}

pub fn cache_home_with(env: Env) -> Option<PathBuf> {
    xdg_with(env, "XDG_CACHE_HOME", ".cache")
}

pub fn state_home_with(env: Env) -> Option<PathBuf> {
    xdg_with(env, "XDG_STATE_HOME", ".local/state")
}

/// `~/.config` (configurações).
pub fn config_home() -> Option<PathBuf> {
    config_home_with(&process_env)
}

/// `~/.local/share` (dados).
pub fn data_home() -> Option<PathBuf> {
    data_home_with(&process_env)
}

/// `~/.cache` (o que se pode refazer).
pub fn cache_home() -> Option<PathBuf> {
    cache_home_with(&process_env)
}

/// `~/.local/state` (registros e estado que sobrevive a reinícios).
pub fn state_home() -> Option<PathBuf> {
    state_home_with(&process_env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn the_variable_wins_over_home() {
        let e = env(&[("HOME", "/home/a"), ("XDG_DATA_HOME", "/dados")]);
        assert_eq!(data_home_with(&e), Some(PathBuf::from("/dados")));
        assert_eq!(config_home_with(&e), Some(PathBuf::from("/home/a/.config")));
    }

    #[test]
    fn a_relative_variable_is_ignored_as_the_xdg_spec_says() {
        let e = env(&[("HOME", "/home/a"), ("XDG_CACHE_HOME", "relativo")]);
        assert_eq!(cache_home_with(&e), Some(PathBuf::from("/home/a/.cache")));
    }

    #[test]
    fn without_home_or_variable_there_is_nothing() {
        let e = env(&[]);
        assert_eq!(state_home_with(&e), None);
        assert_eq!(home_with(&e), None);
    }

    #[test]
    fn each_kind_has_its_own_default() {
        let e = env(&[("HOME", "/h")]);
        assert_eq!(state_home_with(&e), Some(PathBuf::from("/h/.local/state")));
        assert_eq!(data_home_with(&e), Some(PathBuf::from("/h/.local/share")));
    }
}

//! Manutenção do cache de efeitos compilados: ver quanto ocupa, limpar à força e medir quanto custa compilar de novo.
//! Serve para testar os cenários de compilação (cache frio) e para ver o tempo de processamento.

use crate::compiler::{self, Compiler};
use std::path::Path;
use std::time::{Duration, Instant};
use i18n::{fmt, tr};

/// O que há no cache.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheUsage {
    /// Efeitos compilados (um diretório por efeito, resolução e macros).
    pub bundles: usize,
    /// Análises de compatibilidade guardadas.
    pub analyses: usize,
    pub bytes: u64,
}

/// O que apagar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clear {
    /// Os efeitos compilados; as análises ficam.
    Bundles,
    /// As análises de compatibilidade; os compilados ficam.
    Analyses,
    /// Tudo o que o compilador guarda.
    All,
}

fn dir_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Um diretório do cache que é um bundle compilado (ou resto de compilação interrompida).
fn is_bundle_dir(file_name: &str) -> bool {
    file_name.ends_with(".tmp") || compiler::is_bundle_name(file_name)
}

pub fn usage(compiler: &Compiler) -> CacheUsage {
    let mut out = CacheUsage::default();
    let Ok(entries) = std::fs::read_dir(&compiler.cache) else { return out };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "analysis" {
            out.analyses = crate::analysis::count_saved(compiler);
            out.bytes += dir_bytes(&e.path());
        } else if e.path().is_dir() && is_bundle_dir(&name) {
            out.bundles += 1;
            out.bytes += dir_bytes(&e.path());
        }
    }
    out
}

/// Apaga o que foi pedido e devolve quantos diretórios (ou análises) saíram. O vigia do daemon recompila o que um jogo
/// aberto usa na próxima olhada.
pub fn clear(compiler: &Compiler, what: Clear) -> usize {
    let mut removed = 0;
    let Ok(entries) = std::fs::read_dir(&compiler.cache) else { return 0 };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        let bundle = path.is_dir() && is_bundle_dir(&name);
        let analysis = name == "analysis" && path.is_dir();
        if bundle && matches!(what, Clear::Bundles | Clear::All) && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
        if analysis && matches!(what, Clear::Analyses | Clear::All) {
            removed += crate::analysis::count_saved(compiler);
            let _ = std::fs::remove_dir_all(&path);
        }
    }
    removed
}

/// Quanto custou compilar um efeito.
#[derive(Debug, Clone)]
pub struct Timing {
    pub effect: String,
    pub took: Duration,
    pub error: Option<String>,
}

/// Apaga os compilados dos efeitos e os compila de novo, um por vez, medindo cada um (resolução padrão, sem macros).
pub fn time_compiles(compiler: &Compiler, effects: &[String], mut progress: impl FnMut(&Timing)) -> Vec<Timing> {
    let (w, h) = compiler::DEFAULT_SIZE;
    effects
        .iter()
        .map(|name| {
            compiler.invalidate(name);
            let start = Instant::now();
            let result = compiler.ensure(name, w, h);
            let timing = Timing { effect: name.clone(), took: start.elapsed(), error: result.err().map(|e| e.to_string().lines().next().unwrap_or(tr("failed")).to_string()) };
            progress(&timing);
            timing
        })
        .collect()
}

/// O texto do relatório de [`time_compiles`]: total, média e os mais lentos.
pub fn report(timings: &[Timing]) -> String {
    if timings.is_empty() {
        return tr("No effects to measure.").to_string();
    }
    let total: Duration = timings.iter().map(|t| t.took).sum();
    let mut sorted: Vec<&Timing> = timings.iter().collect();
    sorted.sort_by_key(|t| std::cmp::Reverse(t.took));
    let mut out = fmt("{} effects compiled in {} s (average {} ms)\n\n", &[&timings.len(), &format!("{:.2}", total.as_secs_f64()), &format!("{:.0}", total.as_millis() as f64 / timings.len() as f64)]);
    for t in sorted {
        match &t.error {
            Some(e) => out.push_str(&format!("{:>8} ms  {}  ({})\n", t.took.as_millis(), t.effect, fmt("error: {}", &[e]))),
            None => out.push_str(&format!("{:>8} ms  {}\n", t.took.as_millis(), t.effect)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_lists_the_slowest_first() {
        let t = |n: &str, ms| Timing { effect: n.into(), took: Duration::from_millis(ms), error: None };
        let text = report(&[t("A", 10), t("B", 300), t("C", 50)]);
        let (b, c, a) = (text.find("B").unwrap(), text.find("C").unwrap(), text.find("  A").unwrap());
        assert!(b < c && c < a, "{text}");
        assert!(text.starts_with("3 effects compiled in 0.36 s"), "{text}");
        assert_eq!(report(&[]), tr("No effects to measure."));
    }
}

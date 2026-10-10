//! Analisa todos os efeitos das pastas configuradas e resume o resultado:
//! `cargo run --release -p control-core --example analyze_all`. Aproveita (e enche) o cache de análises.

use control_core::analysis::{self, Feature};
use control_core::compiler::Compiler;
use std::collections::BTreeMap;

fn main() {
    let compiler = Compiler::from_env().expect("compilador");
    let names: Vec<String> = compiler.available().into_iter().map(|(n, _)| n).collect();
    let started = std::time::Instant::now();
    let (mut ok, mut bad) = (0, 0);
    let mut reasons: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut features: BTreeMap<&str, usize> = BTreeMap::new();
    for name in &names {
        let info = analysis::ensure(&compiler, name);
        for f in &info.features {
            *features.entry(f.label()).or_default() += 1;
        }
        match info.reason() {
            None => ok += 1,
            Some(why) => {
                bad += 1;
                let key: String = why.chars().take(90).collect();
                reasons.entry(key).or_default().push(name.clone());
            }
        }
    }
    println!("{} efeitos em {:.0?}: {ok} compatíveis, {bad} incompatíveis", names.len(), started.elapsed());
    for (why, list) in &reasons {
        println!("  {:>3}  {why}", list.len());
    }
    println!("características:");
    for (f, n) in &features {
        println!("  {n:>3}  {f}");
    }
    let _ = Feature::ALL;
}

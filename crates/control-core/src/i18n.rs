//! Idiomas da interface. O texto-fonte do código é o inglês (o idioma padrão); cada outro idioma é um
//! catálogo em `locales/<código>.tsv` (uma tradução por linha: `inglês<TAB>tradução`, com `\n` e `\t`
//! escritos como `\\n` e `\\t`). Texto sem tradução aparece em inglês, nunca vazio.
//!
//! - [`tr`]: traduz um texto fixo.
//! - [`fmt`]: traduz um modelo com `{}` e preenche os valores na ordem (a ordem pode mudar na tradução usando `{1}`, `{2}`…).
//! - O idioma vem de `LR_LANG`, depois de `language` em `~/.config/linux-reshade/ui.ini`; sem nenhum, inglês.
//!   Mudar o idioma vale a partir da próxima abertura.

use ini::Ini;
use std::collections::HashMap;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Um idioma disponível: código (`en`, `pt_BR`), nome no próprio idioma e catálogo (vazio para o inglês).
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    catalog: &'static str,
}

pub const LANGUAGES: &[Language] = &[
    Language { code: "en", name: "English", catalog: "" },
    Language { code: "pt_BR", name: "Português (Brasil)", catalog: include_str!("../locales/pt_BR.tsv") },
];

pub const DEFAULT: &str = "en";
const KEY: &str = "language";

fn ui_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join(config::LOCATION_DIR).join("ui.ini"))
}

/// Acha o idioma pelo código, aceitando `pt-BR`, `pt_br.UTF-8` e `pt` (o primeiro que começa igual).
pub fn find(code: &str) -> Option<&'static Language> {
    let norm = |s: &str| s.split('.').next().unwrap_or("").replace('-', "_").to_lowercase();
    let want = norm(code);
    LANGUAGES
        .iter()
        .find(|l| norm(l.code) == want)
        .or_else(|| LANGUAGES.iter().find(|l| !want.is_empty() && norm(l.code).split('_').next() == want.split('_').next()))
}

/// O idioma que a pessoa escolheu (`None` = nenhum, vale o padrão).
pub fn saved() -> Option<String> {
    let ini = Ini::load(&ui_file()?).ok()?;
    ini.get("", KEY).map(str::to_string).filter(|v| !v.is_empty())
}

/// Guarda o idioma escolhido (vale na próxima abertura).
pub fn save(code: &str) {
    let Some(file) = ui_file() else { return };
    let mut ini = Ini::load(&file).unwrap_or_default();
    ini.set("", KEY, code);
    let _ = ini.save(&file);
}

fn current() -> &'static Language {
    static CURRENT: OnceLock<&'static Language> = OnceLock::new();
    CURRENT.get_or_init(|| {
        // Os testes do próprio crate conferem textos: sempre em inglês, seja qual for o idioma salvo na máquina.
        if cfg!(test) {
            return find(DEFAULT).expect("o inglês sempre existe");
        }
        std::env::var("LR_LANG").ok().and_then(|c| find(&c)).or_else(|| saved().and_then(|c| find(&c))).or_else(|| find(DEFAULT)).expect("o inglês sempre existe")
    })
}

/// O código do idioma em uso.
pub fn code() -> &'static str {
    current().code
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\t", "\t")
}

fn parse(text: &'static str) -> HashMap<String, String> {
    text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).map(|(en, tr)| (unescape(en), unescape(tr))).collect()
}

fn catalog() -> &'static HashMap<String, String> {
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| parse(current().catalog))
}

/// Traduz um texto fixo (o texto em inglês do código é a chave).
pub fn tr(text: &'static str) -> &'static str {
    tr_lookup(text).unwrap_or(text)
}

fn tr_lookup(text: &str) -> Option<&'static str> {
    // As traduções vivem a vida toda do programa; devolver `&'static str` evita cópia a cada redesenho.
    static LEAKED: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    let map = LEAKED.get_or_init(|| catalog().iter().map(|(k, v)| (k.clone(), &*Box::leak(v.clone().into_boxed_str()))).collect());
    map.get(text).copied()
}

/// Traduz um texto que só se conhece na hora (guardado em inglês, por exemplo numa análise em disco); sem tradução, devolve o mesmo.
pub fn tr_dyn(text: &str) -> String {
    tr_lookup(text).map_or_else(|| text.to_string(), str::to_string)
}

/// Traduz um modelo e preenche os `{}` na ordem (ou `{1}`, `{2}`… para mudar a ordem na tradução).
pub fn fmt(template: &'static str, args: &[&dyn Display]) -> String {
    fill(tr(template), args)
}

fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { out.push_str(&rest[start..]); return out };
        let name = &after[..end];
        let index = if name.is_empty() { let i = next; next += 1; Some(i) } else { name.parse::<usize>().ok().map(|n| n.saturating_sub(1)) };
        match index.and_then(|i| args.get(i)) {
            Some(arg) => out.push_str(&arg.to_string()),
            None => { out.push('{'); out.push_str(name); out.push('}') }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_order_and_by_number() {
        assert_eq!(fill("{} of {}", &[&1, &"two"]), "1 of two");
        assert_eq!(fill("{2} / {1}", &[&"a", &"b"]), "b / a");
        assert_eq!(fill("no args {x}", &[]), "no args {x}");
    }

    #[test]
    fn finds_languages_loosely() {
        assert_eq!(find("pt-BR").map(|l| l.code), Some("pt_BR"));
        assert_eq!(find("pt_BR.UTF-8").map(|l| l.code), Some("pt_BR"));
        assert_eq!(find("pt").map(|l| l.code), Some("pt_BR"));
        assert_eq!(find("en_US.UTF-8").map(|l| l.code), Some("en"));
        assert!(find("xx").is_none());
    }

    #[test]
    fn catalogs_are_well_formed() {
        for l in LANGUAGES {
            let lines = l.catalog.lines().filter(|x| !x.is_empty() && !x.starts_with('#')).count();
            assert_eq!(lines, parse(l.catalog).len(), "{}: linha sem tab ou chave repetida", l.code);
        }
    }
}

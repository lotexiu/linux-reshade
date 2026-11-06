//! Idiomas da interface. O texto-fonte do código é o inglês (o idioma padrão); cada outro idioma é um catálogo
//! `<código>.tsv` (uma tradução por linha: `inglês<TAB>tradução`, com `\n` e `\t` escritos como `\\n` e `\\t`; a linha
//! `# language: Nome do idioma` dá o nome mostrado na lista). Texto sem tradução aparece em inglês, nunca vazio.
//!
//! - Os catálogos de `locales/` entram no programa sozinhos (ver `build.rs`): acrescentar um idioma é acrescentar o arquivo.
//! - Sem recompilar: um `<código>.tsv` em `~/.local/share/linux-reshade/locales/` (ou na pasta de `LINUX_RESHADE_LOCALES`) também vale,
//!   e vence o embutido de mesmo código.
//! - [`tr`]: traduz um texto fixo. [`fmt`]: traduz um modelo com `{}` e preenche os valores na ordem (`{1}`, `{2}`… mudam a ordem).
//! - O idioma vem de `LR_LANG`, depois de `language` em `~/.config/linux-reshade/ui.ini`; sem nenhum, inglês. [`set`] troca na hora.

use ini::Ini;
use std::collections::HashMap;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

include!(concat!(env!("OUT_DIR"), "/builtin_locales.rs"));

/// Um idioma disponível: código (`en`, `pt_BR`), nome no próprio idioma e catálogo (vazio para o inglês).
#[derive(Debug)]
pub struct Language {
    pub code: String,
    pub name: String,
    catalog: &'static str,
}

pub const DEFAULT: &str = "en";
const KEY: &str = "language";

fn ui_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join(config::LOCATION_DIR).join("ui.ini"))
}

/// As pastas com catálogos de usuário.
fn user_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::var_os("LINUX_RESHADE_LOCALES").filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(d));
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(data) = data {
        dirs.push(data.join(config::LOCATION_DIR).join("locales"));
    }
    dirs
}

fn name_of(code: &str, catalog: &str) -> String {
    catalog.lines().find_map(|l| l.strip_prefix("# language:")).map_or_else(|| code.to_string(), |n| n.trim().to_string())
}

/// Todos os idiomas: o inglês, os embutidos e os da pasta do usuário (estes não existem nos testes do próprio crate).
pub fn languages() -> &'static [Language] {
    static ALL: OnceLock<Vec<Language>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = vec![Language { code: DEFAULT.to_string(), name: "English".to_string(), catalog: "" }];
        let mut add = |code: String, catalog: &'static str| {
            let language = Language { name: name_of(&code, catalog), code: code.clone(), catalog };
            match all.iter().position(|l| l.code == code) {
                Some(i) if i > 0 => all[i] = language,
                Some(_) => {}
                None => all.push(language),
            }
        };
        for (code, catalog) in BUILTIN {
            add((*code).to_string(), catalog);
        }
        if !cfg!(test) {
            for dir in user_dirs() {
                let Ok(entries) = std::fs::read_dir(dir) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "tsv")
                        && let (Some(code), Ok(text)) = (path.file_stem().map(|s| s.to_string_lossy().into_owned()), std::fs::read_to_string(&path))
                    {
                        add(code, Box::leak(text.into_boxed_str()));
                    }
                }
            }
        }
        all
    })
}

fn norm(s: &str) -> String {
    s.split('.').next().unwrap_or("").replace('-', "_").to_lowercase()
}

/// Acha o idioma pelo código, aceitando `pt-BR`, `pt_br.UTF-8` e `pt` (o primeiro que começa igual).
pub fn find(code: &str) -> Option<&'static Language> {
    let want = norm(code);
    let all = languages();
    all.iter().find(|l| norm(&l.code) == want).or_else(|| all.iter().find(|l| !want.is_empty() && norm(&l.code).split('_').next() == want.split('_').next()))
}

/// O idioma que a pessoa escolheu (`None` = nenhum, vale o padrão).
pub fn saved() -> Option<String> {
    let ini = Ini::load(&ui_file()?).ok()?;
    ini.get("", KEY).map(str::to_string).filter(|v| !v.is_empty())
}

/// Guarda o idioma escolhido para as próximas aberturas.
pub fn save(code: &str) {
    let Some(file) = ui_file() else { return };
    let mut ini = Ini::load(&file).unwrap_or_default();
    ini.set("", KEY, code);
    let _ = ini.save(&file);
}

fn initial() -> usize {
    // Os testes conferem textos: rodando sob o `cargo` (test, run), o idioma salvo na máquina não vale (só `LR_LANG`).
    if cfg!(test) {
        return 0;
    }
    let under_cargo = std::env::var_os("CARGO_MANIFEST_DIR").is_some();
    let wanted = std::env::var("LR_LANG").ok().and_then(|c| find(&c)).or_else(|| if under_cargo { None } else { saved().and_then(|c| find(&c)) });
    wanted.and_then(|w| languages().iter().position(|l| l.code == w.code)).unwrap_or(0)
}

fn current_index() -> &'static RwLock<usize> {
    static CURRENT: OnceLock<RwLock<usize>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new(initial()))
}

fn current() -> &'static Language {
    let index = *current_index().read().unwrap_or_else(|e| e.into_inner());
    &languages()[index]
}

/// O código do idioma em uso.
pub fn code() -> &'static str {
    &current().code
}

/// Troca o idioma agora (não grava; ver [`save`]). Devolve se o código existe.
pub fn set(code: &str) -> bool {
    let Some(language) = find(code) else { return false };
    let index = languages().iter().position(|l| l.code == language.code).unwrap_or(0);
    *current_index().write().unwrap_or_else(|e| e.into_inner()) = index;
    true
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\t", "\t")
}

fn parse(text: &'static str) -> HashMap<String, &'static str> {
    // As traduções vivem a vida toda do programa; devolver `&'static str` evita cópia a cada redesenho.
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(en, tr)| (unescape(en), &*Box::leak(unescape(tr).into_boxed_str())))
        .collect()
}

/// O catálogo do idioma em uso, lido uma vez por idioma.
fn lookup(text: &str) -> Option<&'static str> {
    static CATALOGS: OnceLock<RwLock<HashMap<String, &'static HashMap<String, &'static str>>>> = OnceLock::new();
    let language = current();
    if language.catalog.is_empty() {
        return None;
    }
    let catalogs = CATALOGS.get_or_init(|| RwLock::new(HashMap::new()));
    let cached = catalogs.read().unwrap_or_else(|e| e.into_inner()).get(&language.code).copied();
    let catalog = cached.unwrap_or_else(|| {
        let leaked: &'static HashMap<String, &'static str> = Box::leak(Box::new(parse(language.catalog)));
        catalogs.write().unwrap_or_else(|e| e.into_inner()).insert(language.code.clone(), leaked);
        leaked
    });
    catalog.get(text).copied()
}

/// Traduz um texto fixo (o texto em inglês do código é a chave).
pub fn tr(text: &'static str) -> &'static str {
    lookup(text).unwrap_or(text)
}

/// Traduz um texto que só se conhece na hora (guardado em inglês, por exemplo numa análise em disco); sem tradução, devolve o mesmo.
pub fn tr_dyn(text: &str) -> String {
    lookup(text).map(str::to_string).or_else(|| translate_template(text)).unwrap_or_else(|| text.to_string())
}

/// Os modelos do catálogo (as chaves com `{}`) do idioma em uso, cada um com a tradução.
fn templates() -> &'static [(Vec<String>, &'static str)] {
    static CACHE: OnceLock<RwLock<HashMap<String, &'static [(Vec<String>, &'static str)]>>> = OnceLock::new();
    let language = current();
    let cache = CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(found) = cache.read().unwrap_or_else(|e| e.into_inner()).get(&language.code).copied() {
        return found;
    }
    let mut list: Vec<(Vec<String>, &'static str)> = Vec::new();
    if !language.catalog.is_empty() {
        for (key, translation) in parse(language.catalog) {
            if key.contains("{}") {
                list.push((key.split("{}").map(str::to_string).collect(), translation));
            }
        }
    }
    let leaked: &'static [(Vec<String>, &'static str)] = Box::leak(list.into_boxed_slice());
    cache.write().unwrap_or_else(|e| e.into_inner()).insert(language.code.clone(), leaked);
    leaked
}

/// Casa `text` com um modelo do catálogo (`step {}: …`) e devolve a tradução com os trechos de `{}` no lugar.
fn translate_template(text: &str) -> Option<String> {
    for (parts, translation) in templates() {
        let (first, last) = (parts.first()?, parts.last()?);
        if parts.len() < 2 || !text.starts_with(first.as_str()) || text.len() < first.len() + last.len() || !text.ends_with(last.as_str()) {
            continue;
        }
        let middle = &text[first.len()..text.len() - last.len()];
        let mut captures: Vec<&str> = Vec::new();
        let mut rest = middle;
        let mut ok = true;
        for sep in &parts[1..parts.len() - 1] {
            match (sep.is_empty(), rest.find(sep.as_str())) {
                (false, Some(at)) => {
                    captures.push(&rest[..at]);
                    rest = &rest[at + sep.len()..];
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        captures.push(rest);
        // O que cabe em `{}` pode ser outra mensagem (`etapa 1: não suportado: …`): traduz por dentro também.
        let inner: Vec<String> = captures.iter().map(|c| if c.len() > 8 && c.contains(' ') { tr_dyn(c) } else { (*c).to_string() }).collect();
        let args: Vec<&dyn Display> = inner.iter().map(|c| c as &dyn Display).collect();
        return Some(fill(translation, &args));
    }
    None
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
    fn templates_match_messages_with_values() {
        let key = "step {}: the bundle was compiled for {}x{}";
        let parts: Vec<String> = key.split("{}").map(str::to_string).collect();
        let text = "step 2: the bundle was compiled for 1920x1080";
        // O casamento não depende do idioma: o catálogo de teste é só inglês, então conferimos as peças diretamente.
        assert!(text.starts_with(&parts[0]) && text.ends_with(&parts[3]));
        assert_eq!(fill("passo {1}: bundle para {2}x{3}", &[&"2", &"1920", &"1080"]), "passo 2: bundle para 1920x1080");
    }

    #[test]
    fn finds_languages_loosely() {
        assert_eq!(find("pt-BR").map(|l| l.code.as_str()), Some("pt_BR"));
        assert_eq!(find("pt_BR.UTF-8").map(|l| l.code.as_str()), Some("pt_BR"));
        assert_eq!(find("pt").map(|l| l.code.as_str()), Some("pt_BR"));
        assert_eq!(find("en_US.UTF-8").map(|l| l.code.as_str()), Some("en"));
        assert!(find("xx").is_none());
    }

    #[test]
    fn catalogs_are_well_formed() {
        for l in languages() {
            let lines = l.catalog.lines().filter(|x| !x.is_empty() && !x.starts_with('#')).count();
            assert_eq!(lines, parse(l.catalog).len(), "{}: linha sem tab ou chave repetida", l.code);
        }
    }
}

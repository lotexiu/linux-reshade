//! Presets no formato do ReShade, sem conversão: um preset da comunidade serve como está.
//!
//! Seção global: `Techniques` (só as ligadas, `Nome@Arquivo.fx`), `TechniqueSorting` (todas, na ordem de
//! execução), `PreprocessorDefinitions` (macros do preset) e `Key<técnica>` (atalho que liga/desliga a
//! técnica). Uma seção por efeito, `[Arquivo.fx]`, com as macros dele e o valor de cada uniform.
//! As regras estão em `docs/reshade-estudo.md`, seção 3.
//!
//! [`game`] diz qual preset vale para um jogo.
//!
//! O arquivo é um [`ini::Ini`]: ao regravar, o que não foi mexido (comentários, chaves desconhecidas, CRLF,
//! espaços) sai exatamente como entrou.

pub mod game;

use std::fmt;
use std::io;
use std::path::Path;

use ini::Ini;

const TECHNIQUES: &str = "Techniques";
const SORTING: &str = "TechniqueSorting";
const DEFINES: &str = "PreprocessorDefinitions";

/// Uma técnica: `Nome@Arquivo.fx`. O ReShade também aceita só `Nome` ao ler; então `file` pode faltar.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TechId {
    pub name: String,
    pub file: Option<String>,
}

impl TechId {
    pub fn new(name: &str, file: &str) -> Self {
        Self { name: name.to_string(), file: Some(file.to_string()) }
    }

    pub fn parse(text: &str) -> Self {
        match text.split_once('@') {
            Some((name, file)) => Self { name: name.to_string(), file: Some(file.to_string()) },
            None => Self { name: text.to_string(), file: None },
        }
    }

    /// Mesma técnica? Sem o arquivo de um dos lados, basta o nome.
    pub fn matches(&self, other: &TechId) -> bool {
        self.name == other.name && (self.file.is_none() || other.file.is_none() || self.file == other.file)
    }
}

impl fmt::Display for TechId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.file {
            Some(file) => write!(f, "{}@{file}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// Uma técnica na ordem de execução.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ordered {
    pub id: TechId,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Resolved {
    /// Todas as técnicas disponíveis, na ordem de execução.
    pub order: Vec<Ordered>,
    /// Ligadas no preset mas que nenhum efeito carregado declara (aviso, não erro: como no ReShade).
    pub unknown: Vec<TechId>,
}

impl Resolved {
    /// As ligadas, na ordem em que rodam.
    pub fn enabled(&self) -> impl Iterator<Item = &TechId> {
        self.order.iter().filter(|o| o.enabled).map(|o| &o.id)
    }
}

/// Atalho de teclado como o ReShade grava: `código,ctrl,shift,alt` (código de tecla do Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCombo {
    pub code: u32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyCombo {
    pub fn parse(text: &str) -> Option<Self> {
        let mut it = text.split(',').map(|s| s.trim().parse::<u32>());
        let code = it.next()?.ok()?;
        let mut flag = || it.next().and_then(Result::ok).map(|v| v != 0);
        let (ctrl, shift, alt) = (flag().unwrap_or(false), flag().unwrap_or(false), flag().unwrap_or(false));
        // Tecla 0 = sem atalho.
        (code != 0).then_some(Self { code, ctrl, shift, alt })
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{},{}", self.code, self.ctrl as u8, self.shift as u8, self.alt as u8)
    }
}

/// Macros `(nome, valor)`. Valor vazio vale `1`. Dentro do mesmo nível, a última repetição vale.
pub type Macros = Vec<(String, String)>;

pub fn parse_macros(text: &str) -> Macros {
    let mut out: Macros = Vec::new();
    for item in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (name, value) = item.split_once('=').unwrap_or((item, ""));
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() {
            continue;
        }
        let value = if value.is_empty() { "1" } else { value };
        match out.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = value.to_string(),
            None => out.push((name.to_string(), value.to_string())),
        }
    }
    out
}

/// Junta níveis de macros do menos para o mais específico (globais, preset, efeito): o último vence.
pub fn merge_macros(levels: &[&[(String, String)]]) -> Macros {
    let mut out: Macros = Vec::new();
    for (name, value) in levels.iter().flat_map(|l| l.iter()) {
        match out.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = value.clone(),
            None => out.push((name.clone(), value.clone())),
        }
    }
    out
}

fn format_macros(macros: &[(String, String)]) -> String {
    macros.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(",")
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Preset {
    ini: Ini,
}

impl Preset {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse(text: &str) -> Self {
        Self { ini: Ini::parse(text) }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self { ini: Ini::load(path)? })
    }

    /// Grava de forma atômica (arquivo temporário e `rename`).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.ini.save(path)
    }

    // ---- técnicas ----

    fn ids(&self, key: &str) -> Vec<TechId> {
        self.ini.get_list("", key).iter().map(|s| TechId::parse(s)).collect()
    }

    /// As técnicas ligadas (`Techniques`), na ordem do arquivo.
    pub fn techniques(&self) -> Vec<TechId> {
        self.ids(TECHNIQUES)
    }

    /// A ordem de execução gravada (`TechniqueSorting`); sem ela, a de `Techniques`.
    pub fn sorting(&self) -> Vec<TechId> {
        if self.ini.get("", SORTING).is_some() { self.ids(SORTING) } else { self.techniques() }
    }

    /// Ordem completa: a lista de ordenação e, depois, as ligadas que ela não cita.
    fn full_order(&self) -> Vec<TechId> {
        let mut order = self.sorting();
        for id in self.techniques() {
            if !order.iter().any(|o| o.matches(&id)) {
                order.push(id);
            }
        }
        order
    }

    pub fn is_enabled(&self, id: &TechId) -> bool {
        self.techniques().iter().any(|t| t.matches(id))
    }

    /// Casa o preset com o que os efeitos carregados declaram. `available` vem na ordem de reserva: a de
    /// declaração dentro de cada arquivo e, entre arquivos, a alfabética pelo rótulo. As técnicas que o preset
    /// não cita (desligadas, ou ligadas mas sem lugar na lista) seguem essa ordem, depois das citadas.
    pub fn resolve(&self, available: &[TechId]) -> Resolved {
        let enabled = self.techniques();
        let mut taken = vec![false; available.len()];
        let mut out = Resolved::default();
        for id in self.sorting() {
            match available.iter().enumerate().find(|(i, a)| !taken[*i] && a.matches(&id)) {
                Some((i, a)) => {
                    taken[i] = true;
                    out.order.push(Ordered { id: a.clone(), enabled: enabled.iter().any(|e| e.matches(a)) });
                }
                None if enabled.iter().any(|e| e.matches(&id)) => out.unknown.push(id),
                None => {}
            }
        }
        for a in available.iter().zip(&taken).filter(|(_, t)| !**t).map(|(a, _)| a) {
            out.order.push(Ordered { id: a.clone(), enabled: enabled.iter().any(|e| e.matches(a)) });
        }
        for id in enabled {
            if !out.unknown.iter().any(|u| u.matches(&id)) && !available.iter().any(|a| a.matches(&id)) {
                out.unknown.push(id);
            }
        }
        out
    }

    /// Escreve `Techniques` (as ligadas, na ordem dada) e, quando precisa, `TechniqueSorting`: só se já existia
    /// ou se a ordem difere da de `Techniques` (como o ReShade).
    fn store(&mut self, order: &[TechId], enabled: &[TechId]) {
        let on: Vec<String> =
            order.iter().filter(|t| enabled.iter().any(|e| e.matches(t))).map(TechId::to_string).collect();
        let all: Vec<String> = order.iter().map(TechId::to_string).collect();
        let had_sorting = self.ini.get("", SORTING).is_some();
        self.ini.set_list("", TECHNIQUES, &on);
        if had_sorting || all != on {
            self.ini.set_list("", SORTING, &all);
        }
    }

    /// Liga ou desliga uma técnica. Uma técnica nova entra no fim da ordem.
    pub fn set_enabled(&mut self, id: &TechId, on: bool) {
        if self.is_enabled(id) == on {
            return;
        }
        let mut order = self.full_order();
        if !order.iter().any(|o| o.matches(id)) {
            order.push(id.clone());
        }
        let mut enabled = self.techniques();
        if on {
            enabled.push(id.clone());
        } else {
            enabled.retain(|e| !e.matches(id));
        }
        self.store(&order, &enabled);
    }

    /// Tira a técnica do preset (da lista de ligadas e da ordem). `false` se ele não a conhecia.
    pub fn remove_technique(&mut self, id: &TechId) -> bool {
        let mut order = self.full_order();
        let before = order.len();
        order.retain(|o| !o.matches(id));
        if order.len() == before {
            return false;
        }
        let mut enabled = self.techniques();
        enabled.retain(|e| !e.matches(id));
        self.store(&order, &enabled);
        true
    }

    /// Muda a posição de uma técnica na ordem de execução (`to` conta de 0 e é limitado ao fim da lista).
    /// `false` se o preset não a conhece.
    pub fn move_technique(&mut self, id: &TechId, to: usize) -> bool {
        let mut order = self.full_order();
        let Some(from) = order.iter().position(|o| o.matches(id)) else { return false };
        let item = order.remove(from);
        order.insert(to.min(order.len()), item);
        let enabled = self.techniques();
        self.store(&order, &enabled);
        true
    }

    // ---- atalhos ----

    fn key_name(id: &TechId) -> String {
        format!("Key{id}")
    }

    /// O atalho que liga/desliga a técnica. Ausente, ou com tecla 0, é "sem atalho".
    pub fn technique_key(&self, id: &TechId) -> Option<KeyCombo> {
        if let Some(v) = self.ini.get("", &Self::key_name(id)) {
            return KeyCombo::parse(v);
        }
        // O preset pode citar a técnica sem o arquivo (ou o contrário).
        self.ini.entries("").into_iter().find_map(|(k, v)| {
            let other = TechId::parse(k.strip_prefix("Key")?);
            other.matches(id).then(|| KeyCombo::parse(v)).flatten()
        })
    }

    pub fn set_technique_key(&mut self, id: &TechId, key: Option<KeyCombo>) {
        let name = Self::key_name(id);
        match key {
            Some(k) => self.ini.set("", &name, &k.to_string()),
            None => {
                self.ini.remove("", &name);
            }
        }
    }

    // ---- macros ----

    /// As macros do preset inteiro.
    pub fn macros(&self) -> Macros {
        parse_macros(self.ini.get("", DEFINES).unwrap_or(""))
    }

    /// As macros só de um efeito (`[Arquivo.fx]`).
    pub fn effect_macros(&self, file: &str) -> Macros {
        parse_macros(self.ini.get(file, DEFINES).unwrap_or(""))
    }

    /// Macros efetivas de um efeito: `global` (config.ini) < preset < efeito.
    pub fn effective_macros(&self, global: &[(String, String)], file: &str) -> Macros {
        merge_macros(&[global, &self.macros(), &self.effect_macros(file)])
    }

    /// Define as macros do preset. Lista vazia remove a chave (o ReShade a omite quando vazia).
    pub fn set_macros(&mut self, macros: &[(String, String)]) {
        self.set_defines("", macros);
    }

    pub fn set_effect_macros(&mut self, file: &str, macros: &[(String, String)]) {
        self.set_defines(file, macros);
    }

    fn set_defines(&mut self, section: &str, macros: &[(String, String)]) {
        if macros.is_empty() {
            self.ini.remove(section, DEFINES);
        } else {
            self.ini.set(section, DEFINES, &format_macros(macros));
        }
    }

    // ---- valores de uniform ----

    /// O valor gravado de um uniform (um número por componente, separados por vírgula).
    pub fn value(&self, file: &str, uniform: &str) -> Option<&str> {
        self.ini.get(file, uniform)
    }

    pub fn set_value(&mut self, file: &str, uniform: &str, value: &str) {
        self.ini.set(file, uniform, value);
    }

    pub fn remove_value(&mut self, file: &str, uniform: &str) -> bool {
        self.ini.remove(file, uniform)
    }

    /// Os uniforms gravados de um efeito, na ordem do arquivo; sem as macros do efeito nem os atalhos
    /// (`Key...`). Um uniform cujo nome começa por `Key` só é alcançado por [`Preset::value`].
    pub fn values(&self, file: &str) -> Vec<(&str, &str)> {
        let mut v = self.ini.entries(file);
        v.retain(|(k, _)| *k != DEFINES && !k.starts_with("Key"));
        v
    }

    /// Os efeitos (`[Arquivo.fx]`) que o preset tem seção.
    /// Apaga a seção de um efeito (`[Arquivo.fx]`): os valores e as macros dele. `true` se havia.
    pub fn remove_effect(&mut self, file: &str) -> bool {
        self.ini.remove_section(file)
    }

    pub fn effect_files(&self) -> Vec<&str> {
        self.ini.sections()
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.ini.fmt(f)
    }
}

#[cfg(test)]
mod tests;

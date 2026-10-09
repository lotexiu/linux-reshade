//! Leitura e escrita de `.ini` sem perder o que não se entende.
//!
//! O formato é o do ReShade: `chave=valor`, seções `[Nome]`, comentários em linhas que começam por `;` ou
//! `#`, listas separadas por vírgula. A parte antes da primeira seção é a seção "global" (nome vazio), onde o
//! preset do ReShade guarda `Techniques=`. Chaves e nomes de seção diferenciam maiúsculas de minúsculas.
//!
//! Ao regravar, as linhas que não mudaram saem **exatamente** como entraram (espaços, comentários, ordem,
//! terminador de linha); só o que foi alterado é reescrito. Assim um preset da comunidade, ou um arquivo
//! que o usuário comentou à mão, não se desfaz ao ser salvo.

use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
enum Line {
    /// Linha em branco, comentário, ou lixo que não é nem seção nem `chave=valor`: guardada como está.
    Raw(String),
    Section { name: String, raw: String },
    Pair { key: String, value: String, raw: String },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ini {
    lines: Vec<Line>,
    /// O arquivo usava `\r\n` (o ReShade no Windows grava assim): as linhas novas seguem o mesmo estilo.
    crlf: bool,
}

/// Onde uma seção começa e termina em `lines` (o fim é exclusivo).
struct Span {
    /// Índice do cabeçalho; `None` para a seção global.
    header: Option<usize>,
    end: usize,
}

impl Ini {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse(text: &str) -> Self {
        let crlf = text.contains("\r\n");
        let mut lines = Vec::new();
        for raw in text.split('\n') {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            lines.push(classify(raw));
        }
        // `split` deixa um item vazio depois do último `\n`: não é uma linha do arquivo.
        if text.is_empty() || text.ends_with('\n') {
            lines.pop();
        }
        Self { lines, crlf }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self::parse(&String::from_utf8_lossy(&std::fs::read(path)?)))
    }

    /// Grava de forma atômica: arquivo temporário ao lado e `rename`. Cria as pastas que faltam.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&tmp, self.to_string())?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    fn span(&self, section: &str) -> Option<Span> {
        let headers = self.lines.iter().enumerate().filter_map(|(i, l)| match l {
            Line::Section { name, .. } => Some((i, name.as_str())),
            _ => None,
        });
        if section.is_empty() {
            let end = headers.map(|(i, _)| i).next().unwrap_or(self.lines.len());
            return Some(Span { header: None, end });
        }
        let headers: Vec<_> = headers.collect();
        // Se a seção aparece mais de uma vez, vale a primeira (e `set` escreve nela).
        let pos = headers.iter().position(|(_, n)| *n == section)?;
        let end = headers.get(pos + 1).map_or(self.lines.len(), |(i, _)| *i);
        Some(Span { header: Some(headers[pos].0), end })
    }

    fn range(&self, span: &Span) -> std::ops::Range<usize> {
        span.header.map_or(0, |h| h + 1)..span.end
    }

    pub fn has_section(&self, section: &str) -> bool {
        self.span(section).is_some_and(|s| s.header.is_some()) || (section.is_empty() && !self.lines.is_empty())
    }

    /// O valor de uma chave. Se a chave se repete, vale a última (como ao carregar o arquivo no ReShade).
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        let span = self.span(section)?;
        self.lines[self.range(&span)].iter().rev().find_map(|l| match l {
            Line::Pair { key: k, value, .. } if k == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// Uma lista separada por vírgulas, sem itens vazios. Ausente = lista vazia.
    pub fn get_list(&self, section: &str, key: &str) -> Vec<String> {
        self.get(section, key).map(split_list).unwrap_or_default()
    }

    /// Define uma chave. Se já existe com o mesmo valor, nada muda (a linha original fica intacta).
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let new = || Line::Pair { key: key.to_string(), value: value.to_string(), raw: format!("{key}={value}") };
        match self.span(section) {
            Some(span) => {
                let range = self.range(&span);
                let found = self.lines[range.clone()].iter().rposition(|l| matches!(l, Line::Pair { key: k, .. } if k == key));
                match found {
                    Some(i) => {
                        let idx = range.start + i;
                        if matches!(&self.lines[idx], Line::Pair { value: v, .. } if v == value) {
                            return;
                        }
                        self.lines[idx] = new();
                    }
                    None => {
                        // Entra logo depois da última chave da seção; sem nenhuma, logo depois do cabeçalho.
                        let at = self.lines[range.clone()].iter().rposition(|l| matches!(l, Line::Pair { .. })).map_or(range.start, |i| range.start + i + 1);
                        let line = new();
                        self.lines.insert(at, line);
                    }
                }
            }
            None => {
                if !self.lines.is_empty() && !matches!(self.lines.last(), Some(Line::Raw(r)) if r.trim().is_empty()) {
                    self.lines.push(Line::Raw(String::new()));
                }
                self.lines.push(Line::Section { name: section.to_string(), raw: format!("[{section}]") });
                let line = new();
                self.lines.push(line);
            }
        }
    }

    pub fn set_list(&mut self, section: &str, key: &str, items: &[String]) {
        self.set(section, key, &items.join(","));
    }

    /// Remove todas as ocorrências de uma chave. `true` se havia alguma.
    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        let Some(span) = self.span(section) else { return false };
        let range = self.range(&span);
        let before = self.lines.len();
        let mut i = range.start;
        let mut end = range.end;
        while i < end {
            if matches!(&self.lines[i], Line::Pair { key: k, .. } if k == key) {
                self.lines.remove(i);
                end -= 1;
            } else {
                i += 1;
            }
        }
        self.lines.len() != before
    }

    /// Remove uma seção inteira, com o que estiver nela. A seção global não pode ser removida (só esvaziada
    /// chave a chave).
    pub fn remove_section(&mut self, section: &str) -> bool {
        match self.span(section) {
            Some(Span { header: Some(h), end }) => {
                self.lines.drain(h..end);
                true
            }
            _ => false,
        }
    }

    /// Os nomes das seções nomeadas, na ordem do arquivo e sem repetição.
    pub fn sections(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for l in &self.lines {
            if let Line::Section { name, .. } = l
                && !names.contains(&name.as_str())
            {
                names.push(name);
            }
        }
        names
    }

    /// As chaves de uma seção com seus valores, na ordem do arquivo (a última repetição de cada chave).
    pub fn entries(&self, section: &str) -> Vec<(&str, &str)> {
        let Some(span) = self.span(section) else { return Vec::new() };
        let mut out: Vec<(&str, &str)> = Vec::new();
        for l in &self.lines[self.range(&span)] {
            if let Line::Pair { key, value, .. } = l {
                match out.iter_mut().find(|(k, _)| k == key) {
                    Some(e) => e.1 = value,
                    None => out.push((key, value)),
                }
            }
        }
        out
    }
}

impl std::fmt::Display for Ini {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let eol = if self.crlf { "\r\n" } else { "\n" };
        for l in &self.lines {
            let raw = match l {
                Line::Raw(r) | Line::Section { raw: r, .. } | Line::Pair { raw: r, .. } => r,
            };
            write!(f, "{raw}{eol}")?;
        }
        Ok(())
    }
}

fn classify(raw: &str) -> Line {
    let t = raw.trim();
    if t.is_empty() || t.starts_with(';') || t.starts_with('#') {
        return Line::Raw(raw.to_string());
    }
    if let Some(inner) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return Line::Section { name: inner.to_string(), raw: raw.to_string() };
    }
    match t.split_once('=') {
        Some((key, value)) if !key.trim().is_empty() => {
            Line::Pair { key: key.trim().to_string(), value: value.trim().to_string(), raw: raw.to_string() }
        }
        _ => Line::Raw(raw.to_string()),
    }
}

/// Divide uma lista separada por vírgulas, aparando espaços e descartando itens vazios.
pub fn split_list(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

#[cfg(test)]
mod tests;

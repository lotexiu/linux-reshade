//! Teclas escritas como a pessoa as escreve (`Ctrl+Shift+F10`, `'`, `OEM_7`, `aspas`) para o formato XDG que o portal
//! entende (`CTRL+SHIFT+F10`, `apostrophe`): modificadores em maiúsculas e a tecla pelo nome de keysym do xkb.
//!
//! As teclas OEM do Windows (as que o ReShade mostra, como "OEM 7") valem pela posição no teclado americano; no xkb isso
//! vira o keysym que a tecla produz sem modificador.

/// Modificadores aceitos, na ordem em que saem no texto final.
const MODIFIERS: [(&str, &[&str]); 4] = [("CTRL", &["ctrl", "control", "ctl"]), ("ALT", &["alt"]), ("SHIFT", &["shift"]), ("LOGO", &["super", "logo", "meta", "win", "windows"])];

/// Nomes de tecla que a pessoa costuma escrever, e o keysym do xkb de cada uma.
const KEYS: &[(&[&str], &str)] = &[
    (&["'", "aspas", "apostrophe", "oem_7", "oem7", "oem 7", "quote", "quotedbl", "\""], "apostrophe"),
    (&[";", "semicolon", "oem_1", "oem1", "oem 1", "pontoevirgula"], "semicolon"),
    (&["/", "slash", "oem_2", "oem2", "oem 2"], "slash"),
    (&["`", "grave", "oem_3", "oem3", "oem 3", "crase"], "grave"),
    (&["[", "bracketleft", "oem_4", "oem4", "oem 4"], "bracketleft"),
    (&["\\", "backslash", "oem_5", "oem5", "oem 5"], "backslash"),
    (&["]", "bracketright", "oem_6", "oem6", "oem 6"], "bracketright"),
    (&["=", "equal", "oem_plus", "oemplus", "oem plus", "mais"], "equal"),
    (&["-", "minus", "oem_minus", "oemminus", "oem minus", "menos"], "minus"),
    (&[",", "comma", "oem_comma", "oemcomma", "oem comma", "virgula", "vírgula"], "comma"),
    (&[".", "period", "oem_period", "oemperiod", "oem period", "ponto"], "period"),
    (&["space", "espaco", "espaço"], "space"),
    (&["return", "enter"], "Return"),
    (&["tab"], "Tab"),
    (&["escape", "esc"], "Escape"),
    (&["backspace"], "BackSpace"),
    (&["delete", "del"], "Delete"),
    (&["insert", "ins"], "Insert"),
    (&["home"], "Home"),
    (&["end"], "End"),
    (&["pageup", "page_up", "prior"], "Prior"),
    (&["pagedown", "page_down", "next"], "Next"),
    (&["up", "seta_cima"], "Up"),
    (&["down", "seta_baixo"], "Down"),
    (&["left", "seta_esquerda"], "Left"),
    (&["right", "seta_direita"], "Right"),
    (&["pause"], "Pause"),
    (&["scrolllock", "scroll_lock"], "Scroll_Lock"),
    (&["print", "printscreen", "prtsc"], "Print"),
];

/// Uma tecla (sem modificadores) escrita pela pessoa, no nome de keysym do xkb. `None` se não se reconhece.
fn key_name(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    if let Some((_, name)) = KEYS.iter().find(|(names, _)| names.contains(&lower.as_str())) {
        return Some((*name).to_string());
    }
    // F1..F24
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some(format!("F{n}"));
    }
    // Letras e números: uma letra vira minúscula (o xkb pede o keysym sem shift), um dígito fica como está.
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_string()),
        _ => None,
    }
}

/// O mesmo atalho no formato XDG. Aceita `+` entre as partes (`Ctrl + Shift + F10`); a última parte é a tecla. A tecla
/// `+` sozinha escreve-se `plus` (ou `mais`) e a `-`, `minus`.
pub fn normalize(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("atalho vazio".into());
    }
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    // Um "+" literal no fim ("Ctrl++") deixa uma parte vazia: vira a tecla `plus`.
    let mut parts: Vec<String> = parts.iter().map(|p| (*p).to_string()).collect();
    if parts.len() >= 2 && parts.last().is_some_and(String::is_empty) {
        parts.pop();
        if let Some(last) = parts.last_mut() {
            if last.is_empty() {
                *last = "plus".into();
            }
        }
        parts.retain(|p| !p.is_empty());
        parts.push("plus".into());
        parts.dedup();
    }
    let (key, mods) = parts.split_last().ok_or("atalho vazio")?;
    let mut found: Vec<&str> = Vec::new();
    for m in mods {
        let lower = m.to_lowercase();
        let Some((name, _)) = MODIFIERS.iter().find(|(_, aliases)| aliases.contains(&lower.as_str())) else {
            return Err(format!("modificador desconhecido: {m:?} (use Ctrl, Alt, Shift ou Super)"));
        };
        if !found.contains(name) {
            found.push(name);
        }
    }
    found.sort_by_key(|m| MODIFIERS.iter().position(|(n, _)| n == m));
    let key = if key.eq_ignore_ascii_case("plus") { "plus".to_string() } else { key_name(key).ok_or_else(|| format!("tecla desconhecida: {key:?}"))? };
    let mut out: Vec<String> = found.into_iter().map(String::from).collect();
    out.push(key);
    Ok(out.join("+"))
}

/// Uma tecla sem modificador nenhum vale em TODOS os programas (a pessoa deixa de poder digitá-la em qualquer lugar).
pub fn is_bare_key(normalized: &str) -> bool {
    !normalized.contains('+') || normalized == "plus"
}

/// O texto para mostrar: `CTRL+apostrophe` -> `Ctrl+'`.
pub fn display(normalized: &str) -> String {
    normalized
        .split('+')
        .map(|part| match part {
            "CTRL" => "Ctrl".to_string(),
            "ALT" => "Alt".to_string(),
            "SHIFT" => "Shift".to_string(),
            "LOGO" => "Super".to_string(),
            "apostrophe" => "' (aspas)".to_string(),
            "semicolon" => ";".to_string(),
            "slash" => "/".to_string(),
            "grave" => "`".to_string(),
            "bracketleft" => "[".to_string(),
            "backslash" => "\\".to_string(),
            "bracketright" => "]".to_string(),
            "equal" => "=".to_string(),
            "minus" => "-".to_string(),
            "comma" => ",".to_string(),
            "period" => ".".to_string(),
            "plus" => "+".to_string(),
            other if other.len() == 1 => other.to_uppercase(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_write_keys_their_own_way_and_get_the_xdg_form() {
        assert_eq!(normalize("Ctrl+Shift+F10").unwrap(), "CTRL+SHIFT+F10");
        assert_eq!(normalize("shift + ctrl + f10").unwrap(), "CTRL+SHIFT+F10", "ordem e caixa não importam");
        assert_eq!(normalize("'").unwrap(), "apostrophe");
        assert_eq!(normalize("\"").unwrap(), "apostrophe", "a tecla do ' e do \" é a mesma");
        assert_eq!(normalize("OEM_7").unwrap(), "apostrophe", "a tecla OEM do ReShade");
        assert_eq!(normalize("Alt+OEM 3").unwrap(), "ALT+grave");
        assert_eq!(normalize("Super+Z").unwrap(), "LOGO+z");
        assert_eq!(normalize("Ctrl+Page_Down").unwrap(), "CTRL+Next");
        assert_eq!(normalize("Ctrl++").unwrap(), "CTRL+plus");
        assert_eq!(normalize("Ctrl+7").unwrap(), "CTRL+7");
    }

    #[test]
    fn nonsense_is_refused_with_a_reason() {
        assert!(normalize("").is_err());
        assert!(normalize("Ctrl+").is_err() || normalize("Ctrl+").is_ok_and(|t| t == "CTRL+plus"));
        assert!(normalize("Hyper+F1").unwrap_err().contains("modificador"));
        assert!(normalize("Ctrl+Banana").unwrap_err().contains("tecla"));
    }

    #[test]
    fn a_bare_key_is_flagged_and_keys_display_readably() {
        assert!(is_bare_key("apostrophe") && is_bare_key("F9") && !is_bare_key("CTRL+F9"));
        assert_eq!(display("CTRL+SHIFT+F10"), "Ctrl+Shift+F10");
        assert_eq!(display("apostrophe"), "' (aspas)");
        assert_eq!(display("LOGO+z"), "Super+Z");
    }
}

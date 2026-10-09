use super::*;

const PRESET: &str = "PreprocessorDefinitions=RESHADE_DEPTH_INPUT_IS_REVERSED=1\n\
Techniques=Sepia@Sepia.fx,Bloom@Bloom.fx\n\
TechniqueSorting=Sepia@Sepia.fx,Bloom@Bloom.fx\n\
\n\
; ajustei à mão\n\
[Sepia.fx]\n\
Intensity = 0.8\n\
Cor=1.0,0.5,0.2\n\
\n\
[Bloom.fx]\n\
Force=0.3\n";

#[test]
fn reads_global_and_named_sections() {
    let ini = Ini::parse(PRESET);
    assert_eq!(ini.get("", "Techniques"), Some("Sepia@Sepia.fx,Bloom@Bloom.fx"));
    assert_eq!(ini.get("Sepia.fx", "Intensity"), Some("0.8"));
    assert_eq!(ini.get("Bloom.fx", "Force"), Some("0.3"));
    assert_eq!(ini.get("Bloom.fx", "Intensity"), None);
    assert_eq!(ini.get("Nada.fx", "Force"), None);
    assert_eq!(ini.sections(), ["Sepia.fx", "Bloom.fx"]);
}

#[test]
fn global_section_stops_at_the_first_header() {
    let ini = Ini::parse(PRESET);
    // `Intensity` é da seção Sepia.fx; não vaza para a global.
    assert_eq!(ini.get("", "Intensity"), None);
    assert_eq!(ini.entries("").len(), 3);
}

#[test]
fn unchanged_file_round_trips_byte_for_byte() {
    assert_eq!(Ini::parse(PRESET).to_string(), PRESET);
    let odd = "  chave  =  valor  \n;c\n#d\nlixo sem igual\n[ S ]\nx=1";
    // Sem `\n` final no original, a saída ganha um (a única normalização).
    assert_eq!(Ini::parse(odd).to_string(), format!("{odd}\n"));
}

#[test]
fn crlf_is_kept() {
    let text = "A=1\r\n[S]\r\nB=2\r\n";
    let mut ini = Ini::parse(text);
    assert_eq!(ini.to_string(), text);
    ini.set("S", "C", "3");
    assert_eq!(ini.to_string(), "A=1\r\n[S]\r\nB=2\r\nC=3\r\n");
}

#[test]
fn set_changes_only_the_touched_line() {
    let mut ini = Ini::parse(PRESET);
    ini.set("Sepia.fx", "Intensity", "0.5");
    let expected = PRESET.replace("Intensity = 0.8", "Intensity=0.5");
    assert_eq!(ini.to_string(), expected);
    // Mesmo valor: a linha original (com seus espaços) é mantida.
    let mut same = Ini::parse(PRESET);
    same.set("Sepia.fx", "Intensity", "0.8");
    assert_eq!(same.to_string(), PRESET);
}

#[test]
fn set_adds_keys_inside_their_section() {
    let mut ini = Ini::parse(PRESET);
    ini.set("Sepia.fx", "Novo", "1");
    ini.set("", "Key", "x");
    let text = ini.to_string();
    // A chave nova da seção fica antes da linha em branco que separa a próxima seção.
    assert!(text.contains("Cor=1.0,0.5,0.2\nNovo=1\n\n[Bloom.fx]"), "{text}");
    // A global entra no fim da parte global, antes de qualquer cabeçalho.
    assert!(text.contains("TechniqueSorting=Sepia@Sepia.fx,Bloom@Bloom.fx\nKey=x\n"), "{text}");
}

#[test]
fn set_creates_missing_sections() {
    let mut ini = Ini::parse(PRESET);
    ini.set("Novo.fx", "A", "1");
    assert!(ini.to_string().ends_with("Force=0.3\n\n[Novo.fx]\nA=1\n"));
    let mut empty = Ini::new();
    empty.set("S", "A", "1");
    assert_eq!(empty.to_string(), "[S]\nA=1\n");
    let mut global = Ini::new();
    global.set("", "A", "1");
    assert_eq!(global.to_string(), "A=1\n");
}

#[test]
fn remove_key_and_section() {
    let mut ini = Ini::parse(PRESET);
    assert!(ini.remove("Sepia.fx", "Cor"));
    assert!(!ini.remove("Sepia.fx", "Cor"));
    assert_eq!(ini.get("Sepia.fx", "Cor"), None);
    assert!(ini.remove_section("Bloom.fx"));
    assert!(!ini.sections().contains(&"Bloom.fx"));
    assert!(ini.to_string().contains("; ajustei à mão"), "comentários ficam");
    assert!(!ini.remove_section(""), "a seção global não se remove");
}

#[test]
fn last_duplicate_wins() {
    let ini = Ini::parse("A=1\nA=2\n[S]\nB=1\n[T]\nB=9\n[S]\nB=5\n");
    assert_eq!(ini.get("", "A"), Some("2"));
    // Seção repetida: vale a primeira ocorrência.
    assert_eq!(ini.get("S", "B"), Some("1"));
    assert_eq!(ini.get("T", "B"), Some("9"));
}

#[test]
fn lists_are_comma_separated() {
    let ini = Ini::parse("L= a , b,, c \nE=\n");
    assert_eq!(ini.get_list("", "L"), ["a", "b", "c"]);
    assert!(ini.get_list("", "E").is_empty());
    assert!(ini.get_list("", "Falta").is_empty());
    let mut ini = ini;
    ini.set_list("", "N", &["x".into(), "y".into()]);
    assert_eq!(ini.get("", "N"), Some("x,y"));
}

#[test]
fn value_may_contain_equals() {
    let ini = Ini::parse("PreprocessorDefinitions=A=1,B=2\n");
    assert_eq!(ini.get("", "PreprocessorDefinitions"), Some("A=1,B=2"));
}

#[test]
fn empty_input() {
    let ini = Ini::parse("");
    assert_eq!(ini.to_string(), "");
    assert_eq!(ini.get("", "A"), None);
    assert!(ini.sections().is_empty());
}

#[test]
fn save_is_atomic_and_creates_directories() {
    let dir = std::env::temp_dir().join(format!("ini-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("a").join("b.ini");
    let mut ini = Ini::new();
    ini.set("S", "A", "1");
    ini.save(&path).unwrap();
    assert_eq!(Ini::load(&path).unwrap().get("S", "A"), Some("1"));
    // Nenhum temporário sobra.
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

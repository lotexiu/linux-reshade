//! `cargo run -p i18n --example dyn -- "<mensagem>"`: mostra como uma mensagem em inglês (por exemplo vinda da layer) é traduzida no idioma de `LR_LANG`.
fn main() {
    for text in std::env::args().skip(1) {
        println!("{}", i18n::tr_dyn(&text));
    }
}

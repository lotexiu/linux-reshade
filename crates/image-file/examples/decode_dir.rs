//! Decodifica todos os arquivos de uma pasta e conta os resultados: `cargo run -p image-file --example decode_dir <pasta>`.
fn main() {
    let dir = std::env::args().nth(1).expect("uso: decode_dir <pasta>");
    let (mut ok, mut bad) = (0, 0);
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().is_file()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        match image_file::decode(&name, &std::fs::read(e.path()).unwrap()) {
            Ok(img) => {
                ok += 1;
                println!("ok      {name} {}x{}", img.width, img.height);
            }
            Err(err) => {
                bad += 1;
                println!("recusou {name}: {err}");
            }
        }
    }
    println!("{ok} ok, {bad} recusadas");
}

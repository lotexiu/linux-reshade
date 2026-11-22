#[test]
fn depth_policy_is_merged_per_key_and_removed_when_empty() {
    use super::*;
    let dir = std::env::temp_dir().join(format!("lr-policy-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(depth_policy_in(&dir, "layer-1-2").is_empty());
    set_depth_policy_in(&dir, "layer-1-2", "split", Some("off")).unwrap();
    set_depth_policy_in(&dir, "layer-1-2", "copy", Some("on")).unwrap();
    assert_eq!(depth_policy_in(&dir, "layer-1-2"), [("split".to_string(), "off".to_string()), ("copy".to_string(), "on".to_string())]);
    set_depth_policy_in(&dir, "layer-1-2", "split", None).unwrap();
    assert_eq!(depth_policy_in(&dir, "layer-1-2"), [("copy".to_string(), "on".to_string())]);
    assert!(set_depth_policy_in(&dir, "layer-1-2", "bogus", Some("on")).is_err());
    set_depth_policy_in(&dir, "layer-1-2", "copy", None).unwrap();
    assert!(depth_policy_in(&dir, "layer-1-2").is_empty() && !dir.join("depth-policy-layer-1-2").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_depth_pick_text_round_trips_and_rejects_garbage() {
    use super::*;
    for pick in [DepthPick::Candidate(3), DepthPick::Pass(2), DepthPick::Clear(2)] {
        assert_eq!(DepthPick::parse(&pick.text().unwrap()), Some(pick));
    }
    assert_eq!(DepthPick::Auto.text(), None);
    for bad in ["", "pass", "pass:0", "pass:x", "outra:1", "candidate:-1"] {
        assert_eq!(DepthPick::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn the_depth_pick_round_trips_through_its_file() {
    use super::*;
    let dir = std::env::temp_dir().join(format!("lr-pick-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(depth_pick_in(&dir, "layer-1"), DepthPick::Auto);
    set_depth_pick_in(&dir, "layer-1", DepthPick::Candidate(2)).unwrap();
    assert_eq!(depth_pick_in(&dir, "layer-1"), DepthPick::Candidate(2));
    set_depth_pick_in(&dir, "layer-1", DepthPick::Pass(5)).unwrap();
    assert_eq!(depth_pick_in(&dir, "layer-1"), DepthPick::Pass(5));
    set_depth_pick_in(&dir, "layer-1", DepthPick::Auto).unwrap();
    assert_eq!(depth_pick_in(&dir, "layer-1"), DepthPick::Auto);
    assert_eq!(depth_pick_in(&dir, "../x"), DepthPick::Auto);
    let _ = fs::remove_dir_all(&dir);
}

use super::*;

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("linux-reshade-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir.join("state")
}

#[test]
fn reader_sees_nothing_before_writer_exists() {
    let path = temp_path("absent");
    assert!(Reader::open_at(&path).unwrap().is_none());
}

#[test]
fn reader_follows_writer() {
    let path = temp_path("follow");
    let writer = Writer::create_at(&path).unwrap();
    let reader = Reader::open_at(&path).unwrap().expect("arquivo existe");
    assert!(!reader.effects_enabled());
    writer.set_effects_enabled(true);
    assert!(reader.effects_enabled());
    assert!(!writer.toggle_effects());
    assert!(!reader.effects_enabled());
    assert!(writer.toggle_effects());
    assert!(reader.effects_enabled());
}

#[test]
fn restarting_writer_keeps_state_and_existing_reader() {
    let path = temp_path("restart");
    let first = Writer::create_at(&path).unwrap();
    first.set_effects_enabled(true);
    let reader = Reader::open_at(&path).unwrap().unwrap();
    drop(first);
    let second = Writer::create_at(&path).unwrap();
    assert!(second.effects_enabled(), "estado preservado");
    second.set_effects_enabled(false);
    assert!(!reader.effects_enabled(), "o leitor antigo continua válido");
}

#[test]
fn params_travel_from_writer_to_reader_through_the_file() {
    let path = temp_path("params");
    let writer = Writer::create_at(&path).unwrap();
    let reader = Reader::open_at(&path).unwrap().unwrap();
    let mut out = [0u8; 8];
    assert_eq!(reader.read_params(9, &mut out), None, "nada publicado ainda");

    writer.publish_params(9, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    assert_eq!(reader.read_params(9, &mut out), Some(8));
    assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(reader.read_params(10, &mut out), None, "outro efeito");
    assert_eq!(writer.published_bundle(), 9);

    // O escritor também lê o que publicou (é como o `lrctl set` altera só um valor).
    let mut again = [0u8; 8];
    assert_eq!(writer.read_params(9, &mut again), Some(8));
    assert_eq!(again, out);
}

#[test]
fn params_survive_a_writer_restart() {
    let path = temp_path("params-restart");
    let first = Writer::create_at(&path).unwrap();
    first.publish_params(3, &[7; 4]).unwrap();
    drop(first);
    let second = Writer::create_at(&path).unwrap();
    let mut out = [0u8; 4];
    assert_eq!(second.read_params(3, &mut out), Some(4));
    assert_eq!(out, [7; 4]);
}

#[test]
fn an_old_smaller_state_file_is_upgraded() {
    // Um arquivo de uma versão anterior (menor, só com o estado) deixado em /dev/shm.
    let path = temp_path("old");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut old = Vec::new();
    for word in [MAGIC, 1u32, FLAG_EFFECTS_ENABLED] {
        old.extend_from_slice(&word.to_ne_bytes());
    }
    fs::write(&path, &old).unwrap();
    assert!(Reader::open_at(&path).unwrap().is_none(), "ainda pequeno demais para o layout novo");

    let writer = Writer::create_at(&path).unwrap();
    writer.publish_params(1, &[5; 4]).unwrap();
    let reader = Reader::open_at(&path).unwrap().expect("agora é do tamanho certo");
    assert!(!reader.effects_enabled(), "o arquivo antigo é reinicializado");
    assert_eq!(reader.read_params(1, &mut [0u8; 4]), Some(4));
}

#[test]
fn the_selected_effect_travels_to_the_reader_and_can_be_cleared() {
    let path = temp_path("selection");
    let writer = Writer::create_at(&path).unwrap();
    let reader = Reader::open_at(&path).unwrap().unwrap();
    assert_eq!(reader.read_selection(), SelectionRead::Cleared, "nada escolhido ainda");

    let g0 = reader.selection_generation();
    writer.publish_selection("/jogos/efeitos/Sepia-1920x1080").unwrap();
    assert_eq!(reader.read_selection(), SelectionRead::Path("/jogos/efeitos/Sepia-1920x1080".into()));
    assert_eq!(writer.selection(), reader.read_selection());
    let g1 = reader.selection_generation();
    assert_ne!(g0, g1, "a geração muda ao publicar");
    assert_eq!(reader.selection_generation(), g1, "e não muda sozinha");

    writer.clear_selection();
    assert_eq!(reader.read_selection(), SelectionRead::Cleared);
    assert_ne!(reader.selection_generation(), g1, "limpar também é uma mudança");
}

#[test]
fn a_path_with_non_ascii_characters_and_the_size_limit() {
    let path = temp_path("selection-utf8");
    let writer = Writer::create_at(&path).unwrap();
    let reader = Reader::open_at(&path).unwrap().unwrap();
    writer.publish_selection("/home/são josé/efeitos/Sépia-1920x1080").unwrap();
    assert_eq!(reader.read_selection(), SelectionRead::Path("/home/são josé/efeitos/Sépia-1920x1080".into()));
    let huge = "a".repeat(MAX_SELECTION_BYTES + 1);
    assert!(writer.publish_selection(&huge).is_err());
    assert_eq!(
        reader.read_selection(),
        SelectionRead::Path("/home/são josé/efeitos/Sépia-1920x1080".into()),
        "a recusa não estraga a seleção anterior"
    );
}

#[test]
fn selection_and_params_are_independent() {
    let path = temp_path("independent");
    let writer = Writer::create_at(&path).unwrap();
    let reader = Reader::open_at(&path).unwrap().unwrap();
    writer.publish_params(7, &[1, 2, 3, 4]).unwrap();
    writer.publish_selection("/a").unwrap();
    writer.publish_params(7, &[5, 6, 7, 8]).unwrap();
    assert_eq!(reader.read_selection(), SelectionRead::Path("/a".into()));
    let mut out = [0u8; 4];
    assert_eq!(reader.read_params(7, &mut out), Some(4));
    assert_eq!(out, [5, 6, 7, 8]);
}

#[test]
fn rejects_uninitialized_file() {
    let path = temp_path("uninit");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, vec![0u8; SIZE]).unwrap(); // tamanho certo, magic zerado
    assert!(Reader::open_at(&path).unwrap().is_none());
}

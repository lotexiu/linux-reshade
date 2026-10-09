//! Executa na GPU de software cada bundle que `scripts/dev.sh fx-test` gerou em `build/bundles`
//! (efeitos próprios e da comunidade). Não confere pixels: confere que o runtime cria, grava,
//! submete e destrói o efeito sem falhar. Um `Unsupported` é aceitável e é listado; qualquer outro
//! erro, ou uma falha do Vulkan, reprova o teste. Sem bundles gerados, o teste só avisa.

mod common;

use ash::vk;
use common::*;
use fx_bundle::Bundle;
use postfx::effect::Target;
use postfx::{Effect, Error};
use std::fs;
use std::path::Path;

#[test]
fn every_compiled_bundle_either_runs_or_is_refused_explicitly() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/bundles");
    let Ok(entries) = fs::read_dir(&dir) else {
        eprintln!("sem {}; rode scripts/dev.sh fx-test para exercitar este teste", dir.display());
        return;
    };
    let Some(gpu) = Gpu::software() else {
        eprintln!("sem dispositivo Vulkan de software; teste ignorado");
        return;
    };

    let mut names: Vec<_> = entries.flatten().filter(|e| e.path().join("manifest.json").is_file()).collect();
    names.sort_by_key(|e| e.file_name());

    let (mut ran, mut refused) = (Vec::new(), Vec::new());
    for entry in names {
        let name = entry.file_name().to_string_lossy().into_owned();
        let bundle = match Bundle::load(&entry.path()) {
            Ok(b) => b,
            Err(fx_bundle::Error::Unsupported(why)) => {
                refused.push(format!("{name}: {why}"));
                continue;
            }
            Err(e) => panic!("{name}: o leitor recusou um bundle do próprio compilador: {e}"),
        };
        unsafe {
            let targets = gpu.targets(2, input);
            let target = Target {
                images: &targets.images,
                format: FORMAT,
                extent: vk::Extent2D { width: W, height: H },
                rest_layout: REST,
                queue_family: gpu.family,
                init_command_buffer: None,
            };
            let created = Effect::new(&gpu.device, &gpu.memory, &bundle, 0, &target, gpu.queue);
            match created {
                Ok(mut effect) => {
                    for i in 0..targets.images.len() {
                        gpu.submit(effect.command_buffer(i));
                    }
                    effect.destroy(&gpu.device);
                    ran.push(name);
                }
                Err(Error::Unsupported(why)) => refused.push(format!("{name}: {why}")),
                Err(e) => panic!("{name}: falha inesperada ao criar o efeito: {e}"),
            }
            gpu.destroy_targets(targets);
        }
    }
    eprintln!("\n{} efeitos executados: {}", ran.len(), ran.join(", "));
    eprintln!("{} recusados:", refused.len());
    for r in &refused {
        eprintln!("  {r}");
    }
}

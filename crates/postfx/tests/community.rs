//! Executa na GPU de software cada bundle que `scripts/dev.sh fx-test` gerou em `build/bundles`
//! (efeitos próprios e da comunidade). Não confere pixels: confere que o runtime cria, grava,
//! submete e destrói o efeito sem falhar. Um `Unsupported` é aceitável e é listado; qualquer outro
//! erro, ou uma falha do Vulkan, reprova o teste. Sem bundles gerados, o teste só avisa.

mod common;

use ash::vk;
use common::*;
use fx_bundle::Bundle;
use postfx::effect::{DepthInput, FrameInfo, Target};
use postfx::{Effect, Error};
use std::fs;
use std::path::Path;

#[test]
fn every_compiled_bundle_either_runs_or_is_refused_explicitly() {
    // LINUX_RESHADE_SURVEY_BUNDLES aponta para outra pasta de bundles (o levantamento de `scripts/dev.sh fx-survey`);
    // nesse modo uma falha inesperada é listada em vez de reprovar o teste.
    let survey = std::env::var_os("LINUX_RESHADE_SURVEY_BUNDLES");
    let dir = survey.as_ref().map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/bundles"), std::path::PathBuf::from);
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

    let (mut ran, mut refused, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for entry in names {
        let name = entry.file_name().to_string_lossy().into_owned();
        let bundle = match Bundle::load(&entry.path()) {
            Ok(b) => b,
            Err(fx_bundle::Error::Unsupported(why)) => {
                refused.push(format!("{name}: {why}"));
                continue;
            }
            Err(e) if survey.is_some() => {
                failed.push(format!("{name}: bundle: {e}"));
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
                depth: None,
            };
            let mut created = Effect::new(&gpu.device, &gpu.memory, &bundle, 0, &target, gpu.queue);
            // Pede profundidade: dá uma imagem de profundidade como a de um jogo (cheia de 0,5).
            let depth_image = if matches!(&created, Err(Error::Unsupported(why)) if why.contains("profundidade")) {
                let depth = depth_image(&gpu, W, H, 0.5);
                let input = DepthInput { image: depth.image, format: DEPTH_FORMAT, layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL, extent: vk::Extent2D { width: W, height: H }, linear_filter: true };
                created = Effect::new(&gpu.device, &gpu.memory, &bundle, 0, &Target { depth: Some(input), ..target }, gpu.queue);
                Some(depth)
            } else {
                None
            };
            match created {
                Ok(mut effect) => {
                    // Frame típico de um jogo rodando há 10 s: os uniforms com `source` precisam de valores.
                    let frame = FrameInfo { time_ms: 10_000.0, frame_time_ms: 16.6, frame_count: 600, date: [2026.0, 10.0, 9.0, 43_200.0] };
                    for i in 0..targets.images.len() {
                        effect.update_sources(i, &frame);
                        gpu.submit(effect.command_buffer(i));
                    }
                    // Regressão: o NightVision multiplica a imagem por smoothstep(.., timer). Sem o `timer`
                    // alimentado ele saía todo preto no jogo; com o tempo andando tem de aparecer imagem.
                    if name == "NightVision" {
                        let lit = gpu.read(targets.images[0]).iter().filter(|p| p[..3].iter().any(|&c| c > 8)).count();
                        assert!(lit > 0, "NightVision continua todo preto com o timer em 10 s");
                        effect.update_sources(0, &FrameInfo::default());
                        gpu.submit(effect.command_buffer(0));
                        // Com timer = 0 ele ZERA a imagem (comportamento do próprio efeito), o que prova que o `timer` importa.
                        let still_lit = gpu.read(targets.images[0]).iter().filter(|p| p[..3].iter().any(|&c| c > 8)).count();
                        assert_eq!(still_lit, 0, "com timer = 0 o NightVision deveria sair preto");
                    }
                    effect.destroy(&gpu.device);
                    ran.push(name);
                }
                Err(Error::Unsupported(why)) => refused.push(format!("{name}: {why}")),
                Err(e) if survey.is_some() => failed.push(format!("{name}: {e}")),
                Err(e) => panic!("{name}: falha inesperada ao criar o efeito: {e}"),
            }
            if let Some(d) = depth_image {
                gpu.device.destroy_image(d.image, None);
                gpu.device.free_memory(d.memory, None);
            }
            gpu.destroy_targets(targets);
        }
    }
    eprintln!("\n{} efeitos executados: {}", ran.len(), ran.join(", "));
    eprintln!("{} falharam: {}", failed.len(), failed.join(" | "));
    eprintln!("{} recusados:", refused.len());
    for r in &refused {
        eprintln!("  {r}");
    }
}

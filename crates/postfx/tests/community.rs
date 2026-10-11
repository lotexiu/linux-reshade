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

    // LINUX_RESHADE_SURVEY_ONLY=Texto: só os bundles cujo nome contém o texto.
    let only = std::env::var("LINUX_RESHADE_SURVEY_ONLY").ok();
    let mut names: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().join("manifest.json").is_file())
        .filter(|e| only.as_deref().is_none_or(|o| e.file_name().to_string_lossy().contains(o)))
        .collect();
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
        for technique in 0..bundle.manifest.techniques.len() {
        let name = if technique == 0 { name.clone() } else { format!("{name}#{}", bundle.manifest.techniques[technique].name) };
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
            let mut created = Effect::new(&gpu.device, &gpu.memory, &bundle, technique, &target, gpu.queue);
            // Pede profundidade: dá uma imagem de profundidade como a de um jogo (cheia de 0,5).
            let depth_image = if matches!(&created, Err(Error::Unsupported(why)) if why.contains("profundidade")) {
                let depth = depth_image(&gpu, W, H, 0.5);
                let input = DepthInput { image: depth.image, format: DEPTH_FORMAT, layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL, extent: vk::Extent2D { width: W, height: H }, linear_filter: true };
                created = Effect::new(&gpu.device, &gpu.memory, &bundle, technique, &Target { depth: Some(input), ..target }, gpu.queue);
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
    }
    eprintln!("\n{} efeitos executados: {}", ran.len(), ran.join(", "));
    eprintln!("{} falharam: {}", failed.len(), failed.join(" | "));
    eprintln!("{} recusados:", refused.len());
    for r in &refused {
        eprintln!("  {r}");
    }
}

/// Executa uma cadeia de bundles já compilados (`LINUX_RESHADE_CHAIN_BUNDLES=dir1:dir2:...`, na ordem), com uma imagem de
/// profundidade falsa: o que o app de controle compilou para um jogo, para ver o que o driver da GPU aceita (com
/// `LINUX_RESHADE_TEST_HARDWARE=1`). `LINUX_RESHADE_CHAIN_SIZE=1920x1080` muda o tamanho do alvo (padrão 8x4),
/// `LINUX_RESHADE_CHAIN_SET=NOME=valor,...` escreve uniforms (inteiros ou decimais) em todas as etapas que os têm e
/// `LINUX_RESHADE_CHAIN_DEPTH=0.3` muda o valor da profundidade falsa (padrão 0,5) e
/// `LINUX_RESHADE_CHAIN_PNG=arquivo.ppm` grava o último quadro. Sem a primeira variável, não faz nada.
#[test]
fn a_chain_of_compiled_bundles_runs() {
    let Some(list) = std::env::var_os("LINUX_RESHADE_CHAIN_BUNDLES") else { return };
    let Some(gpu) = Gpu::software() else {
        eprintln!("sem dispositivo Vulkan; teste ignorado");
        return;
    };
    let (w, h) = std::env::var("LINUX_RESHADE_CHAIN_SIZE")
        .ok()
        .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))))
        .unwrap_or((W, H));
    let bundles: Vec<Bundle> = std::env::split_paths(&list).map(|p| Bundle::load(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).collect();
    let steps: Vec<postfx::Step<'_>> = bundles.iter().map(|b| postfx::Step { bundle: b, technique: 0 }).collect();
    unsafe {
        // Quadro de entrada: um degradê colorido com xadrez (algo para os efeitos enxergarem).
        let (image, image_mem) = gpu.target_sized(w, h, |x, y| [(x * 255 / w) as u8, (y * 255 / h) as u8, if (x / 64 + y / 64) % 2 == 0 { 200 } else { 60 }, 255]);
        let images = [image];
        let depth = depth_image(&gpu, w, h, std::env::var("LINUX_RESHADE_CHAIN_DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5));
        let input_depth = DepthInput { image: depth.image, format: DEPTH_FORMAT, layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL, extent: vk::Extent2D { width: w, height: h }, linear_filter: true };
        let target = Target { images: &images, format: FORMAT, extent: vk::Extent2D { width: w, height: h }, rest_layout: REST, queue_family: gpu.family, init_command_buffer: None, depth: Some(input_depth) };
        let started = std::time::Instant::now();
        let built = postfx::Chain::new_lenient(&gpu.device, &gpu.memory, &steps, &target, gpu.queue);
        eprintln!("cadeia construída em {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
        match built {
            Ok((mut chain, skipped)) => {
                eprintln!("cadeia criada: {} etapa(s), {} pulada(s)", chain.len(), skipped.len());
                for s in &skipped {
                    eprintln!("  pulada: etapa {}: {}", s.step, s.why);
                }
                // Uniforms pedidos.
                if let Ok(sets) = std::env::var("LINUX_RESHADE_CHAIN_SET") {
                    for item in sets.split(',').filter(|i| !i.is_empty()) {
                        let Some((name, value)) = item.split_once('=') else { continue };
                        for pos in 0..chain.len() {
                            let Some(origin) = chain.origin(pos) else { continue };
                            let m = &bundles[origin].manifest;
                            let Some(u) = m.uniforms.iter().find(|u| u.name == name) else { continue };
                            let bytes = match u.ty.base {
                                fx_bundle::manifest::ScalarBase::Float => value.parse::<f32>().unwrap().to_le_bytes().to_vec(),
                                fx_bundle::manifest::ScalarBase::Int => value.parse::<i32>().unwrap().to_le_bytes().to_vec(),
                                _ => value.parse::<u32>().unwrap().to_le_bytes().to_vec(),
                            };
                            chain.write_uniforms(pos, 0, u.offset as usize, &bytes);
                            eprintln!("  uniforme {name}={value} na etapa {pos}");
                        }
                    }
                }
                let frame = FrameInfo { time_ms: 10_000.0, frame_time_ms: 16.6, frame_count: 600, date: [2026.0, 10.0, 9.0, 43_200.0] };
                for n in 0..4 {
                    chain.update_sources(0, &FrameInfo { frame_count: 600 + n, ..frame });
                    gpu.submit_all(&chain.command_buffers(0));
                }
                eprintln!("4 quadros executados");
                // Diagnóstico: o que há em cada textura compartilhada pedida (`LINUX_RESHADE_CHAIN_DUMP=nome1,nome2`).
                if let Ok(names) = std::env::var("LINUX_RESHADE_CHAIN_DUMP") {
                    for name in names.split(',').filter(|n| !n.is_empty()) {
                        let Some(img) = chain.shared_image(name) else {
                            eprintln!("  {name}: não é uma textura compartilhada");
                            continue;
                        };
                        let tex = bundles.iter().flat_map(|b| b.manifest.textures.iter()).find(|t| t.unique_name == name).unwrap();
                        let (bytes_per, channels) = match tex.format {
                            fx_bundle::manifest::TextureFormat::Rgba16f => (8usize, 4usize),
                            fx_bundle::manifest::TextureFormat::Rg16f | fx_bundle::manifest::TextureFormat::Rg16 => (4, 2),
                            fx_bundle::manifest::TextureFormat::Rgba8 => (4, 4),
                            other => {
                                eprintln!("  {name}: formato {other:?} sem decodificador");
                                continue;
                            }
                        };
                        let raw = gpu.read_image_raw(img, tex.width, tex.height, bytes_per);
                        let half = |lo: u8, hi: u8| {
                            let v = u16::from_le_bytes([lo, hi]);
                            let (sign, exp, man) = ((v >> 15) & 1, ((v >> 10) & 0x1f) as i32, (v & 0x3ff) as f32);
                            let f = if exp == 0 { man / 1024.0 * 2f32.powi(-14) } else if exp == 31 { f32::INFINITY } else { (1.0 + man / 1024.0) * 2f32.powi(exp - 15) };
                            if sign == 1 { -f } else { f }
                        };
                        let (mut min, mut max, mut nonzero) = (f32::MAX, f32::MIN, 0usize);
                        for px in raw.chunks_exact(bytes_per) {
                            for c in 0..channels {
                                let v = match (tex.format, bytes_per / channels) {
                                    (fx_bundle::manifest::TextureFormat::Rg16, 2) => u16::from_le_bytes([px[c * 2], px[c * 2 + 1]]) as f32 / 65535.0,
                                    (fx_bundle::manifest::TextureFormat::Rgba8, 1) => px[c] as f32 / 255.0,
                                    _ => half(px[c * 2], px[c * 2 + 1]),
                                };
                                min = min.min(v);
                                max = max.max(v);
                                if v != 0.0 {
                                    nonzero += 1;
                                }
                            }
                        }
                        eprintln!("  {name} ({}x{} {:?}): min {min:.4} max {max:.4}, {nonzero} de {} valores não zero", tex.width, tex.height, tex.format, raw.len() / (bytes_per / channels));
                    }
                }
                if let Ok(path) = std::env::var("LINUX_RESHADE_CHAIN_PNG") {
                    let pixels = gpu.read_sized(image, w, h);
                    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
                    for p in &pixels {
                        out.extend_from_slice(&p[..3]);
                    }
                    std::fs::write(&path, out).unwrap();
                    eprintln!("quadro gravado em {path}");
                }
                chain.destroy(&gpu.device);
            }
            Err(e) => eprintln!("a cadeia falhou: {e}"),
        }
        gpu.device.destroy_image(depth.image, None);
        gpu.device.free_memory(depth.memory, None);
        gpu.device.destroy_image(image, None);
        gpu.device.free_memory(image_mem, None);
    }
}

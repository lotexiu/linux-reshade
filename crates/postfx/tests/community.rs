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
/// `LINUX_RESHADE_CHAIN_ACTIVE=AUTOFOCUS_RANGE` faz de conta que a pessoa edita esse controle (auxílios visuais), `LINUX_RESHADE_CHAIN_FRAMES=300` roda mais quadros (efeitos que convergem no tempo, como o autofoco),
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
    // `LINUX_RESHADE_CHAIN_ALL_TECHNIQUES=1`: uma etapa por técnica de cada bundle (o painel acrescenta todas as de um efeito).
        let all = std::env::var_os("LINUX_RESHADE_CHAIN_ALL_TECHNIQUES").is_some();
        // `LINUX_RESHADE_CHAIN_ONLY_TECHNIQUE=1`: só a técnica de índice 1 de cada bundle.
        let only: Option<usize> = std::env::var("LINUX_RESHADE_CHAIN_ONLY_TECHNIQUE").ok().and_then(|v| v.parse().ok());
        let steps: Vec<postfx::Step<'_>> = bundles
            .iter()
            .flat_map(|b| {
                let range = match only {
                    Some(n) => n..n + 1,
                    None => 0..if all { b.manifest.techniques.len() } else { 1 },
                };
                range.map(move |t| postfx::Step { bundle: b, technique: t })
            })
            .collect();
        let step_bundles: Vec<&Bundle> = steps.iter().map(|s| s.bundle).collect();
    unsafe {
        // Quadro de entrada: um degradê colorido com xadrez (algo para os efeitos enxergarem).
        // `LINUX_RESHADE_CHAIN_INPUT=aliased`: círculos e diagonais pretos e brancos, sem suavização (para antialiasing).
        let aliased = std::env::var("LINUX_RESHADE_CHAIN_INPUT").is_ok_and(|v| v == "aliased");
        let (image, image_mem) = gpu.target_sized(w, h, |x, y| {
            if aliased {
                let (dx, dy) = (x as f32 - w as f32 / 2.0, y as f32 - h as f32 / 2.0);
                let on = (dx * dx + dy * dy).sqrt() < h as f32 * 0.3 || ((x as i32 - y as i32).rem_euclid(200) < 40);
                let v = if on { 235 } else { 20 };
                [v, v, v, 255]
            } else {
                [(x * 255 / w) as u8, (y * 255 / h) as u8, if (x / 64 + y / 64) % 2 == 0 { 200 } else { 60 }, 255]
            }
        });
        let images = [image];
        let base: f32 = std::env::var("LINUX_RESHADE_CHAIN_DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5);
        // `LINUX_RESHADE_CHAIN_DEPTH_SPREAD=0.3`: a profundidade varia de `base` a `base + spread` da esquerda para a direita.
        let spread: f32 = std::env::var("LINUX_RESHADE_CHAIN_DEPTH_SPREAD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        // `LINUX_RESHADE_CHAIN_DEPTH_DISC=0.2`: um disco no centro (raio de 15% da largura) com essa profundidade (um personagem
        // perto, no meio de um cenário distante).
        let disc: Option<f32> = std::env::var("LINUX_RESHADE_CHAIN_DEPTH_DISC").ok().and_then(|v| v.parse().ok());
        let gradient = move |x: u32, y: u32| {
            let (dx, dy) = (x as f32 - w as f32 / 2.0, y as f32 - h as f32 / 2.0);
            match disc {
                Some(d) if (dx * dx + dy * dy).sqrt() < w as f32 * 0.15 => d,
                _ => base + spread * x as f32 / w as f32,
            }
        };
        // `LINUX_RESHADE_CHAIN_DEPTH_SIZE=1056x1056` e `LINUX_RESHADE_CHAIN_DEPTH_D16=1`: uma profundidade de outro tamanho e
        // formato que o da tela (a pessoa pode escolher qualquer candidata, até um mapa de sombras).
        let (dw, dh) = std::env::var("LINUX_RESHADE_CHAIN_DEPTH_SIZE")
            .ok()
            .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))))
            .unwrap_or((w, h));
        // `LINUX_RESHADE_CHAIN_DEPTH_D24S8=1`: o formato do A Hat in Time (D24_UNORM_S8, com aspecto de estêncil).
        let depth_format = if std::env::var_os("LINUX_RESHADE_CHAIN_DEPTH_D16").is_some() {
            vk::Format::D16_UNORM
        } else if std::env::var_os("LINUX_RESHADE_CHAIN_DEPTH_D24S8").is_some() {
            vk::Format::D24_UNORM_S8_UINT
        } else {
            DEPTH_FORMAT
        };
        let depth = depth_image_with(&gpu, depth_format, dw, dh, base, ((spread != 0.0 || disc.is_some()) && depth_format == DEPTH_FORMAT && (dw, dh) == (w, h)).then_some(&gradient));
        let input_depth = DepthInput { image: depth.image, format: depth_format, layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL, extent: vk::Extent2D { width: dw, height: dh }, linear_filter: true };
        let target = Target { images: &images, format: FORMAT, extent: vk::Extent2D { width: w, height: h }, rest_layout: REST, queue_family: gpu.family, init_command_buffer: None, depth: Some(input_depth) };
        // `LINUX_RESHADE_CHAIN_PROFILE=1`: o tempo da GPU de cada passe (média dos quadros), do mais caro ao mais barato.
        let profile = std::env::var_os("LINUX_RESHADE_CHAIN_PROFILE").is_some();
        if profile {
            // SAFETY: o teste roda sozinho nesta thread antes de criar a cadeia.
            std::env::set_var("LINUX_RESHADE_PROFILE_PASSES", "1");
        }
        let started = std::time::Instant::now();
        let built = postfx::Chain::new_lenient(&gpu.device, &gpu.memory, &steps, &target, gpu.queue);
        eprintln!("cadeia construída em {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
        match built {
            Ok((mut chain, skipped)) => {
                eprintln!("cadeia criada: {} etapa(s), {} pulada(s)", chain.len(), skipped.len());
                for s in &skipped {
                    eprintln!("  pulada: etapa {}: {}", s.step, s.why);
                }
                // Os valores padrão do efeito (a layer os recebe pelo plano): sem isso, limiares e passos ficam em zero.
                for pos in 0..chain.len() {
                    if let Some(origin) = chain.origin(pos) {
                        chain.write_uniforms(pos, 0, 0, &fx_bundle::uniforms::default_block(&step_bundles[origin].manifest));
                    }
                }
                // Uniforms pedidos.
                if let Ok(sets) = std::env::var("LINUX_RESHADE_CHAIN_SET") {
                    for item in sets.split(',').filter(|i| !i.is_empty()) {
                        let Some((name, value)) = item.split_once('=') else { continue };
                        for pos in 0..chain.len() {
                            let Some(origin) = chain.origin(pos) else { continue };
                            let m = &step_bundles[origin].manifest;
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
                if let Ok(name) = std::env::var("LINUX_RESHADE_CHAIN_ACTIVE") {
                    chain.set_active_control(Some(&name));
                }
                let frames: u32 = std::env::var("LINUX_RESHADE_CHAIN_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
                let mut sums: std::collections::BTreeMap<(usize, String), (f64, u32)> = std::collections::BTreeMap::new();
                let tick_ns = gpu.instance.get_physical_device_properties(gpu.physical).limits.timestamp_period;
                for n in 0..frames {
                    chain.update_sources(0, &FrameInfo { frame_count: 600 + n, ..frame });
                    gpu.submit_all(&chain.command_buffers(0));
                    // Os primeiros quadros pagam compilação e cache frio: só conta do quadro 5 em diante.
                    if profile && n >= 5 {
                        for (effect, label, us) in chain.pass_times(&gpu.device, tick_ns) {
                            let entry = sums.entry((effect, label)).or_default();
                            entry.0 += f64::from(us);
                            entry.1 += 1;
                        }
                    }
                }
                if profile {
                    let mut rows: Vec<_> = sums.into_iter().map(|((e, l), (sum, n))| (e, l, sum / f64::from(n.max(1)))).collect();
                    let total: f64 = rows.iter().map(|r| r.2).sum();
                    rows.sort_by(|a, b| b.2.total_cmp(&a.2));
                    eprintln!("PERFIL por passe (média, total {:.2} ms):", total / 1000.0);
                    let mut per_effect: std::collections::BTreeMap<usize, f64> = std::collections::BTreeMap::new();
                    for (e, _, us) in &rows {
                        *per_effect.entry(*e).or_default() += us;
                    }
                    let mut per_effect: Vec<_> = per_effect.into_iter().collect();
                    per_effect.sort_by(|a, b| b.1.total_cmp(&a.1));
                    for (e, us) in &per_effect {
                        eprintln!("  TOTAL do efeito {e} ({}): {:.2} ms", step_bundles.get(*e).map_or("?".to_string(), |b| b.manifest.techniques.first().map_or("?".to_string(), |t| t.name.clone())), us / 1000.0);
                    }
                    for (e, l, us) in rows.iter().take(25) {
                        eprintln!("  efeito {e} passe {l}: {:.3} ms", us / 1000.0);
                    }
                }
                eprintln!("{frames} quadros executados");
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

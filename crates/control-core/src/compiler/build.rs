//! A compilação em si: chamar o `fx2spv`, aplicar os remendos e o otimizador, e publicar o bundle completo de uma vez.

use super::*;

/// Executa o comando, repetindo um pouco se o executável estiver "ocupado" (`ETXTBSY`): acontece
/// quando ele acabou de ser escrito (por exemplo, o `fx2spv` sendo recompilado) e algum `fork`
/// concorrente ainda segura o arquivo aberto para escrita. Passa em milissegundos.
pub(super) fn run_retrying_busy(command: &mut Command) -> std::io::Result<std::process::Output> {
    let mut attempts = 0;
    loop {
        match command.output() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            other => return other,
        }
    }
}

/// As imagens do bundle ainda correspondem aos arquivos de origem (nenhum foi editado depois da compilação)?
pub(super) fn images_up_to_date(dir: &Path, compiled: SystemTime) -> bool {
    let Ok(list) = std::fs::read_to_string(dir.join(IMAGE_SOURCES)) else { return true };
    list.lines().all(|path| modified(Path::new(path)).is_some_and(|edited| edited <= compiled))
}

/// O arquivo `source` dentro das pastas de texturas: o nome exato e, se não houver, sem diferenciar maiúsculas
/// (os efeitos foram escritos no Windows) e com `\` como separador.
/// O otimizador de SPIR-V (`spirv-opt`), se existe e a pessoa não o desligou. Medido em 145 efeitos na GPU real: a saída é a mesma
/// (nenhum passou de 1e-3 de diferença no primeiro quadro) e a GPU gasta uns 5 a 10% menos.
pub(super) fn optimizer() -> Option<&'static Path> {
    static TOOL: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    TOOL.get_or_init(|| {
        if !Config::load().optimize_shaders {
            return None;
        }
        let candidate = std::env::var_os("LINUX_RESHADE_SPIRV_OPT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("spirv-opt"));
        Command::new(&candidate).arg("--version").output().ok().filter(|o| o.status.success()).map(|_| candidate)
    })
    .as_deref()
}

/// Otimiza (em várias threads) cada `.spv` do bundle. Um módulo que o otimizador recusa, ou que não valida depois (quando o
/// `spirv-validator` existe), fica como estava.
pub(super) fn optimize_bundle(dir: &Path, tool: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "spv")).collect();
    let validator = Command::new("spirv-val").arg("--version").output().ok().filter(|o| o.status.success()).is_some();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).min(8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(file) = files.get(i) else { break };
                    let out = file.with_extension("spv.opt");
                    // `--loop-unroll`: os laços marcados `[unroll]` viram sequências, e o deslocamento de `tex2D(s, uv, int2(x, y))` deixa de ser
                    // calculado em tempo de execução: o `spirv-opt` o troca por `ConstOffset`, o único que o Vulkan 1.0 aceita em amostragem
                    // (o gerador do ReShade emite `Offset`, que o driver pode executar de qualquer jeito: um Xid 69 da NVIDIA no
                    // ExposureFusion do iMMERSE). `--skip-validation`: a entrada ainda é a inválida; a saída é validada abaixo.
                    let optimized = Command::new(tool).arg("--skip-validation").arg("--loop-unroll").arg("-O").arg("--target-env=vulkan1.0").arg(file).arg("-o").arg(&out).output().is_ok_and(|o| o.status.success());
                    let valid = optimized && (!validator || Command::new("spirv-val").arg("--target-env").arg("vulkan1.0").arg(&out).output().is_ok_and(|o| o.status.success()));
                    if valid {
                        let _ = std::fs::rename(&out, file);
                    } else {
                        let _ = std::fs::remove_file(&out);
                    }
                }
            });
        }
    });
}

pub(super) fn patches_up_to_date(dir: &Path, name: &str, source: &Path) -> bool {
    let wanted = crate::patches::applicable(base_name(name), source);
    let have = std::fs::read_to_string(dir.join(PATCHES_MARKER)).unwrap_or_default();
    have.lines().filter(|l| !l.is_empty()).eq(wanted.iter().copied())
}

impl Compiler {
    /// Garante um bundle atual de `name` para `width`x`height`: usa o do cache se o `.fx` não mudou
    /// desde a compilação, senão compila. (Mudanças só num `#include` não são notadas.)
    pub fn ensure(&self, name: &str, width: u32, height: u32) -> Result<Ensured, CompileError> {
        self.ensure_with(name, width, height, &[])
    }

    /// Como [`ensure`](Self::ensure), compilando com `defines` (macros do preset e do efeito). Cada combinação tem o
    /// seu bundle (ver [`variant_key`]).
    pub fn ensure_with(&self, name: &str, width: u32, height: u32, defines: &[(String, String)]) -> Result<Ensured, CompileError> {
        let source = self.source(name)?;
        let key = variant_key(name, defines);
        let dir = self.bundle_dir(&key, width, height);
        if self.is_current(&key, width, height) {
            return Ok(Ensured::Cached(dir));
        }
        std::fs::create_dir_all(&self.cache).map_err(|e| CompileError::Io(format!("{}: {e}", self.cache.display())))?;
        // O bundle é montado numa pasta de trabalho e só entra no lugar pronto (já otimizado, com as imagens): o compilador grava
        // primeiro o SPIR-V que o Vulkan não perdoa, e uma layer que carregasse o bundle nessa hora (um jogo abrindo enquanto o
        // daemon recompila) mandaria à GPU o código inválido: Xid 69 da NVIDIA, com o jogo congelado.
        let work = self.cache.join("building").join(format!("{}.{}", dir.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(work.parent().unwrap_or(&self.cache)).map_err(|e| CompileError::Io(format!("{}: {e}", self.cache.display())))?;
        let mut command = Command::new(&self.fx2spv);
        command.arg("--width").arg(width.to_string()).arg("--height").arg(height.to_string());
        for include in config::expand(&self.includes) {
            command.arg("-I").arg(include);
        }
        for (name, value) in defines {
            if !valid_define(name, value) {
                return Err(CompileError::BadName(format!("{name}={value}")));
            }
            command.arg("-D").arg(format!("{name}={value}"));
        }
        // Correções de código que o Vulkan não perdoa (ver `patches`): compila uma cópia corrigida, e os `#include` relativos
        // continuam a valer pelo diretório do arquivo original.
        let applied = crate::patches::applicable(base_name(name), &source);
        let source = if applied.is_empty() {
            source
        } else {
            let text = std::fs::read_to_string(&source).map_err(|e| CompileError::Io(format!("{}: {e}", source.display())))?;
            let (patched, _) = crate::patches::apply(base_name(name), &text);
            // Uma pasta de trabalho com links para tudo o que há ao lado do original e a cópia corrigida no lugar dele: os `#include`
            // relativos resolvem pelo mesmo caminho de sempre (o mesmo cabeçalho por dois caminhos entraria duas vezes).
            let staging = self.cache.join("patched").join(dir.file_name().unwrap_or_default());
            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging).map_err(|e| CompileError::Io(format!("{}: {e}", staging.display())))?;
            let copy = staging.join(format!("{}.fx", base_name(name)));
            if let Some(parent) = source.parent() {
                for entry in std::fs::read_dir(parent).map_err(|e| CompileError::Io(format!("{}: {e}", parent.display())))?.flatten() {
                    if entry.path() != source {
                        let _ = std::os::unix::fs::symlink(entry.path(), staging.join(entry.file_name()));
                    }
                }
            }
            std::fs::write(&copy, patched).map_err(|e| CompileError::Io(format!("{}: {e}", copy.display())))?;
            copy
        };
        command.arg("-o").arg(&work).arg(&source);
        let output = run_retrying_busy(&mut command).map_err(|e| CompileError::Io(format!("{}: {e}", self.fx2spv.display())))?;
        if !output.status.success() {
            let _ = std::fs::remove_dir_all(&work);
            return Err(CompileError::Tool {
                status: output.status.to_string(),
                output: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let _ = std::fs::write(work.join(PATCHES_MARKER), applied.join("\n"));
        if let Some(tool) = optimizer() {
            optimize_bundle(&work, tool);
            let _ = std::fs::write(work.join(OPTIMIZED_MARKER), OPTIMIZE_VERSION);
        }
        if let Err(e) = self.write_images(&work) {
            // Um bundle sem as imagens não serve e não pode passar por "atual" na próxima olhada.
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
        // Publica: troca a pasta de uma vez (a antiga sai do caminho com um rename, nunca é editada no lugar).
        let old = work.with_extension(format!("old{}", std::process::id()));
        let had_old = dir.exists() && std::fs::rename(&dir, &old).is_ok();
        if let Err(e) = std::fs::rename(&work, &dir) {
            if had_old {
                let _ = std::fs::rename(&old, &dir);
            }
            let _ = std::fs::remove_dir_all(&work);
            return Err(CompileError::Io(format!("{}: {e}", dir.display())));
        }
        if had_old {
            let _ = std::fs::remove_dir_all(&old);
        }
        Ok(Ensured::Compiled(dir))
    }

    /// Decodifica as imagens das texturas de arquivo do bundle recém-compilado e as grava nele.
    pub(super) fn write_images(&self, dir: &Path) -> Result<(), CompileError> {
        let _ = std::fs::remove_dir_all(dir.join("images")); // de uma compilação anterior
        let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else { return Ok(()) };
        let Ok(manifest) = fx_bundle::Manifest::parse(&text) else { return Ok(()) }; // quem carrega o bundle reclama
        let dirs = config::expand(&self.textures);
        let mut sources = Vec::new();
        for (index, texture) in manifest.textures.iter().enumerate() {
            let Some(source) = fx_bundle::images::source_of(texture) else { continue };
            let path = find_texture(&dirs, source).ok_or_else(|| {
                let searched: Vec<_> = dirs.iter().map(|d| d.display().to_string()).collect();
                CompileError::Texture(fmt("could not find the image {} of the texture {} (searched in: {})", &[&format!("{:?}", source), &format!("{:?}", texture.name), &(searched.join(", "))]))
            })?;
            let bytes = std::fs::read(&path).map_err(|e| CompileError::Texture(format!("{}: {e}", path.display())))?;
            let decoded = image_file::decode(source, &bytes).map_err(|e| CompileError::Texture(format!("{}: {e}", path.display())))?;
            let image = fx_bundle::images::TextureImage { width: decoded.width, height: decoded.height, rgba: decoded.pixels };
            // O ReShade estica a imagem ao tamanho que o efeito declara para a textura.
            let image = if u64::from(texture.width) * u64::from(texture.height) * 4 <= fx_bundle::images::MAX_IMAGE_BYTES { image.resized(texture.width, texture.height) } else { image };
            fx_bundle::images::write(dir, index, &image).map_err(|e| CompileError::Io(format!("{}: {e}", dir.display())))?;
            sources.push(path.display().to_string());
        }
        if !sources.is_empty() {
            std::fs::write(dir.join(IMAGE_SOURCES), sources.join("\n")).map_err(|e| CompileError::Io(format!("{}: {e}", dir.display())))?;
        }
        Ok(())
    }
}

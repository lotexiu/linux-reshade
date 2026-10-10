//! Validação do manifesto: tudo que o runtime assume sem conferir de novo.

use crate::manifest::*;
use crate::Error;
use std::collections::HashSet;

/// Mínimo que o Vulkan garante para `maxUniformBufferRange`.
pub const MAX_UNIFORM_BYTES: u32 = 16384;
/// Máximo de anexos de cor por passe.
pub const MAX_RENDER_TARGETS: usize = 8;

fn invalid(why: impl Into<String>) -> Error {
    Error::Invalid(why.into())
}

fn unsupported(what: impl Into<String>) -> Error {
    Error::Unsupported(what.into())
}

/// Nome de arquivo simples: sem diretório, sem `..`, terminando em `.spv`.
fn is_plain_spv_name(name: &str) -> bool {
    !name.is_empty()
        && name.ends_with(".spv")
        && !name.contains(['/', '\\', '\0'])
        && name != ".."
        && !name.starts_with('.')
}

impl Manifest {
    /// Confere o manifesto inteiro. `Unsupported` = válido, mas fora do que a v1 do runtime executa.
    pub fn validate(&self) -> Result<(), Error> {
        if self.format != FORMAT {
            return Err(invalid(format!("format {:?}, esperado {FORMAT:?}", self.format)));
        }
        if self.version != VERSION {
            return Err(unsupported(format!("versão {} do formato (esta build lê a {VERSION})", self.version)));
        }
        if self.spirv_version != "1.0" && self.spirv_version != "1.3" {
            return Err(invalid(format!("spirv_version {:?}", self.spirv_version)));
        }
        if self.target.width == 0 || self.target.height == 0 {
            return Err(invalid("target com largura ou altura zero"));
        }
        if self.total_uniform_size > MAX_UNIFORM_BYTES {
            return Err(unsupported(format!(
                "buffer de uniforms de {} bytes (máximo {MAX_UNIFORM_BYTES})",
                self.total_uniform_size
            )));
        }

        let mut entries = HashSet::new();
        for e in &self.entry_points {
            if !is_plain_spv_name(&e.file) {
                return Err(invalid(format!("arquivo de entry point {:?} não é um nome simples .spv", e.file)));
            }
            if !entries.insert(e.name.as_str()) {
                return Err(invalid(format!("entry point duplicado: {:?}", e.name)));
            }
        }

        let mut textures = HashSet::new();
        for t in &self.textures {
            if !textures.insert(t.unique_name.as_str()) {
                return Err(invalid(format!("textura duplicada: {:?}", t.unique_name)));
            }
            if t.kind != TextureType::D2 {
                return Err(unsupported(format!("textura {:?} não é 2D", t.name)));
            }
            // DEPTH pode ser só declarada: quase todo efeito inclui o ReShade.fxh, que a declara. O que a
            // v1 não executa é um passe que a AMOSTRA (ver `validate_pass`).
            match t.semantic.as_deref() {
                None | Some("COLOR") | Some("DEPTH") => {}
                Some(other) => return Err(unsupported(format!("semântica {other:?} da textura {:?}", t.name))),
            }
            if t.semantic.is_none() && (t.width == 0 || t.height == 0 || t.format == TextureFormat::Unknown) {
                return Err(invalid(format!("textura {:?} sem tamanho ou formato", t.name)));
            }
            if t.levels == 0 {
                return Err(invalid(format!("textura {:?} com 0 níveis", t.name)));
            }
        }

        for s in &self.samplers {
            if !textures.contains(s.texture.as_str()) {
                return Err(invalid(format!("sampler {:?} usa a textura inexistente {:?}", s.name, s.texture)));
            }
        }

        let mut uniforms = HashSet::new();
        for u in &self.uniforms {
            if !uniforms.insert(u.unique_name.as_str()) {
                return Err(invalid(format!("uniform duplicado: {:?}", u.unique_name)));
            }
            if matches!(u.ty.base, ScalarBase::String | ScalarBase::Other) {
                return Err(unsupported(format!("uniform {:?} de tipo não numérico", u.name)));
            }
            if !(1..=4).contains(&u.ty.rows) || !(1..=4).contains(&u.ty.cols) {
                return Err(invalid(format!("uniform {:?} com {}x{} componentes", u.name, u.ty.rows, u.ty.cols)));
            }
            let end = u.offset.checked_add(u.size).ok_or_else(|| invalid(format!("uniform {:?}: offset + size estoura", u.name)))?;
            if end > self.total_uniform_size {
                return Err(invalid(format!("uniform {:?} termina em {end}, além de total_uniform_size", u.name)));
            }
        }

        if self.techniques.is_empty() {
            return Err(invalid("bundle sem nenhuma técnica"));
        }
        for tech in &self.techniques {
            if tech.passes.is_empty() {
                return Err(invalid(format!("técnica {:?} sem passes", tech.name)));
            }
            for (i, pass) in tech.passes.iter().enumerate() {
                self.validate_pass(&tech.name, i, pass, &entries)?;
            }
        }
        Ok(())
    }

    fn entry_stage(&self, name: &str) -> Option<Stage> {
        self.entry_points.iter().find(|e| e.name == name).map(|e| e.stage)
    }

    fn validate_pass(&self, tech: &str, index: usize, pass: &Pass, entries: &HashSet<&str>) -> Result<(), Error> {
        let at = format!("técnica {tech:?}, passe {index}");
        if pass.compute_entry.is_some() {
            return Err(unsupported(format!("{at}: passe de compute")));
        }
        for (entry, want) in [(&pass.vertex_entry, Stage::Vertex), (&pass.pixel_entry, Stage::Pixel)] {
            if !entries.contains(entry.as_str()) {
                return Err(invalid(format!("{at}: entry point {entry:?} não existe")));
            }
            if self.entry_stage(entry) != Some(want) {
                return Err(invalid(format!("{at}: entry point {entry:?} não é do estágio {want:?}")));
            }
        }
        if pass.render_targets.len() > MAX_RENDER_TARGETS {
            return Err(invalid(format!("{at}: mais de {MAX_RENDER_TARGETS} anexos")));
        }
        let mut targets = HashSet::new();
        for rt in pass.render_targets.iter().flatten() {
            let tex = self
                .textures
                .iter()
                .find(|t| &t.unique_name == rt)
                .ok_or_else(|| invalid(format!("{at}: anexo {rt:?} não é uma textura")))?;
            if !tex.render_target {
                return Err(invalid(format!("{at}: a textura {rt:?} não é um alvo de renderização")));
            }
            targets.insert(rt.as_str());
        }
        if pass.blend.len() < pass.render_targets.len().max(1) {
            return Err(invalid(format!("{at}: faltam estados de blend")));
        }
        // `VertexCount = 0` é válido (o ReShade o usa em técnicas utilitárias que não desenham): o desenho é vazio.
        let mut bindings = HashSet::new();
        for b in &pass.samplers {
            let sampler = self
                .samplers
                .get(b.sampler as usize)
                .ok_or_else(|| invalid(format!("{at}: sampler {} não existe", b.sampler)))?;
            if !bindings.insert(b.binding) {
                return Err(invalid(format!("{at}: binding {} usado duas vezes", b.binding)));
            }
            // Amostrar `DEPTH` é válido no bundle; quem decide se dá é o runtime (precisa de uma imagem de profundidade).
            if targets.contains(sampler.texture.as_str()) {
                return Err(invalid(format!("{at}: amostra a textura que é o próprio alvo ({:?})", sampler.texture)));
            }
        }
        Ok(())
    }
}

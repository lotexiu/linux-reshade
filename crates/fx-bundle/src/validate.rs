//! Validação do manifesto: tudo que o runtime assume sem conferir de novo.

use crate::manifest::*;
use crate::Error;
use std::collections::HashSet;
use i18n::{fmt, tr};

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
            return Err(invalid(fmt("format {}, expected {}", &[&format!("{:?}", self.format), &format!("{:?}", FORMAT)])));
        }
        if !(1..=VERSION).contains(&self.version) {
            return Err(unsupported(fmt("format version {} (this build reads up to {})", &[&self.version, &VERSION])));
        }
        if self.spirv_version != "1.0" && self.spirv_version != "1.3" {
            return Err(invalid(format!("spirv_version {:?}", self.spirv_version)));
        }
        if self.target.width == 0 || self.target.height == 0 {
            return Err(invalid(tr("target with zero width or height")));
        }
        if self.total_uniform_size > MAX_UNIFORM_BYTES {
            return Err(unsupported(fmt("uniform buffer of {} bytes (maximum {})", &[&self.total_uniform_size, &MAX_UNIFORM_BYTES])));
        }

        let mut entries = HashSet::new();
        for e in &self.entry_points {
            if !is_plain_spv_name(&e.file) {
                return Err(invalid(fmt("entry point file {} is not a plain .spv name", &[&format!("{:?}", e.file)])));
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
            // Texturas 1D e 3D só como amostradas ou de armazenamento: não são alvos de renderização (ver `validate_pass`).
            if t.kind != TextureType::D2 && (t.render_target || t.semantic.is_some()) {
                return Err(unsupported(fmt("1D or 3D texture {} used as a target or semantic", &[&format!("{:?}", t.name)])));
            }
            // DEPTH pode ser só declarada: quase todo efeito inclui o ReShade.fxh, que a declara. O que a
            // v1 não executa é um passe que a AMOSTRA (ver `validate_pass`).
            match t.semantic.as_deref() {
                None | Some("COLOR") | Some("DEPTH") => {}
                Some(other) => return Err(unsupported(fmt("semantic {} of texture {}", &[&format!("{:?}", other), &format!("{:?}", t.name)]))),
            }
            if t.semantic.is_none() && (t.width == 0 || t.height == 0 || t.format == TextureFormat::Unknown) {
                return Err(invalid(fmt("texture {} without size or format", &[&format!("{:?}", t.name)])));
            }
            if t.levels == 0 {
                return Err(invalid(fmt("texture {} with 0 levels", &[&format!("{:?}", t.name)])));
            }
        }

        for st in &self.storages {
            if !textures.contains(st.texture.as_str()) {
                return Err(invalid(format!("storage {:?} usa a textura inexistente {:?}", st.name, st.texture)));
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
                return Err(unsupported(fmt("uniform {} of non-numeric type", &[&format!("{:?}", u.name)])));
            }
            if !(1..=4).contains(&u.ty.rows) || !(1..=4).contains(&u.ty.cols) {
                return Err(invalid(fmt("uniform {} with {}x{} components", &[&format!("{:?}", u.name), &u.ty.rows, &u.ty.cols])));
            }
            let end = u.offset.checked_add(u.size).ok_or_else(|| invalid(format!("uniform {:?}: offset + size estoura", u.name)))?;
            if end > self.total_uniform_size {
                return Err(invalid(fmt("uniform {} ends at {}, beyond total_uniform_size", &[&format!("{:?}", u.name), &end])));
            }
        }

        if self.techniques.is_empty() {
            return Err(invalid(tr("bundle without any technique")));
        }
        for tech in &self.techniques {
            if tech.passes.is_empty() {
                return Err(invalid(fmt("technique {} without passes", &[&format!("{:?}", tech.name)])));
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
        let at = fmt("technique {}, pass {}", &[&format!("{:?}", tech), &index]);
        if pass.compute_entry.is_some() {
            return self.validate_compute_pass(&at, pass, entries);
        }
        for (entry, want) in [(&pass.vertex_entry, Stage::Vertex), (&pass.pixel_entry, Stage::Pixel)] {
            if !entries.contains(entry.as_str()) {
                return Err(invalid(fmt("{}: entry point {} does not exist", &[&at, &format!("{:?}", entry)])));
            }
            if self.entry_stage(entry) != Some(want) {
                return Err(invalid(fmt("{}: entry point {} is not of stage {}", &[&at, &format!("{:?}", entry), &format!("{:?}", want)])));
            }
        }
        if pass.render_targets.len() > MAX_RENDER_TARGETS {
            return Err(invalid(fmt("{}: more than {} attachments", &[&at, &MAX_RENDER_TARGETS])));
        }
        let mut targets = HashSet::new();
        for rt in pass.render_targets.iter().flatten() {
            let tex = self
                .textures
                .iter()
                .find(|t| &t.unique_name == rt)
                .ok_or_else(|| invalid(fmt("{}: attachment {} is not a texture", &[&at, &format!("{:?}", rt)])))?;
            if !tex.render_target {
                return Err(invalid(fmt("{}: texture {} is not a render target", &[&at, &format!("{:?}", rt)])));
            }
            targets.insert(rt.as_str());
        }
        if pass.blend.len() < pass.render_targets.len().max(1) {
            return Err(invalid(fmt("{}: blend states are missing", &[&at])));
        }
        // `VertexCount = 0` é válido (o ReShade o usa em técnicas utilitárias que não desenham): o desenho é vazio.
        let mut bindings = HashSet::new();
        for b in &pass.samplers {
            let sampler = self
                .samplers
                .get(b.sampler as usize)
                .ok_or_else(|| invalid(fmt("{}: sampler {} does not exist", &[&at, &b.sampler])))?;
            if !bindings.insert(b.binding) {
                return Err(invalid(format!("{at}: binding {} usado duas vezes", b.binding)));
            }
            // Amostrar `DEPTH` é válido no bundle; quem decide se dá é o runtime (precisa de uma imagem de profundidade).
            if targets.contains(sampler.texture.as_str()) {
                return Err(invalid(fmt("{}: samples the texture that is its own target ({})", &[&at, &format!("{:?}", sampler.texture)])));
            }
        }
        Ok(())
    }

    /// Passe de compute: sem render pass, só um entry point de compute, o tamanho do despacho e as imagens de armazenamento.
    fn validate_compute_pass(&self, at: &str, pass: &Pass, entries: &HashSet<&str>) -> Result<(), Error> {
        if self.version < 2 {
            return Err(invalid(fmt("{}: compute pass in a version 1 bundle", &[&at])));
        }
        let entry = pass.compute_entry.as_deref().unwrap_or_default();
        if !entries.contains(entry) {
            return Err(invalid(fmt("{}: entry point {} does not exist", &[&at, &format!("{:?}", entry)])));
        }
        if self.entry_stage(entry) != Some(Stage::Compute) {
            return Err(invalid(fmt("{}: entry point {} is not a compute one", &[&at, &format!("{:?}", entry)])));
        }
        let Some(dispatch) = pass.dispatch else {
            return Err(invalid(fmt("{}: compute pass without a dispatch size", &[&at])));
        };
        if dispatch.contains(&0) {
            return Err(invalid(fmt("{}: dispatch with a zero dimension", &[&at])));
        }
        let mut bindings = HashSet::new();
        for b in &pass.storages {
            if self.storages.get(b.storage as usize).is_none() {
                return Err(invalid(fmt("{}: storage {} does not exist", &[&at, &b.storage])));
            }
            if !bindings.insert(b.binding) {
                return Err(invalid(fmt("{}: storage binding {} used twice", &[&at, &b.binding])));
            }
        }
        let mut sampler_bindings = HashSet::new();
        for b in &pass.samplers {
            if self.samplers.get(b.sampler as usize).is_none() {
                return Err(invalid(fmt("{}: sampler {} does not exist", &[&at, &b.sampler])));
            }
            if !sampler_bindings.insert(b.binding) {
                return Err(invalid(format!("{at}: binding {} usado duas vezes", b.binding)));
            }
        }
        Ok(())
    }
}

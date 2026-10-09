//! Tipos do `manifest.json` (ver `docs/bundle-format.md`). Campos desconhecidos são ignorados, para
//! um runtime mais velho ainda ler um bundle com campos novos; já um valor desconhecido num campo
//! enumerado (por exemplo um novo modo de blend) é erro, porque o runtime não saberia aplicá-lo.

use crate::Error;
use serde::Deserialize;
use std::collections::HashMap;

pub const FORMAT: &str = "linux-reshade-bundle";
pub const VERSION: u32 = 1;

/// Valor de uma anotação ou de um default: escalar ou lista de escalares.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Scalar(Scalar),
    List(Vec<Scalar>),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Number(f64),
    Text(String),
}

pub type Annotations = HashMap<String, Value>;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub source: String,
    pub compiler: Compiler,
    pub target: Target,
    #[serde(default)]
    pub defines: HashMap<String, String>,
    pub spirv_version: String,
    /// O vertex shader já inverte o Y (clip space do D3D para o do Vulkan). Sem isso a imagem sairia
    /// de cabeça para baixo, a não ser que o runtime use viewport de altura negativa.
    #[serde(default)]
    pub y_flipped: bool,
    pub entry_points: Vec<EntryPoint>,
    pub total_uniform_size: u32,
    #[serde(default)]
    pub textures: Vec<Texture>,
    #[serde(default)]
    pub samplers: Vec<Sampler>,
    #[serde(default)]
    pub uniforms: Vec<Uniform>,
    pub techniques: Vec<Technique>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Compiler {
    pub name: String,
    pub reshade_commit: String,
    pub spirv_headers_commit: String,
}

/// Valores das macros `BUFFER_*` com que o efeito foi compilado.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Target {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Vertex,
    Pixel,
    Compute,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntryPoint {
    pub name: String,
    pub stage: Stage,
    pub file: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureType {
    #[serde(rename = "1d")]
    D1,
    #[serde(rename = "2d")]
    D2,
    #[serde(rename = "3d")]
    D3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureFormat {
    R8, R16f, R16, R32f, R32u, R32i,
    Rg8, Rg16f, Rg16, Rg32f,
    Rgba8, Rgba16f, Rgba16, Rgba32f, Rgba32u, Rgba32i,
    Rgb10a2, Rg11b10f,
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Texture {
    pub name: String,
    pub unique_name: String,
    /// `"COLOR"` (o frame do jogo), `"DEPTH"` ou nada.
    pub semantic: Option<String>,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub levels: u32,
    #[serde(rename = "type")]
    pub kind: TextureType,
    pub format: TextureFormat,
    pub render_target: bool,
    #[serde(default)]
    pub annotations: Annotations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Point,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Address {
    Wrap,
    Mirror,
    Clamp,
    Border,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Sampler {
    pub name: String,
    pub unique_name: String,
    /// `unique_name` da textura amostrada.
    pub texture: String,
    pub min_filter: Filter,
    pub mag_filter: Filter,
    pub mip_filter: Filter,
    pub anisotropic: bool,
    pub address_u: Address,
    pub address_v: Address,
    pub address_w: Address,
    pub min_lod: f32,
    pub max_lod: f32,
    pub lod_bias: f32,
    pub srgb: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarBase {
    Bool,
    Int,
    Uint,
    Float,
    /// Strings e outros tipos não existem num buffer de uniforms.
    String,
    Other,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct UniformType {
    pub base: ScalarBase,
    pub rows: u32,
    pub cols: u32,
    /// 0 = não é array.
    pub array: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Uniform {
    pub name: String,
    pub unique_name: String,
    #[serde(rename = "type")]
    pub ty: UniformType,
    pub offset: u32,
    pub size: u32,
    pub default: Option<Value>,
    #[serde(default)]
    pub annotations: Annotations,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Technique {
    pub name: String,
    #[serde(default)]
    pub annotations: Annotations,
    pub passes: Vec<Pass>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendFactor {
    Zero,
    One,
    SourceColor,
    OneMinusSourceColor,
    DestColor,
    OneMinusDestColor,
    SourceAlpha,
    OneMinusSourceAlpha,
    DestAlpha,
    OneMinusDestAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendOp {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Blend {
    pub enable: bool,
    pub src_color: BlendFactor,
    pub dst_color: BlendFactor,
    pub color_op: BlendOp,
    pub src_alpha: BlendFactor,
    pub dst_alpha: BlendFactor,
    pub alpha_op: BlendOp,
    pub write_mask: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topology {
    PointList,
    LineList,
    LineStrip,
    TriangleList,
    TriangleStrip,
}

/// Um sampler usado por um passe e o binding dele (set 1) nos módulos desse passe.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SamplerBinding {
    pub sampler: u32,
    pub binding: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pass {
    pub name: String,
    pub vertex_entry: String,
    pub pixel_entry: String,
    pub compute_entry: Option<String>,
    /// `unique_name` das texturas de saída; `None` (ou lista vazia) = o frame do jogo.
    pub render_targets: Vec<Option<String>>,
    pub samplers: Vec<SamplerBinding>,
    pub clear: bool,
    pub srgb_write: bool,
    pub blend: Vec<Blend>,
    pub topology: Topology,
    pub num_vertices: u32,
    /// `[0, 0]` = tamanho do alvo.
    pub viewport: [u32; 2],
}

impl Manifest {
    /// Lê o texto do `manifest.json`. Não valida o conteúdo (ver [`Manifest::validate`]).
    pub fn parse(text: &str) -> Result<Self, Error> {
        serde_json::from_str(text).map_err(|e| Error::Parse(e.to_string()))
    }
}

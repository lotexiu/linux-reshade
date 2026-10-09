// fx2spv: compila um efeito ReShade FX para um bundle (manifest.json + shaders.spv).
//
// Usa o front-end `reshadefx` do ReShade (BSD-3-Clause) exatamente como o runtime do ReShade o
// usa no Vulkan: mesmas macros BUFFER_*, mesmo gerador de SPIR-V. É uma ferramenta SEPARADA: nada
// disto é ligado à layer, que só lê o bundle. O formato está em docs/bundle-format.md.

#include "effect_codegen.hpp"
#include "effect_module.hpp"
#include "effect_parser.hpp"
#include "effect_preprocessor.hpp"

#include <cmath>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <memory>
#include <sstream>
#include <string>
#include <vector>

#ifndef RESHADE_COMMIT
#define RESHADE_COMMIT "desconhecido"
#endif
#ifndef SPIRV_HEADERS_COMMIT
#define SPIRV_HEADERS_COMMIT "desconhecido"
#endif

namespace fs = std::filesystem;

// ---------------------------------------------------------------------------------------------
// Escritor de JSON mínimo (sem dependências). Cuida das vírgulas e do escape de strings.
// ---------------------------------------------------------------------------------------------
class Json
{
public:
	Json &object() { value_prefix(); out_ << '{'; stack_.push_back({ '}', true }); return *this; }
	Json &array() { value_prefix(); out_ << '['; stack_.push_back({ ']', true }); return *this; }
	Json &end()
	{
		out_ << stack_.back().close;
		stack_.pop_back();
		return *this;
	}
	Json &key(const std::string &name)
	{
		comma();
		string_literal(name);
		out_ << ':';
		after_key_ = true;
		return *this;
	}
	Json &str(const std::string &s) { value_prefix(); string_literal(s); return *this; }
	Json &boolean(bool b) { value_prefix(); out_ << (b ? "true" : "false"); return *this; }
	Json &null() { value_prefix(); out_ << "null"; return *this; }
	Json &num(double v)
	{
		value_prefix();
		if (!std::isfinite(v)) { out_ << "null"; return *this; }
		char buf[64];
		std::snprintf(buf, sizeof buf, "%.9g", v);
		out_ << buf;
		return *this;
	}
	Json &integer(long long v) { value_prefix(); out_ << v; return *this; }

	template <typename T> Json &field(const std::string &name, const T &v);
	std::string text() const { return out_.str(); }

private:
	struct Level { char close; bool first; };
	std::ostringstream out_;
	std::vector<Level> stack_;
	bool after_key_ = false;

	void comma()
	{
		if (!stack_.empty() && !stack_.back().first) out_ << ',';
		if (!stack_.empty()) stack_.back().first = false;
	}
	void value_prefix()
	{
		if (after_key_) { after_key_ = false; return; }
		comma();
	}
	void string_literal(const std::string &s)
	{
		out_ << '"';
		for (unsigned char c : s)
		{
			switch (c)
			{
			case '"': out_ << "\\\""; break;
			case '\\': out_ << "\\\\"; break;
			case '\n': out_ << "\\n"; break;
			case '\r': out_ << "\\r"; break;
			case '\t': out_ << "\\t"; break;
			default:
				if (c < 0x20) { char b[8]; std::snprintf(b, sizeof b, "\\u%04x", c); out_ << b; }
				else out_ << static_cast<char>(c);
			}
		}
		out_ << '"';
	}
};

template <> Json &Json::field(const std::string &n, const std::string &v) { return key(n).str(v); }
template <> Json &Json::field(const std::string &n, const bool &v) { return key(n).boolean(v); }
template <> Json &Json::field(const std::string &n, const uint32_t &v) { return key(n).integer(v); }
template <> Json &Json::field(const std::string &n, const int &v) { return key(n).integer(v); }
template <> Json &Json::field(const std::string &n, const float &v) { return key(n).num(v); }
template <> Json &Json::field(const std::string &n, const size_t &v) { return key(n).integer(static_cast<long long>(v)); }

// ---------------------------------------------------------------------------------------------
// Conversões do modelo do reshadefx para os nomes do formato do bundle.
// ---------------------------------------------------------------------------------------------
using namespace reshadefx;

static const char *base_name(type::datatype b)
{
	switch (b)
	{
	case type::t_bool: return "bool";
	case type::t_int: case type::t_min16int: return "int";
	case type::t_uint: case type::t_min16uint: return "uint";
	case type::t_float: case type::t_min16float: return "float";
	case type::t_string: return "string";
	default: return "other";
	}
}

static const char *texture_format_name(texture_format f)
{
	switch (f)
	{
	case texture_format::r8: return "r8";
	case texture_format::r16f: return "r16f";
	case texture_format::r16: return "r16";
	case texture_format::r32f: return "r32f";
	case texture_format::r32u: return "r32u";
	case texture_format::r32i: return "r32i";
	case texture_format::rg8: return "rg8";
	case texture_format::rg16f: return "rg16f";
	case texture_format::rg16: return "rg16";
	case texture_format::rg32f: return "rg32f";
	case texture_format::rgba8: return "rgba8";
	case texture_format::rgba16f: return "rgba16f";
	case texture_format::rgba16: return "rgba16";
	case texture_format::rgba32f: return "rgba32f";
	case texture_format::rgba32u: return "rgba32u";
	case texture_format::rgba32i: return "rgba32i";
	case texture_format::rgb10a2: return "rgb10a2";
	case texture_format::rg11b10f: return "rg11b10f";
	default: return "unknown";
	}
}

static const char *address_name(texture_address_mode m)
{
	switch (m)
	{
	case texture_address_mode::wrap: return "wrap";
	case texture_address_mode::mirror: return "mirror";
	case texture_address_mode::border: return "border";
	default: return "clamp";
	}
}

static const char *blend_factor_name(blend_factor f)
{
	switch (f)
	{
	case blend_factor::zero: return "zero";
	case blend_factor::one: return "one";
	case blend_factor::source_color: return "source_color";
	case blend_factor::one_minus_source_color: return "one_minus_source_color";
	case blend_factor::dest_color: return "dest_color";
	case blend_factor::one_minus_dest_color: return "one_minus_dest_color";
	case blend_factor::source_alpha: return "source_alpha";
	case blend_factor::one_minus_source_alpha: return "one_minus_source_alpha";
	case blend_factor::dest_alpha: return "dest_alpha";
	default: return "one_minus_dest_alpha";
	}
}

static const char *blend_op_name(blend_op o)
{
	switch (o)
	{
	case blend_op::subtract: return "subtract";
	case blend_op::reverse_subtract: return "reverse_subtract";
	case blend_op::min: return "min";
	case blend_op::max: return "max";
	default: return "add";
	}
}

static const char *topology_name(primitive_topology t)
{
	switch (t)
	{
	case primitive_topology::point_list: return "point_list";
	case primitive_topology::line_list: return "line_list";
	case primitive_topology::line_strip: return "line_strip";
	case primitive_topology::triangle_strip: return "triangle_strip";
	default: return "triangle_list";
	}
}

// Escreve os `rows*cols` componentes de uma constante numérica (escalar se for só um).
static void write_constant(Json &j, const type &t, const constant &c)
{
	const unsigned n = t.rows * t.cols;
	if (t.base == type::t_string) { j.str(c.string_data); return; }
	auto one = [&](unsigned i) {
		if (t.is_boolean()) j.boolean(c.as_uint[i] != 0);
		else if (t.is_floating_point()) j.num(c.as_float[i]);
		else if (t.is_unsigned()) j.integer(c.as_uint[i]);
		else j.integer(c.as_int[i]);
	};
	if (n <= 1) { one(0); return; }
	j.array();
	for (unsigned i = 0; i < n && i < 16; ++i) one(i);
	j.end();
}

static void write_annotations(Json &j, const std::vector<annotation> &list)
{
	j.key("annotations").object();
	for (const annotation &a : list) { j.key(a.name); write_constant(j, a.type, a.value); }
	j.end();
}

static void write_type(Json &j, const type &t)
{
	j.key("type").object();
	j.field("base", std::string(base_name(t.base)));
	j.field("rows", static_cast<uint32_t>(t.rows));
	j.field("cols", static_cast<uint32_t>(t.cols));
	j.field("array", static_cast<uint32_t>(t.array_length == 0xFFFFFFFF ? 0 : t.array_length));
	j.end();
}

// ---------------------------------------------------------------------------------------------
// Linha de comando
// ---------------------------------------------------------------------------------------------
struct Options
{
	fs::path input;
	fs::path out_dir;
	std::vector<fs::path> include_paths;
	std::vector<std::pair<std::string, std::string>> defines;
	uint32_t width = 1920, height = 1080;
	// O gerador do ReShade força SPIR-V 1.3 no cabeçalho, mas o conteúdo é compatível com 1.0
	// (conferido com spirv-val). 1.0 funciona em dispositivos Vulkan 1.0; 1.3 exige Vulkan 1.1+.
	uint32_t spirv_version_word = 0x10000;
	bool debug_info = false;
	// O clip space do D3D (que os efeitos FX usam) tem Y para cima; o do Vulkan tem Y para baixo.
	// O ReShade corrige no runtime com viewport de altura negativa (exige VK_KHR_maintenance1); aqui
	// o vertex shader já sai com Y invertido, o que funciona em qualquer dispositivo Vulkan 1.0.
	bool flip_y = true;
};

static void usage()
{
	std::cerr << "uso: fx2spv [-I dir]... [-D NOME[=VALOR]]... [--width W] [--height H] [--spirv-version 1.0|1.3] [--no-flip-y] [--debug] -o diretorio arquivo.fx\n";
}

static bool parse_args(int argc, char **argv, Options &o)
{
	for (int i = 1; i < argc; ++i)
	{
		const std::string a = argv[i];
		auto next = [&](std::string &dst) { if (i + 1 >= argc) return false; dst = argv[++i]; return true; };
		std::string v;
		if (a == "-o") { if (!next(v)) return false; o.out_dir = v; }
		else if (a == "-I") { if (!next(v)) return false; o.include_paths.emplace_back(v); }
		else if (a == "-D")
		{
			if (!next(v)) return false;
			const size_t eq = v.find('=');
			o.defines.emplace_back(v.substr(0, eq), eq == std::string::npos ? "1" : v.substr(eq + 1));
		}
		else if (a == "--spirv-version")
		{
			if (!next(v)) return false;
			if (v == "1.0") o.spirv_version_word = 0x10000;
			else if (v == "1.3") o.spirv_version_word = 0x10300;
			else return false;
		}
		else if (a == "--debug") o.debug_info = true;
		else if (a == "--no-flip-y") o.flip_y = false;
		else if (a == "--width") { if (!next(v)) return false; o.width = static_cast<uint32_t>(std::stoul(v)); }
		else if (a == "--height") { if (!next(v)) return false; o.height = static_cast<uint32_t>(std::stoul(v)); }
		else if (!a.empty() && a[0] == '-') return false;
		else o.input = a;
	}
	return !o.input.empty() && !o.out_dir.empty() && o.width > 0 && o.height > 0;
}

int main(int argc, char **argv)
{
	Options opt;
	if (!parse_args(argc, argv, opt)) { usage(); return 2; }

	// 1. Pré-processador, com as mesmas macros que o ReShade define no Vulkan.
	preprocessor pp;
	pp.add_macro_definition("__RESHADE__", "60000");
	pp.add_macro_definition("__RESHADE_PERMUTATION__", "0");
	pp.add_macro_definition("__RESHADE_PERFORMANCE_MODE__", "0");
	pp.add_macro_definition("__VENDOR__", "0");
	pp.add_macro_definition("__DEVICE__", "0");
	pp.add_macro_definition("__RENDERER__", "131072"); // 0x20000: Vulkan
	pp.add_macro_definition("__APPLICATION__", "0");
	pp.add_macro_definition("BUFFER_WIDTH", std::to_string(opt.width));
	pp.add_macro_definition("BUFFER_HEIGHT", std::to_string(opt.height));
	pp.add_macro_definition("BUFFER_RCP_WIDTH", "(1.0 / BUFFER_WIDTH)");
	pp.add_macro_definition("BUFFER_RCP_HEIGHT", "(1.0 / BUFFER_HEIGHT)");
	pp.add_macro_definition("BUFFER_COLOR_SPACE", "1"); // sRGB não linear
	pp.add_macro_definition("BUFFER_COLOR_FORMAT", "0");
	pp.add_macro_definition("BUFFER_COLOR_BIT_DEPTH", "8");
	for (const auto &d : opt.defines) pp.add_macro_definition(d.first, d.second);
	for (const fs::path &p : opt.include_paths) pp.add_include_path(p);
	// Compatibilidade com versões antigas do ReShade (igual ao runtime dele).
	pp.append_string(
		"#define tex2Doffset(s, coords, offset) tex2D(s, coords, offset)\n"
		"#define tex2Dlodoffset(s, coords, offset) tex2Dlod(s, coords, offset)\n"
		"#define tex2Dgather(s, t, c) tex2Dgather##c(s, t)\n"
		"#define tex2Dgatheroffset(s, t, o, c) tex2Dgather##c(s, t, o)\n"
		"#define tex2Dgather0 tex2DgatherR\n#define tex2Dgather1 tex2DgatherG\n"
		"#define tex2Dgather2 tex2DgatherB\n#define tex2Dgather3 tex2DgatherA\n");

	const bool preprocessed = pp.append_file(opt.input);
	if (!pp.errors().empty()) std::cerr << pp.errors();
	if (!preprocessed) return 1;

	// 2. Parser + gerador de SPIR-V (semântica Vulkan, sem uniforms como spec constants).
	std::unique_ptr<codegen> backend(create_codegen_spirv(true, opt.debug_info, false, false, opt.flip_y));
	parser p;
	const bool compiled = p.parse(pp.output(), backend.get());
	if (!p.errors().empty()) std::cerr << p.errors();
	if (!compiled) return 1;

	const effect_module &m = backend->module();

	// O gerador de SPIR-V para Vulkan produz um módulo por entry point (com os bindings dos samplers
	// renumerados para aquele entry point). É assim que o runtime do ReShade também os usa.
	struct Entry { std::string name, stage, file, spirv; };
	std::vector<Entry> entries;
	for (const auto &ep : m.entry_points)
	{
		Entry e;
		e.name = ep.first;
		e.stage = ep.second == shader_type::vertex ? "vertex" : ep.second == shader_type::pixel ? "pixel" : "compute";
		e.file = ep.first + ".spv";
		std::string assembly, errors;
		if (!backend->assemble_code_for_entry_point(ep.first, e.spirv, assembly, errors))
		{
			std::cerr << "erro: não consegui gerar o SPIR-V de '" << ep.first << "': " << errors << "\n";
			return 1;
		}
		// A palavra 1 do cabeçalho é a versão do SPIR-V (little endian).
		if (e.spirv.size() >= 8)
			for (int b = 0; b < 4; ++b) e.spirv[4 + b] = static_cast<char>((opt.spirv_version_word >> (8 * b)) & 0xFF);
		entries.push_back(std::move(e));
	}
	size_t spirv_bytes = 0;
	for (const Entry &e : entries) spirv_bytes += e.spirv.size();

	// Recusa o que a v1 do formato não suporta, em vez de produzir um bundle que falharia depois.
	if (!m.storages.empty()) { std::cerr << "erro: imagens de armazenamento (storage) não são suportadas na v1\n"; return 1; }
	for (const technique &t : m.techniques)
		for (const pass &ps : t.passes)
			if (!ps.cs_entry_point.empty()) { std::cerr << "erro: passes de compute não são suportados na v1\n"; return 1; }

	// 3. Manifesto.
	Json j;
	j.object();
	j.field("format", std::string("linux-reshade-bundle"));
	j.field("version", 1);
	j.field("source", opt.input.filename().string());
	j.key("compiler").object();
	j.field("name", std::string("reshadefx"));
	j.field("reshade_commit", std::string(RESHADE_COMMIT));
	j.field("spirv_headers_commit", std::string(SPIRV_HEADERS_COMMIT));
	j.end();
	j.key("target").object();
	j.field("width", opt.width);
	j.field("height", opt.height);
	j.field("color_space", std::string("srgb"));
	j.end();
	j.key("defines").object();
	for (const auto &d : opt.defines) j.field(d.first, d.second);
	j.end();
	j.field("y_flipped", opt.flip_y);
	j.field("spirv_version", std::string(opt.spirv_version_word == 0x10300 ? "1.3" : "1.0"));
	j.key("entry_points").array();
	for (const Entry &e : entries)
	{
		j.object();
		j.field("name", e.name);
		j.field("stage", e.stage);
		j.field("file", e.file);
		j.end();
	}
	j.end();
	j.field("total_uniform_size", m.total_uniform_size);

	j.key("textures").array();
	for (const texture &t : m.textures)
	{
		j.object();
		j.field("name", t.name);
		j.field("unique_name", t.unique_name);
		if (t.semantic.empty()) j.key("semantic").null(); else j.field("semantic", t.semantic);
		j.field("width", t.width);
		j.field("height", t.height);
		j.field("depth", static_cast<uint32_t>(t.depth));
		j.field("levels", static_cast<uint32_t>(t.levels));
		j.field("type", std::string(t.type == texture_type::texture_1d ? "1d" : t.type == texture_type::texture_3d ? "3d" : "2d"));
		j.field("format", std::string(texture_format_name(t.format)));
		j.field("render_target", t.render_target);
		write_annotations(j, t.annotations);
		j.end();
	}
	j.end();

	j.key("samplers").array();
	for (const sampler &s : m.samplers)
	{
		const unsigned f = static_cast<unsigned>(s.filter);
		const bool aniso = s.filter == filter_mode::anisotropic;
		j.object();
		j.field("name", s.name);
		j.field("unique_name", s.unique_name);
		j.field("texture", s.texture_name);
		j.field("min_filter", std::string(aniso || (f & 0x10) ? "linear" : "point"));
		j.field("mag_filter", std::string(aniso || (f & 0x04) ? "linear" : "point"));
		j.field("mip_filter", std::string(aniso || (f & 0x01) ? "linear" : "point"));
		j.field("anisotropic", aniso);
		j.field("address_u", std::string(address_name(s.address_u)));
		j.field("address_v", std::string(address_name(s.address_v)));
		j.field("address_w", std::string(address_name(s.address_w)));
		j.field("min_lod", s.min_lod);
		j.field("max_lod", s.max_lod);
		j.field("lod_bias", s.lod_bias);
		j.field("srgb", s.srgb);
		j.end();
	}
	j.end();

	j.key("uniforms").array();
	for (const uniform &u : m.uniforms)
	{
		j.object();
		j.field("name", u.name);
		j.field("unique_name", u.unique_name);
		write_type(j, u.type);
		j.field("offset", u.offset);
		j.field("size", u.size);
		j.key("default");
		if (u.has_initializer_value) write_constant(j, u.type, u.initializer_value); else j.null();
		write_annotations(j, u.annotations);
		j.end();
	}
	j.end();

	j.key("techniques").array();
	for (const technique &t : m.techniques)
	{
		j.object();
		j.field("name", t.name);
		write_annotations(j, t.annotations);
		j.key("passes").array();
		for (const pass &ps : t.passes)
		{
			j.object();
			j.field("name", ps.name);
			j.field("vertex_entry", ps.vs_entry_point);
			j.field("pixel_entry", ps.ps_entry_point);
			j.key("compute_entry").null();
			// Anexos de saída: até o último usado; item vazio significa "o frame do jogo".
			size_t rt_count = 0;
			for (size_t k = 0; k < 8; ++k) if (!ps.render_target_names[k].empty()) rt_count = k + 1;
			j.key("render_targets").array();
			for (size_t k = 0; k < rt_count; ++k)
			{
				if (ps.render_target_names[k].empty()) j.null(); else j.str(ps.render_target_names[k]);
			}
			j.end();
			j.field("clear", ps.clear_render_targets);
			j.field("srgb_write", ps.srgb_write_enable);
			j.key("blend").array();
			for (size_t k = 0; k < (rt_count ? rt_count : 1); ++k)
			{
				j.object();
				j.field("enable", ps.blend_enable[k]);
				j.field("src_color", std::string(blend_factor_name(ps.source_color_blend_factor[k])));
				j.field("dst_color", std::string(blend_factor_name(ps.dest_color_blend_factor[k])));
				j.field("color_op", std::string(blend_op_name(ps.color_blend_op[k])));
				j.field("src_alpha", std::string(blend_factor_name(ps.source_alpha_blend_factor[k])));
				j.field("dst_alpha", std::string(blend_factor_name(ps.dest_alpha_blend_factor[k])));
				j.field("alpha_op", std::string(blend_op_name(ps.alpha_blend_op[k])));
				j.field("write_mask", static_cast<uint32_t>(ps.render_target_write_mask[k]));
				j.end();
			}
			j.end();
			// Samplers usados pelo passe e o binding de cada um (set 1), iguais nos dois estágios.
			j.key("samplers").array();
			for (const sampler_binding &sb : ps.sampler_bindings)
			{
				j.object();
				j.field("sampler", sb.index);
				j.field("binding", sb.entry_point_binding);
				j.end();
			}
			j.end();
			j.field("topology", std::string(topology_name(ps.topology)));
			j.field("num_vertices", ps.num_vertices);
			j.key("viewport").array().integer(ps.viewport_width).integer(ps.viewport_height).end();
			j.end();
		}
		j.end(); // passes
		j.end(); // technique
	}
	j.end();
	j.end(); // raiz

	// 4. Escrita atômica: grava em um diretório temporário vizinho e renomeia, para a layer (ou o
	// recarregamento a quente) nunca ver um bundle pela metade.
	std::error_code ec;
	const fs::path tmp = opt.out_dir.parent_path() / (opt.out_dir.filename().string() + ".tmp");
	fs::remove_all(tmp, ec);
	fs::create_directories(tmp, ec);
	if (ec) { std::cerr << "erro: não consegui criar " << tmp << ": " << ec.message() << "\n"; return 1; }
	{
		for (const Entry &e : entries)
			std::ofstream(tmp / e.file, std::ios::binary).write(e.spirv.data(), static_cast<std::streamsize>(e.spirv.size()));
		std::ofstream(tmp / "manifest.json") << j.text() << "\n";
	}
	fs::remove_all(opt.out_dir, ec);
	fs::rename(tmp, opt.out_dir, ec);
	if (ec) { std::cerr << "erro: não consegui publicar " << opt.out_dir << ": " << ec.message() << "\n"; return 1; }
	std::cerr << "ok: " << opt.out_dir.string() << " (" << m.techniques.size() << " técnica(s), "
	          << m.uniforms.size() << " uniform(s), " << entries.size() << " entry point(s), " << spirv_bytes << " bytes de SPIR-V)\n";
	return 0;
}

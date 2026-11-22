// Correção de cor por LUT 3D, escrita para este projeto (não depende do ReShade.fxh). A LUT é uma faixa de 16 blocos de 16x16 (256x16), como a
// `BasicLUT.png` que acompanha o programa (uma LUT neutra: não muda nada). Troque o arquivo por uma LUT sua (mesmo formato) para dar o visual.

uniform float Amount <
	ui_type = "slider";
	ui_label = "Quantidade";
	ui_tooltip = "0 = imagem original, 1 = só a LUT.";
	ui_min = 0.0; ui_max = 1.0;
> = 1.0;

#define LUT_SIZE 16.0

texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

texture LutTex < source = "BasicLUT.png"; > { Width = 256; Height = 16; Format = RGBA8; };
sampler Lut { Texture = LutTex; MagFilter = LINEAR; MinFilter = LINEAR; };

void VS_Fullscreen(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float3 ApplyLut(float3 rgb)
{
	float blue = rgb.b * (LUT_SIZE - 1.0);
	float slice_low = floor(blue);
	float slice_high = min(slice_low + 1.0, LUT_SIZE - 1.0);
	float mix_slices = blue - slice_low;
	// O centro de cada texel: meio texel para dentro, para o filtro linear não misturar blocos vizinhos.
	float2 inside = rgb.rg * ((LUT_SIZE - 1.0) / LUT_SIZE) + 0.5 / LUT_SIZE;
	float2 uv_low = float2((slice_low + inside.x) / LUT_SIZE, inside.y);
	float2 uv_high = float2((slice_high + inside.x) / LUT_SIZE, inside.y);
	return lerp(tex2D(Lut, uv_low).rgb, tex2D(Lut, uv_high).rgb, mix_slices);
}

float4 PS_Lut(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 color = tex2D(BackBuffer, texcoord);
	return float4(lerp(color.rgb, ApplyLut(saturate(color.rgb)), Amount), color.a);
}

technique BasicLUT < ui_label = "LUT (básica)"; >
{
	pass { VertexShader = VS_Fullscreen; PixelShader = PS_Lut; }
}

// Cor básica, escrita para este projeto: vibração (satura mais o que está pouco saturado), saturação, contraste e brilho.

uniform float Vibrance <
	ui_type = "slider";
	ui_label = "Vibração";
	ui_tooltip = "Satura mais as cores apagadas e preserva as que já são fortes (tons de pele).";
	ui_min = -1.0; ui_max = 1.0;
> = 0.25;

uniform float Saturation <
	ui_type = "slider";
	ui_label = "Saturação";
	ui_min = 0.0; ui_max = 2.0;
> = 1.0;

uniform float Contrast <
	ui_type = "slider";
	ui_label = "Contraste";
	ui_min = 0.5; ui_max = 1.5;
> = 1.0;

uniform float Brightness <
	ui_type = "slider";
	ui_label = "Brilho";
	ui_min = -0.25; ui_max = 0.25;
> = 0.0;

texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

void VS_Fullscreen(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS_Color(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 color = tex2D(BackBuffer, texcoord);
	float luma = dot(color.rgb, float3(0.2126, 0.7152, 0.0722));
	float3 rgb = lerp(float3(luma, luma, luma), color.rgb, Saturation);
	// Quanto mais saturado o pixel, menos a vibração mexe nele.
	float saturation_now = max(rgb.r, max(rgb.g, rgb.b)) - min(rgb.r, min(rgb.g, rgb.b));
	rgb = lerp(float3(luma, luma, luma), rgb, 1.0 + Vibrance * (1.0 - saturation_now));
	rgb = (rgb - 0.5) * Contrast + 0.5 + Brightness;
	return float4(saturate(rgb), color.a);
}

technique BasicVibrance < ui_label = "Cor (básica)"; >
{
	pass { VertexShader = VS_Fullscreen; PixelShader = PS_Color; }
}

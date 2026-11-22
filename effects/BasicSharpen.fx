// Nitidez básica, escrita para este projeto (não depende do ReShade.fxh): realça o contraste local com uma máscara de nitidez de 4 vizinhos.
// Barata (5 leituras por pixel). "Limite" evita realçar ruído e bordas já muito fortes.

uniform float Strength <
	ui_type = "slider";
	ui_label = "Intensidade";
	ui_tooltip = "0 desliga; valores altos criam halos nas bordas.";
	ui_min = 0.0; ui_max = 2.0;
> = 0.6;

uniform float Limit <
	ui_type = "slider";
	ui_label = "Limite";
	ui_tooltip = "O realce máximo por pixel (evita halos e ruído).";
	ui_min = 0.01; ui_max = 0.5;
> = 0.12;

texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

void VS_Fullscreen(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS_Sharpen(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float2 texel = float2(1.0 / BUFFER_WIDTH, 1.0 / BUFFER_HEIGHT);
	float4 center = tex2D(BackBuffer, texcoord);
	float3 blur = tex2D(BackBuffer, texcoord + float2(texel.x, 0.0)).rgb
	            + tex2D(BackBuffer, texcoord - float2(texel.x, 0.0)).rgb
	            + tex2D(BackBuffer, texcoord + float2(0.0, texel.y)).rgb
	            + tex2D(BackBuffer, texcoord - float2(0.0, texel.y)).rgb;
	float3 detail = center.rgb - blur * 0.25;
	return float4(saturate(center.rgb + clamp(detail * Strength, -Limit, Limit)), center.a);
}

technique BasicSharpen < ui_label = "Nitidez (básica)"; >
{
	pass { VertexShader = VS_Fullscreen; PixelShader = PS_Sharpen; }
}

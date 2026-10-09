// Sépia: efeito simples de teste, escrito para este projeto (não depende do ReShade.fxh).
// Mistura o frame com uma versão em tons de sépia; "Intensidade" controla a mistura.

uniform float Strength <
	ui_type = "slider";
	ui_label = "Intensidade";
	ui_min = 0.0; ui_max = 1.0;
> = 1.0;

texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

// Triângulo de tela cheia a partir do índice do vértice (o mesmo que o ReShade.fxh usa).
void VS_Fullscreen(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS_Sepia(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 color = tex2D(BackBuffer, texcoord);
	float gray = dot(color.rgb, float3(0.299, 0.587, 0.114));
	float3 sepia = gray * float3(1.2, 1.0, 0.8);
	return float4(lerp(color.rgb, sepia, Strength), color.a);
}

technique Sepia < ui_label = "Sépia"; >
{
	pass
	{
		VertexShader = VS_Fullscreen;
		PixelShader = PS_Sepia;
	}
}

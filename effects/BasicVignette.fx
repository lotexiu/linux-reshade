// Vinheta básica, escrita para este projeto: escurece as bordas da tela, com raio, suavidade e formato (círculo ou o da tela).

uniform float Strength <
	ui_type = "slider";
	ui_label = "Intensidade";
	ui_min = 0.0; ui_max = 1.0;
> = 0.45;

uniform float Radius <
	ui_type = "slider";
	ui_label = "Raio";
	ui_tooltip = "Onde a vinheta começa (1 = nos cantos).";
	ui_min = 0.2; ui_max = 1.2;
> = 0.75;

uniform float Softness <
	ui_type = "slider";
	ui_label = "Suavidade";
	ui_min = 0.05; ui_max = 1.0;
> = 0.5;

uniform bool Round <
	ui_label = "Redonda";
	ui_tooltip = "Ligado: um círculo. Desligado: acompanha a proporção da tela.";
> = true;

texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

void VS_Fullscreen(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS_Vignette(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 color = tex2D(BackBuffer, texcoord);
	float2 centered = texcoord - 0.5;
	if (Round)
		centered.x *= float(BUFFER_WIDTH) / float(BUFFER_HEIGHT);
	// 0 no centro, 1 nos cantos (a distância de um canto ao centro vale 1).
	float distance_to_corner = length(centered) / (Round ? length(float2(0.5 * float(BUFFER_WIDTH) / float(BUFFER_HEIGHT), 0.5)) : 0.7071);
	float dark = smoothstep(Radius, Radius + Softness, distance_to_corner);
	return float4(color.rgb * (1.0 - dark * Strength), color.a);
}

technique BasicVignette < ui_label = "Vinheta (básica)"; >
{
	pass { VertexShader = VS_Fullscreen; PixelShader = PS_Vignette; }
}

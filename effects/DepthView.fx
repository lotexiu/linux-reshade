// Mostra a profundidade do jogo em cinza (mais perto = mais escuro, na escala da profundidade bruta). Efeito de teste
// do runtime de profundidade: sem profundidade disponível o linux-reshade o pula com o motivo.
texture DepthTex : DEPTH;
sampler Depth { Texture = DepthTex; };

uniform float Gain < ui_type = "slider"; ui_min = 0.1; ui_max = 8.0; ui_label = "Ganho"; > = 1.0;

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float d = saturate(tex2D(Depth, texcoord).r * Gain);
	return float4(d, d, d, 1.0);
}

technique DepthView < ui_label = "Ver profundidade"; > { pass { VertexShader = VS; PixelShader = PS; } }

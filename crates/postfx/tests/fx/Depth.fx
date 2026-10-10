// Amostra a profundidade do jogo (semântica DEPTH) e a mostra em cinza.
texture DepthTex : DEPTH;
sampler Depth { Texture = DepthTex; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float d = tex2D(Depth, texcoord).r;
	return float4(d, d, d, 1.0);
}

technique Depth { pass { VertexShader = VS; PixelShader = PS; } }

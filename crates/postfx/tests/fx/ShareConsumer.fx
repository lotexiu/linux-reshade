// Consome a textura que outro efeito produz (mesmo nome e propriedades): só a declara e a amostra.
texture SharedTex { Width = 8; Height = 4; Format = RGBA8; };
sampler SharedS { Texture = SharedTex; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS_Use(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return tex2D(SharedS, texcoord);
}

technique ShareConsumer
{
	pass { VertexShader = VS; PixelShader = PS_Use; }
}

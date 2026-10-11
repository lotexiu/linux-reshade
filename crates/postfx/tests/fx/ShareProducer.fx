// Produz uma textura compartilhada: desenha uma cor fixa em SharedTex (como o LAUNCHPAD faz com as normais).
texture SharedTex { Width = 8; Height = 4; Format = RGBA8; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS_Make(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return float4(0.25, 0.5, 0.75, 1.0);
}

technique ShareProducer
{
	pass { VertexShader = VS; PixelShader = PS_Make; RenderTarget = SharedTex; }
}

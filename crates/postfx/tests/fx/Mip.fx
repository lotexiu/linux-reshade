// Mipmaps: o primeiro passe copia o frame para MipTex (4 níveis); o segundo lê o nível 3 (1x1, a média do
// frame) e devolve essa cor com alfa 1. Exercita a geração de mipmaps depois do passe que escreve a textura.
texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };
texture MipTex { Width = 8; Height = 4; Format = RGBA8; MipLevels = 4; };
sampler Mip { Texture = MipTex; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS_Copy(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return tex2D(BackBuffer, texcoord);
}

float4 PS_Average(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return float4(tex2Dlod(Mip, float4(0.5, 0.5, 0.0, 3.0)).rgb, 1.0);
}

technique Mip
{
	pass { VertexShader = VS; PixelShader = PS_Copy; RenderTarget = MipTex; }
	pass { VertexShader = VS; PixelShader = PS_Average; }
}

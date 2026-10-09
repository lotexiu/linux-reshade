// Dois passes com uma textura intermediária: o primeiro escreve metade do frame em HalfTex, o
// segundo lê HalfTex e soma 0,25 (alfa 1). Exercita render target, amostragem e ordem dos passes.
texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };
texture HalfTex { Width = 8; Height = 4; Format = RGBA8; };
sampler Half { Texture = HalfTex; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS_Half(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return tex2D(BackBuffer, texcoord) * 0.5;
}

float4 PS_Add(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return float4(tex2D(Half, texcoord).rgb + 0.25, 1.0);
}

technique TwoPass
{
	pass { VertexShader = VS; PixelShader = PS_Half; RenderTarget = HalfTex; }
	pass { VertexShader = VS; PixelShader = PS_Add; }
}

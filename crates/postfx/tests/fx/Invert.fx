// Inverte as cores. Efeito de teste do postfx (o equivalente FX do antigo InvertPass).
texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 color = tex2D(BackBuffer, texcoord);
	return float4(1.0 - color.rgb, color.a);
}

technique Invert { pass { VertexShader = VS; PixelShader = PS; } }

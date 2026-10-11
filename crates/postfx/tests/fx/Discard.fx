// Só escreve na metade direita; a esquerda é `discard` e precisa continuar com o quadro do jogo (como o SMAA do iMMERSE, que só
// escreve onde há borda). Efeito de teste do postfx.
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
	if (texcoord.x < 0.5)
		discard;
	return float4(1.0, 0.0, 0.0, 1.0);
}

technique Discard { pass { VertexShader = VS; PixelShader = PS; SRGBWriteEnable = true; } }

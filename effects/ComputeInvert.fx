// Inverte as cores com um compute shader: lê o frame (sampler), grava o inverso numa imagem de armazenamento e um
// passe de desenho a mostra. Efeito de teste do runtime de compute (smoke-compute); o resultado é o mesmo do Invert.
texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };
texture InvertedTex { Width = BUFFER_WIDTH; Height = BUFFER_HEIGHT; Format = RGBA8; };
sampler Inverted { Texture = InvertedTex; };
storage2D InvertedStore { Texture = InvertedTex; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

void CS_Invert(uint3 id : SV_DispatchThreadID)
{
	if (id.x < BUFFER_WIDTH && id.y < BUFFER_HEIGHT)
	{
		float4 c = tex2Dfetch(BackBuffer, int2(id.xy));
		tex2Dstore(InvertedStore, int2(id.xy), float4(1.0 - c.rgb, c.a));
	}
}

float4 PS_Show(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return tex2D(Inverted, texcoord);
}

technique ComputeInvert
{
	pass { ComputeShader = CS_Invert<8, 8>; DispatchSizeX = (BUFFER_WIDTH + 7) / 8; DispatchSizeY = (BUFFER_HEIGHT + 7) / 8; }
	pass { VertexShader = VS; PixelShader = PS_Show; }
}

// Compute: o primeiro passe lê o frame (sampler, set 1) e grava o inverso numa imagem de armazenamento (set 2);
// o segundo desenha essa textura no frame. Exercita o despacho, a barreira de layout e o uso da textura depois.
texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };
texture Inverted { Width = 8; Height = 4; Format = RGBA8; };
sampler InvertedS { Texture = Inverted; };
storage2D InvertedW { Texture = Inverted; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

void CS_Invert(uint3 id : SV_DispatchThreadID)
{
	if (id.x < 8 && id.y < 4)
	{
		float4 c = tex2Dfetch(BackBuffer, int2(id.xy));
		tex2Dstore(InvertedW, int2(id.xy), float4(1.0 - c.rgb, c.a));
	}
}

float4 PS_Show(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return tex2D(InvertedS, texcoord);
}

technique Compute
{
	pass { ComputeShader = CS_Invert<8, 4>; DispatchSizeX = 1; DispatchSizeY = 1; }
	pass { VertexShader = VS; PixelShader = PS_Show; }
}

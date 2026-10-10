// Textura que vem de arquivo: devolve a imagem (4x2, amostrada sem filtro) no lugar do frame.
texture Img < source = "img.png"; > { Width = 4; Height = 2; Format = RGBA8; };
sampler Image { Texture = Img; MagFilter = POINT; MinFilter = POINT; MipFilter = POINT; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return tex2D(Image, texcoord);
}

technique Tex { pass { VertexShader = VS; PixelShader = PS; } }

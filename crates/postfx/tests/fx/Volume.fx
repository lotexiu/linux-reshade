// Texturas 3D e 1D de armazenamento: um compute escreve um volume 2x2x2 (R = x/1, G = y/1, B = z/1 da fatia) e uma
// linha 1D de 4 texels; o passe de desenho amostra os dois e soma, para o teste conferir o resultado.
texture3D Volume { Width = 2; Height = 2; Depth = 2; Format = RGBA8; };
sampler3D VolumeS { Texture = Volume; MagFilter = POINT; MinFilter = POINT; };
storage3D VolumeW { Texture = Volume; };
texture1D Line { Width = 4; Format = RGBA8; };
sampler1D LineS { Texture = Line; MagFilter = POINT; MinFilter = POINT; };
storage1D LineW { Texture = Line; };

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

void CS_Fill(uint3 id : SV_DispatchThreadID)
{
	if (id.x < 2 && id.y < 2 && id.z < 2)
		tex3Dstore(VolumeW, int3(id), float4(id.x, id.y, id.z, 1.0));
	if (id.y == 0 && id.z == 0 && id.x < 4)
		tex1Dstore(LineW, int(id.x), float4(0.0, 0.0, 0.25 * id.x, 1.0));
}

float4 PS_Show(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	// Pontos fixos: fatia 1 do volume (z = 0.75), texel 2 da linha (u = 0.625).
	float3 v = tex3D(VolumeS, float3(0.75, 0.25, 0.75)).rgb;
	float3 l = tex1D(LineS, 0.625).rgb;
	return float4(v.x, v.y + v.z * 0.5, l.z, 1.0);
}

technique Volume
{
	pass { ComputeShader = CS_Fill<4, 2, 2>; DispatchSizeX = 1; DispatchSizeY = 1; DispatchSizeZ = 1; }
	pass { VertexShader = VS; PixelShader = PS_Show; }
}

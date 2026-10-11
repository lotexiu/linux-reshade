// Predicação como a do Marty (IPC): um passe "pede" escrevendo 1 num canal de uma textura 1x1 com um ponto e uma máscara
// de escrita; o passe seguinte, no quadro SEGUINTE, lê o pedido. Quadro 1: nada pedido (frame intacto); quadro 2: pedido.
texture Request { Format = RGBA8; };
sampler RequestS { Texture = Request; };

float4 PointVS(in uint id : SV_VertexID) : SV_Position { return float4(0, 0, 0, 1); }
float4 SetPS(in float4 vpos : SV_Position) : SV_Target0 { return 1; }
float4 ClearPS(in float4 vpos : SV_Position) : SV_Target0 { return 0; }

void FullVS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

texture BackBufferTex : COLOR;
sampler BackBuffer { Texture = BackBufferTex; };

float4 UsePS(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	float4 c = tex2D(BackBuffer, texcoord);
	// Canais R (1) e B (4) pedidos => pinta de azul; G (2) não pedido.
	float4 r = tex2Dlod(RequestS, float4(0.5, 0.5, 0, 0));
	return (r.r > 0.5 && r.b > 0.5 && r.g < 0.5) ? float4(0, 0, 1, 1) : c;
}

technique Ipc
{
	pass Use { VertexShader = FullVS; PixelShader = UsePS; }
	pass Clear { PrimitiveTopology = POINTLIST; VertexCount = 1; VertexShader = PointVS; PixelShader = ClearPS; RenderTarget = Request; }
	pass Ask { PrimitiveTopology = POINTLIST; VertexCount = 1; VertexShader = PointVS; PixelShader = SetPS; RenderTarget = Request; RenderTargetWriteMask = 5; }
}

// Como o Ipc.fx, mas a leitura do pedido é no VERTEX shader (como o LAUNCHPAD): sem pedido o triângulo sai da tela e o
// frame fica intacto; com o pedido ele pinta de verde. Usa a mesma função `is_requested` do Marty.
texture Request { Format = RGBA8; };
sampler RequestS { Texture = Request; };

bool is_requested(uint mask)
{
	return dot(tex2Dlod(RequestS, 0.0.xxxx), mask & uint4(1,2,4,8)) > 0.5;
}

float4 PointVS(in uint id : SV_VertexID) : SV_Position { return float4(0, 0, 0, 1); }
float4 SetPS(in float4 vpos : SV_Position) : SV_Target0 { return 1; }
float4 ClearPS(in float4 vpos : SV_Position) : SV_Target0 { return 0; }

void GatedVS(uint id : SV_VertexID, out float4 position : SV_Position)
{
	float2 texcoord;
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
	if (!is_requested(1)) position.xy = -100000;
}

float4 GreenPS(float4 position : SV_Position) : SV_Target { return float4(0, 1, 0, 1); }

technique Ipc2
{
	pass Use { VertexShader = GatedVS; PixelShader = GreenPS; }
	pass Clear { PrimitiveTopology = POINTLIST; VertexCount = 1; VertexShader = PointVS; PixelShader = ClearPS; RenderTarget = Request; }
	pass Ask { PrimitiveTopology = POINTLIST; VertexCount = 1; VertexShader = PointVS; PixelShader = SetPS; RenderTarget = Request; RenderTargetWriteMask = 1; }
}

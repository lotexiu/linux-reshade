// Pinta o frame inteiro com um valor derivado dos uniforms que o runtime fornece:
// vermelho = timer/1000 (segundos, limitado a 1), verde = framecount/255, azul = frametime/255.
uniform float timer < source = "timer"; >;
uniform float frametime < source = "frametime"; >;
uniform int framecount < source = "framecount"; >;
uniform float mouse < source = "mousepoint"; >; // não suportado: tem de ser listado como ignorado

void VS(uint id : SV_VertexID, out float4 position : SV_Position, out float2 texcoord : TEXCOORD0)
{
	texcoord.x = (id == 2) ? 2.0 : 0.0;
	texcoord.y = (id == 1) ? 2.0 : 0.0;
	position = float4(texcoord * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PS(float4 position : SV_Position, float2 texcoord : TEXCOORD0) : SV_Target
{
	return float4(saturate(timer / 1000.0), saturate(framecount / 255.0), saturate(frametime / 255.0), 1.0 + mouse);
}

technique Timer { pass { VertexShader = VS; PixelShader = PS; } }

// linux-reshade DIY: provedor de normais e profundidade linear em meia resolução. Coloque ANTES dos efeitos LR_*.
// Substitui o trabalho que cada efeito de luz repete (pirâmide de profundidade, normais), feito uma vez por quadro.
#include "LR_Common.fxh"

float LinearZ(float2 uv)
{
    return ReShade::GetLinearizedDepth(uv) * RESHADE_DEPTH_LINEARIZATION_FAR_PLANE;
}

float4 PS_Depth(float4 vpos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    float2 px = BUFFER_PIXEL_SIZE;
    float zc = LinearZ(uv);
    // Diferenças dos dois lados; vale o lado mais perto em profundidade (evita o degrau nas bordas dos objetos).
    float zl = LinearZ(uv - float2(px.x, 0)), zr = LinearZ(uv + float2(px.x, 0));
    float zu = LinearZ(uv - float2(0, px.y)), zd = LinearZ(uv + float2(0, px.y));
    float3 pc = LR_ViewPos(uv, zc);
    float3 dx = abs(zl - zc) < abs(zr - zc) ? pc - LR_ViewPos(uv - float2(px.x, 0), zl) : LR_ViewPos(uv + float2(px.x, 0), zr) - pc;
    float3 dy = abs(zu - zc) < abs(zd - zc) ? pc - LR_ViewPos(uv - float2(0, px.y), zu) : LR_ViewPos(uv + float2(0, px.y), zd) - pc;
    float3 n = normalize(cross(dy, dx));
    return float4(n, zc);
}

technique LR_Depth < ui_label = "LR: profundidade e normais (provedor)"; >
{
    pass { VertexShader = PostProcessVS; PixelShader = PS_Depth; RenderTarget = LR_NormalDepth; }
}

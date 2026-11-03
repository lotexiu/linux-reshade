// linux-reshade DIY: luz indireta difusa e oclusão ambiente em meia resolução, sobre o provedor LR_Depth.
// Uma versão leve (12 amostras em espiral, desfoque que respeita bordas, composição com ampliação consciente de profundidade)
// para cenas em que a GPU é o limite. Ordem: LR_Depth, LR_GI, depois a cor.
#include "LR_Common.fxh"

uniform float fBounce < ui_type = "slider"; ui_label = "Luz rebatida"; ui_min = 0.0; ui_max = 16.0; > = 4.0;
uniform float fAO < ui_type = "slider"; ui_label = "Oclusão ambiente"; ui_min = 0.0; ui_max = 2.0; > = 0.8;
uniform float fRadius < ui_type = "slider"; ui_label = "Raio (unidades do mundo)"; ui_min = 0.5; ui_max = 80.0; > = 24.0;
uniform float fBlurDepth < ui_type = "slider"; ui_label = "Tolerância de profundidade do desfoque"; ui_min = 0.01; ui_max = 1.0; > = 0.1;

#define LR_SAMPLES 12

texture LR_GI_A { Width = BUFFER_WIDTH / 2; Height = BUFFER_HEIGHT / 2; Format = RGBA16F; };
texture LR_GI_B { Width = BUFFER_WIDTH / 2; Height = BUFFER_HEIGHT / 2; Format = RGBA16F; };
sampler sLR_GI_A { Texture = LR_GI_A; MinFilter = POINT; MagFilter = POINT; AddressU = CLAMP; AddressV = CLAMP; };
sampler sLR_GI_B { Texture = LR_GI_B; MinFilter = POINT; MagFilter = POINT; AddressU = CLAMP; AddressV = CLAMP; };

float Ign(float2 p) { return frac(52.9829189 * frac(dot(p, float2(0.06711056, 0.00583715)))); }

// Passe 1: para cada pixel de meia resolução, junta luz que vem dos vizinhos (cor) e quanto eles bloqueiam (ao).
float4 PS_Gather(float4 vpos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    float4 nd = tex2Dlod(sLR_NormalDepth, float4(uv, 0, 0));
    float3 n = nd.xyz;
    float z = nd.w;
    if (z > RESHADE_DEPTH_LINEARIZATION_FAR_PLANE * 0.98) return float4(0, 0, 0, 0);
    float3 p = LR_ViewPos(uv, z);
    float t = tan(radians(LR_FovY) * 0.5);
    // Raio do mundo em pixels de meia resolução, limitado para não varrer a tela inteira de perto.
    float rpx = min(fRadius / (2.0 * t * z) * (BUFFER_HEIGHT * 0.5), 96.0);
    float angle0 = Ign(vpos.xy) * 6.2831853;
    float3 light = 0;
    float occ = 0;
    [unroll]
    for (int i = 0; i < LR_SAMPLES; i++)
    {
        float s = (float(i) + 0.5) / LR_SAMPLES;
        float a = angle0 + float(i) * 2.3999632;
        float2 off = float2(cos(a), sin(a)) * (s * s * rpx + 1.5);
        float2 suv = uv + off * float2(2.0 * BUFFER_RCP_WIDTH, 2.0 * BUFFER_RCP_HEIGHT);
        float4 snd = tex2Dlod(sLR_NormalDepth, float4(suv, 0, 0));
        float3 v = LR_ViewPos(suv, snd.w) - p;
        float d2 = dot(v, v) + 1e-4;
        float3 dir = v * rsqrt(d2);
        float fall = saturate(1.0 - d2 / (fRadius * fRadius));
        float cosine = saturate(dot(n, dir) - 0.05);
        occ += cosine * fall;
        float3 c = tex2Dlod(ReShade::BackBuffer, float4(suv, 0, 0)).rgb;
        light += c * cosine * saturate(dot(snd.xyz, -dir) + 0.1) * fall;
    }
    return float4(light / LR_SAMPLES, saturate(occ / LR_SAMPLES * 2.0));
}

float4 Blur(sampler src, float2 uv, float2 dir)
{
    float zc = tex2Dlod(sLR_NormalDepth, float4(uv, 0, 0)).w;
    float4 sum = tex2Dlod(src, float4(uv, 0, 0));
    float wsum = 1.0;
    [unroll]
    for (int k = -3; k <= 3; k++)
    {
        if (k == 0) continue;
        float2 suv = uv + dir * float(k);
        float zs = tex2Dlod(sLR_NormalDepth, float4(suv, 0, 0)).w;
        float w = exp(-abs(zs - zc) / (fBlurDepth * zc + 1e-3)) * exp(-float(k * k) * 0.12);
        sum += tex2Dlod(src, float4(suv, 0, 0)) * w;
        wsum += w;
    }
    return sum / wsum;
}

float4 PS_BlurH(float4 vpos : SV_Position, float2 uv : TEXCOORD) : SV_Target { return Blur(sLR_GI_A, uv, float2(2.0 * BUFFER_RCP_WIDTH, 0)); }
float4 PS_BlurV(float4 vpos : SV_Position, float2 uv : TEXCOORD) : SV_Target { return Blur(sLR_GI_B, uv, float2(0, 2.0 * BUFFER_RCP_HEIGHT)); }

// Passe final, em resolução cheia: amplia o resultado de meia resolução pesando os quatro vizinhos pela profundidade.
float4 PS_Composite(float4 vpos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    float3 color = tex2Dlod(ReShade::BackBuffer, float4(uv, 0, 0)).rgb;
    float zf = ReShade::GetLinearizedDepth(uv) * RESHADE_DEPTH_LINEARIZATION_FAR_PLANE;
    if (zf > RESHADE_DEPTH_LINEARIZATION_FAR_PLANE * 0.98) return float4(color, 1);
    float2 hs = float2(BUFFER_WIDTH, BUFFER_HEIGHT) * 0.5;
    float2 fpos = uv * hs - 0.5;
    float2 base = floor(fpos);
    float2 f = fpos - base;
    float4 acc = 0;
    float wsum = 1e-5;
    [unroll]
    for (int j = 0; j < 4; j++)
    {
        float2 o = float2(j & 1, j >> 1);
        float2 suv = (base + o + 0.5) / hs;
        float zs = tex2Dlod(sLR_NormalDepth, float4(suv, 0, 0)).w;
        float w = (o.x > 0.5 ? f.x : 1.0 - f.x) * (o.y > 0.5 ? f.y : 1.0 - f.y) * exp(-abs(zs - zf) / (fBlurDepth * zf + 1e-3));
        acc += tex2Dlod(sLR_GI_A, float4(suv, 0, 0)) * w;
        wsum += w;
    }
    acc /= wsum;
    float3 lit = color * (1.0 - saturate(acc.a * fAO)) + color * acc.rgb * fBounce;
    return float4(lit, 1);
}

technique LR_GI < ui_label = "LR: luz indireta leve"; >
{
    pass Gather { VertexShader = PostProcessVS; PixelShader = PS_Gather; RenderTarget = LR_GI_A; }
    pass BlurH { VertexShader = PostProcessVS; PixelShader = PS_BlurH; RenderTarget = LR_GI_B; }
    pass BlurV { VertexShader = PostProcessVS; PixelShader = PS_BlurV; RenderTarget = LR_GI_A; }
    pass Composite { VertexShader = PostProcessVS; PixelShader = PS_Composite; }
}

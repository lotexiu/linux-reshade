// linux-reshade DIY: declarações compartilhadas entre o provedor de profundidade (LR_Depth) e os efeitos que o usam.
// Duas declarações de textura com o mesmo nome e as mesmas propriedades são a mesma imagem na cadeia (como o Launchpad do
// Marty): LR_Depth a escreve uma vez por quadro e os demais só a leem.
#pragma once
#include "ReShade.fxh"

// Normal (xyz, espaço da câmera) e profundidade linear (w, em unidades do mundo) em meia resolução.
texture LR_NormalDepth { Width = BUFFER_WIDTH / 2; Height = BUFFER_HEIGHT / 2; Format = RGBA16F; };
sampler sLR_NormalDepth { Texture = LR_NormalDepth; MinFilter = POINT; MagFilter = POINT; MipFilter = POINT; AddressU = CLAMP; AddressV = CLAMP; };

// Ângulo de visão vertical e escala do mundo: o runtime não conhece a projeção do jogo, então são ajustes.
uniform float LR_FovY < ui_type = "slider"; ui_label = "Campo de visão vertical (graus)"; ui_min = 20.0; ui_max = 120.0; ui_tooltip = "Roblox usa 70 por padrão."; > = 70.0;

float3 LR_ViewPos(float2 uv, float z)
{
    float t = tan(radians(LR_FovY) * 0.5);
    return float3((uv.x * 2.0 - 1.0) * t * BUFFER_ASPECT_RATIO * z, (1.0 - uv.y * 2.0) * t * z, z);
}

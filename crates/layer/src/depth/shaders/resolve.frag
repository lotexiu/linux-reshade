#version 450
#extension GL_EXT_samplerless_texture_functions : require
// Resolve a profundidade de uma imagem MSAA numa imagem de uma amostra: copia a amostra 0 de cada pixel.
layout(set = 0, binding = 0) uniform texture2DMS depth_ms;
void main() {
    gl_FragDepth = texelFetch(depth_ms, ivec2(gl_FragCoord.xy), 0).r;
}

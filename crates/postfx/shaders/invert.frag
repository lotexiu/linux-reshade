#version 450
#extension GL_EXT_samplerless_texture_functions : require
// Cópia do backbuffer (texture2D, sem sampler: texelFetch). Inverte as cores e mantém o alfa.
layout(set = 0, binding = 0) uniform texture2D backbuffer;
layout(location = 0) out vec4 out_color;
void main() {
    vec4 c = texelFetch(backbuffer, ivec2(gl_FragCoord.xy), 0);
    out_color = vec4(1.0 - c.rgb, c.a);
}

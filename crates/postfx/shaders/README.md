# Shaders do postfx

Os `.spv` ficam versionados: o build não depende de compilador de shader. Depois de editar um
`.vert`/`.frag`, rode `scripts/dev.sh shaders` (usa `glslangValidator`) e commite os `.spv`.

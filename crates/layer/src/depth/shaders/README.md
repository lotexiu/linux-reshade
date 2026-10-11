Shaders do passe de resolve da profundidade MSAA (`../resolve.rs`). Os `.spv` são commitados; para refazer:

    glslangValidator -V resolve.vert -o resolve.vert.spv
    glslangValidator -V resolve.frag -o resolve.frag.spv

#version 450
// Fullscreen triangle from gl_VertexIndex: no vertex buffer and no vertex
// attributes, so the pipeline needs no vertex input state.
void main() {
    vec2 p = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

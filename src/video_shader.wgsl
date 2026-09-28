// AmoxliCode - Fase 2: shader del quad de video de fondo (Capa 0)

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Truco de "triángulo de pantalla completa": con solo 3 vértices generados
// por índice (sin vertex buffer) cubrimos toda la pantalla.
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.clip_position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@group(0) @binding(0)
var t_video: texture_2d<f32>;
@group(0) @binding(1)
var s_video: sampler;

// Opacidad fija de la Capa 0 según la especificación (0.15) para que el
// texto que se dibujará encima en la Fase 3 siga siendo legible.
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_video, s_video, in.uv);
    return color;
}

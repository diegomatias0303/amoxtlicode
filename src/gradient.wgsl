// AmoxliCode - shader del selector de color estilo "programa de dibujo".
// Modo 0: cuadro de Saturación (eje X) / Brillo (eje Y, invertido) para
// un tono (hue) fijo. Modo 1: barra de tono (arcoíris completo en X).

struct Uniforms {
    hue: f32,
    mode: f32,
    _pad: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VertexInput {
    @location(0) position: vec2<f32>, // ya en NDC
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    return out;
}

fn hsv2rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let hh = h % 360.0;
    let c = v * s;
    let x = c * (1.0 - abs(((hh / 60.0) % 2.0) - 1.0));
    let m = v - c;

    var rgb: vec3<f32>;
    if (hh < 60.0) {
        rgb = vec3<f32>(c, x, 0.0);
    } else if (hh < 120.0) {
        rgb = vec3<f32>(x, c, 0.0);
    } else if (hh < 180.0) {
        rgb = vec3<f32>(0.0, c, x);
    } else if (hh < 240.0) {
        rgb = vec3<f32>(0.0, x, c);
    } else if (hh < 300.0) {
        rgb = vec3<f32>(x, 0.0, c);
    } else {
        rgb = vec3<f32>(c, 0.0, x);
    }
    return rgb + vec3<f32>(m, m, m);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (u.mode > 0.5) {
        // Barra de tono: arcoíris completo a lo largo de X.
        let rgb = hsv2rgb(in.uv.x * 360.0, 1.0, 1.0);
        return vec4<f32>(rgb, 1.0);
    } else {
        // Cuadro saturación/brillo para el tono actual.
        let rgb = hsv2rgb(u.hue, in.uv.x, 1.0 - in.uv.y);
        return vec4<f32>(rgb, 1.0);
    }
}

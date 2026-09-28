//! AmoxliCode - conversiones de color para el selector tipo "programa de
//! dibujo" (HSV <-> RGB <-> hexadecimal).

/// Convierte tono (0-360), saturación (0-1) y valor/brillo (0-1) a RGB.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    [
        (((r1 + m) * 255.0).round().clamp(0.0, 255.0)) as u8,
        (((g1 + m) * 255.0).round().clamp(0.0, 255.0)) as u8,
        (((b1 + m) * 255.0).round().clamp(0.0, 255.0)) as u8,
    ]
}

/// Convierte RGB a (tono 0-360, saturación 0-1, valor 0-1).
pub fn rgb_to_hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let s = if max == 0.0 { 0.0 } else { delta / max };
    let v = max;

    (h, s, v)
}

pub fn rgb_to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

pub fn hex_to_rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b])
}

/// Genera un color arcoíris estilo RGB Gamer / Chroma a partir de un tiempo (segundos)
/// y un desplazamiento de fase horizontal/posicional.
pub fn chroma_rgb(time_secs: f32, offset: f32) -> [u8; 3] {
    let hue = (time_secs * 120.0 + offset * 12.0).rem_euclid(360.0);
    hsv_to_rgb(hue, 0.88, 1.0)
}

/// Versión normalizada [0.0, 1.0] para shaders y pipelines de renderizado.
pub fn chroma_rgb_f32(time_secs: f32, offset: f32) -> [f32; 3] {
    let [r, g, b] = chroma_rgb(time_secs, offset);
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

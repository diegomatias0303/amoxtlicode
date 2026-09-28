//! AmoxliCode - Personalización (Etapa A: datos y persistencia)
//!
//! Guarda las preferencias del usuario (tema de colores, fondo, fuente)
//! en un archivo JSON junto al ejecutable, para que se recuerden entre
//! sesiones.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Background {
    /// Sin video/imagen de fondo: solo el color sólido de la Capa 0.
    None,
    /// Un archivo elegido por el usuario (video o imagen).
    Custom(PathBuf),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeName {
    OneDark,
    Monokai,
    Light,
}

impl ThemeName {
    pub const ALL: [ThemeName; 3] = [ThemeName::OneDark, ThemeName::Monokai, ThemeName::Light];

    pub fn label(&self) -> &'static str {
        match self {
            ThemeName::OneDark => "One Dark",
            ThemeName::Monokai => "Monokai",
            ThemeName::Light => "Claro",
        }
    }

    /// Color de fondo sólido (Capa 0 / clear color) para este tema.
    pub fn background_color(&self) -> [f32; 3] {
        match self {
            ThemeName::OneDark => [0.043, 0.043, 0.059],
            ThemeName::Monokai => [0.11, 0.11, 0.10],
            ThemeName::Light => [0.96, 0.96, 0.94],
        }
    }

    /// Color de texto "normal" (sin resaltado especial) para este tema.
    pub fn default_text_color(&self) -> [u8; 3] {
        match self {
            ThemeName::OneDark | ThemeName::Monokai => [220, 220, 225],
            ThemeName::Light => [40, 40, 40],
        }
    }

    /// Paleta base: nombre de highlight (los mismos que usa tree-sitter en
    /// `syntax.rs`) -> color. El usuario puede sobreescribir cualquiera de
    /// estas entradas individualmente desde `Settings::custom_colors`.
    pub fn palette(&self) -> HashMap<&'static str, [u8; 3]> {
        let entries: &[(&str, [u8; 3])] = match self {
            ThemeName::OneDark => &[
                ("keyword", [198, 120, 221]),
                ("string", [152, 195, 121]),
                ("comment", [130, 137, 151]),
                ("number", [209, 154, 102]),
                ("function", [97, 175, 239]),
                ("type", [229, 192, 123]),
                ("operator", [86, 182, 194]),
            ],
            ThemeName::Monokai => &[
                ("keyword", [249, 38, 114]),
                ("string", [230, 219, 116]),
                ("comment", [117, 113, 94]),
                ("number", [174, 129, 255]),
                ("function", [166, 226, 46]),
                ("type", [102, 217, 239]),
                ("operator", [249, 38, 114]),
            ],
            ThemeName::Light => &[
                ("keyword", [166, 38, 164]),
                ("string", [80, 161, 79]),
                ("comment", [150, 150, 150]),
                ("number", [188, 105, 30]),
                ("function", [64, 120, 242]),
                ("type", [193, 132, 1]),
                ("operator", [90, 90, 90]),
            ],
        };
        entries.iter().cloned().collect()
    }
}

use crate::color::hex_to_rgb;

/// Una combinación de colores personalizados guardada con un nombre, para
/// poder cargarla después.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ColorPreset {
    pub name: String,
    pub colors: HashMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RgbElement {
    Syntax,
    Topbar,
    Cursor,
    Menus,
    Scrollbar,
    Markers,
    LineNumbers,
    Terminal,
}

impl RgbElement {
    pub const ALL: [RgbElement; 8] = [
        RgbElement::Syntax,
        RgbElement::Topbar,
        RgbElement::Cursor,
        RgbElement::Menus,
        RgbElement::Scrollbar,
        RgbElement::Markers,
        RgbElement::LineNumbers,
        RgbElement::Terminal,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            RgbElement::Syntax => "Palabras clave",
            RgbElement::Topbar => "Barra superior",
            RgbElement::Cursor => "Cursor",
            RgbElement::Menus => "Menús desplegables",
            RgbElement::Scrollbar => "Scrollbar",
            RgbElement::Markers => "Marcadores línea",
            RgbElement::LineNumbers => "Números de línea",
            RgbElement::Terminal => "Terminal integrada",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub background: Background,
    pub theme: ThemeName,
    /// Sobreescrituras individuales de color: nombre de highlight (ej.
    /// "keyword") -> color en hexadecimal (ej. "#FF0080"). Tiene
    /// prioridad sobre la paleta del tema.
    pub custom_colors: HashMap<String, String>,
    pub font_family: String,
    /// Modo arcoíris animado Gamer / Chroma maestro (F8).
    #[serde(default)]
    pub rgb_gamer_mode: bool,
    /// Opacidad del fondo del área de código sobre video/imagen (0.0 a 1.0).
    #[serde(default = "default_editor_opacity")]
    pub editor_opacity: f32,
    /// Opacidad de la línea de marcador tipo Dev-C++ (0.0 a 1.0).
    #[serde(default = "default_marker_opacity")]
    pub marker_opacity: f32,
    /// Opacidad de los menús desplegables (0.0 a 1.0).
    #[serde(default = "default_menu_opacity")]
    pub menu_opacity: f32,
    #[serde(default = "default_topbar_opacity")]
    pub topbar_opacity: f32,
    #[serde(default = "default_terminal_opacity")]
    pub terminal_opacity: f32,
    /// Opacidad de la barra de desplazamiento izquierda (0.0 a 1.0).
    #[serde(default = "default_scrollbar_opacity")]
    pub scrollbar_opacity: f32,

    // Elementos donde aplica el modo RGB Gamer
    #[serde(default = "default_true")]
    pub rgb_topbar: bool,
    #[serde(default = "default_true")]
    pub rgb_syntax: bool,
    #[serde(default = "default_true")]
    pub rgb_cursor: bool,
    #[serde(default)]
    pub rgb_menus: bool,
    #[serde(default)]
    pub rgb_scrollbar: bool,
    #[serde(default)]
    pub rgb_markers: bool,
    #[serde(default)]
    pub rgb_line_numbers: bool,
    #[serde(default)]
    pub rgb_terminal: bool,

    /// Presets de colores guardados por el usuario, con nombre.
    #[serde(default)]
    pub presets: Vec<ColorPreset>,
    /// Sonido asignado a cada acción (clave -> ruta del archivo). Claves
    /// usadas: "keypress", "paste", "cut", "copy", "undo", "redo",
    /// "select_all".
    #[serde(default)]
    pub sounds: HashMap<String, PathBuf>,
}

fn default_editor_opacity() -> f32 {
    0.85
}

fn default_marker_opacity() -> f32 {
    0.30
}

fn default_menu_opacity() -> f32 {
    0.96
}

fn default_topbar_opacity() -> f32 {
    0.96
}

fn default_terminal_opacity() -> f32 {
    0.90
}

fn default_scrollbar_opacity() -> f32 {
    0.75
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background: Background::None,
            theme: ThemeName::OneDark,
            custom_colors: HashMap::new(),
            font_family: "monospace".to_string(),
            rgb_gamer_mode: false,
            editor_opacity: 0.85,
            marker_opacity: 0.30,
            menu_opacity: 0.96,
            topbar_opacity: 0.96,
            terminal_opacity: 0.90,
            scrollbar_opacity: 0.75,
            rgb_topbar: true,
            rgb_syntax: true,
            rgb_cursor: true,
            rgb_menus: false,
            rgb_scrollbar: false,
            rgb_markers: false,
            rgb_line_numbers: false,
            rgb_terminal: false,
            presets: Vec::new(),
            sounds: HashMap::new(),
        }
    }
}

impl Settings {
    fn file_path() -> PathBuf {
        PathBuf::from("amoxlicode_settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::file_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(Self::file_path(), json) {
                    log::error!("no se pudo guardar la configuración: {e:?}");
                }
            }
            Err(e) => log::error!("no se pudo serializar la configuración: {e:?}"),
        }
    }

    /// Color final para un highlight dado: la sobreescritura del usuario
    /// si existe, si no el color del tema actual, si no el color de texto
    /// normal del tema.
    pub fn color_for(&self, highlight: Option<&str>) -> [u8; 3] {
        if let Some(name) = highlight {
            if let Some(hex) = self.custom_colors.get(name) {
                if let Some(rgb) = hex_to_rgb(hex) {
                    return rgb;
                }
            }
            match name {
                "topbar_accent" => return match self.theme {
                    ThemeName::OneDark => [97, 175, 239],
                    ThemeName::Monokai => [166, 226, 46],
                    ThemeName::Light => [38, 115, 242],
                },
                "topbar_bg" => return match self.theme {
                    ThemeName::OneDark => [18, 20, 27],
                    ThemeName::Monokai => [30, 30, 28],
                    ThemeName::Light => [230, 230, 235],
                },
                "topbar_text" => return match self.theme {
                    ThemeName::Light => [40, 45, 55],
                    _ => [215, 220, 230],
                },
                "terminal_bg" => return match self.theme {
                    ThemeName::OneDark => [15, 15, 20],
                    ThemeName::Monokai => [20, 20, 18],
                    ThemeName::Light => [240, 240, 245],
                },
                "terminal_text" => return match self.theme {
                    ThemeName::Light => [40, 45, 55],
                    _ => [200, 200, 200],
                },
                "line_numbers" => return match self.theme {
                    ThemeName::OneDark => [110, 114, 128],
                    ThemeName::Monokai => [117, 113, 94],
                    ThemeName::Light => [160, 160, 160],
                },
                "tab_active" => return match self.theme {
                    ThemeName::OneDark => [36, 40, 54],
                    ThemeName::Monokai => [46, 46, 44],
                    ThemeName::Light => [250, 250, 255],
                },
                "line_marker" => return match self.theme {
                    ThemeName::Light => [235, 60, 60],
                    _ => [220, 50, 50],
                },
                "menu_bg" => return match self.theme {
                    ThemeName::OneDark => [22, 24, 30],
                    ThemeName::Monokai => [32, 33, 30],
                    ThemeName::Light => [242, 244, 250],
                },
                "scrollbar" => return match self.theme {
                    ThemeName::OneDark => [120, 140, 180],
                    ThemeName::Monokai => [150, 150, 130],
                    ThemeName::Light => [140, 150, 175],
                },
                _ => {}
            }
            if let Some(theme_color) = self.theme.palette().get(name) {
                return *theme_color;
            }
        }
        self.theme.default_text_color()
    }

    pub fn topbar_accent_rgb(&self) -> [f32; 3] {
        let rgb = self.color_for(Some("topbar_accent"));
        [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0]
    }

    pub fn topbar_bg_rgb(&self) -> [f32; 3] {
        let rgb = self.color_for(Some("topbar_bg"));
        [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0]
    }

    #[allow(dead_code)]
    pub fn topbar_text_rgb(&self) -> [f32; 3] {
        let rgb = self.color_for(Some("topbar_text"));
        [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0]
    }

    pub fn tab_active_rgb(&self) -> [f32; 3] {
        let rgb = self.color_for(Some("tab_active"));
        [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0]
    }

    pub fn toggle_rgb_gamer_mode(&mut self) {
        self.rgb_gamer_mode = !self.rgb_gamer_mode;
    }

    pub fn cycle_editor_opacity(&mut self) {
        self.editor_opacity = match (self.editor_opacity * 100.0).round() as u32 {
            0..=10 => 0.25,
            11..=30 => 0.50,
            31..=60 => 0.75,
            61..=80 => 0.90,
            81..=95 => 1.00,
            _ => 0.00,
        };
    }

    pub fn cycle_marker_opacity(&mut self) {
        self.marker_opacity = match (self.marker_opacity * 100.0).round() as u32 {
            0..=20 => 0.30,
            21..=35 => 0.45,
            36..=50 => 0.65,
            51..=75 => 0.85,
            _ => 0.15,
        };
    }

    pub fn cycle_terminal_opacity(&mut self) {
        self.terminal_opacity = match (self.terminal_opacity * 100.0).round() as u32 {
            0..=40 => 0.50,
            41..=60 => 0.70,
            61..=80 => 0.85,
            81..=90 => 0.95,
            91..=99 => 1.00,
            _ => 0.00,
        };
    }

    pub fn cycle_topbar_opacity(&mut self) {
        self.topbar_opacity = match (self.topbar_opacity * 100.0).round() as u32 {
            0..=40 => 0.50,
            41..=60 => 0.70,
            61..=80 => 0.85,
            81..=90 => 0.92,
            91..=97 => 1.00,
            _ => 0.00,
        };
    }

    pub fn cycle_menu_opacity(&mut self) {
        self.menu_opacity = match (self.menu_opacity * 100.0).round() as u32 {
            0..=40 => 0.50,
            41..=60 => 0.70,
            61..=80 => 0.85,
            81..=90 => 0.92,
            91..=97 => 1.00,
            _ => 0.00,
        };
    }

    pub fn cycle_scrollbar_opacity(&mut self) {
        self.scrollbar_opacity = match (self.scrollbar_opacity * 100.0).round() as u32 {
            0..=40 => 0.50,
            41..=65 => 0.75,
            66..=85 => 0.90,
            86..=95 => 1.00,
            _ => 0.30,
        };
    }

    pub fn menu_bg_rgb(&self) -> [f32; 3] {
        let rgb = self.color_for(Some("menu_bg"));
        [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0]
    }

    pub fn scrollbar_color_rgb(&self) -> [f32; 3] {
        let rgb = self.color_for(Some("scrollbar"));
        [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0]
    }

    pub fn is_rgb_active(&self, elem: RgbElement) -> bool {
        if !self.rgb_gamer_mode {
            return false;
        }
        match elem {
            RgbElement::Topbar => self.rgb_topbar,
            RgbElement::Syntax => self.rgb_syntax,
            RgbElement::Cursor => self.rgb_cursor,
            RgbElement::Menus => self.rgb_menus,
            RgbElement::Scrollbar => self.rgb_scrollbar,
            RgbElement::Markers => self.rgb_markers,
            RgbElement::LineNumbers => self.rgb_line_numbers,
            RgbElement::Terminal => self.rgb_terminal,
        }
    }

    pub fn is_rgb_element_enabled(&self, elem: RgbElement) -> bool {
        match elem {
            RgbElement::Topbar => self.rgb_topbar,
            RgbElement::Syntax => self.rgb_syntax,
            RgbElement::Cursor => self.rgb_cursor,
            RgbElement::Menus => self.rgb_menus,
            RgbElement::Scrollbar => self.rgb_scrollbar,
            RgbElement::Markers => self.rgb_markers,
            RgbElement::LineNumbers => self.rgb_line_numbers,
            RgbElement::Terminal => self.rgb_terminal,
        }
    }

    pub fn toggle_rgb_element(&mut self, elem: RgbElement) {
        match elem {
            RgbElement::Topbar => self.rgb_topbar = !self.rgb_topbar,
            RgbElement::Syntax => self.rgb_syntax = !self.rgb_syntax,
            RgbElement::Cursor => self.rgb_cursor = !self.rgb_cursor,
            RgbElement::Menus => self.rgb_menus = !self.rgb_menus,
            RgbElement::Scrollbar => self.rgb_scrollbar = !self.rgb_scrollbar,
            RgbElement::Markers => self.rgb_markers = !self.rgb_markers,
            RgbElement::LineNumbers => self.rgb_line_numbers = !self.rgb_line_numbers,
            RgbElement::Terminal => self.rgb_terminal = !self.rgb_terminal,
        }
    }

    pub fn tab_bar_bg_rgb(&self) -> [f32; 3] {
        match self.theme {
            ThemeName::OneDark => [0.08, 0.09, 0.12],
            ThemeName::Monokai => [0.13, 0.13, 0.12],
            ThemeName::Light => [0.86, 0.86, 0.88],
        }
    }

    /// Guarda un color nuevo (en hexadecimal) para un tipo de token.
    pub fn set_custom_color(&mut self, highlight: &str, color: [u8; 3]) {
        self.custom_colors
            .insert(highlight.to_string(), crate::color::rgb_to_hex(color));
    }

    pub fn reset_custom_colors(&mut self) {
        self.custom_colors.clear();
    }

    /// Guarda la combinación de colores actual como un preset nuevo con
    /// el nombre dado.
    pub fn save_preset(&mut self, name: String) {
        self.presets.push(ColorPreset {
            name,
            colors: self.custom_colors.clone(),
        });
    }

    /// Reemplaza los colores personalizados actuales por los de un
    /// preset guardado.
    pub fn apply_preset(&mut self, index: usize) {
        if let Some(preset) = self.presets.get(index) {
            self.custom_colors = preset.colors.clone();
        }
    }

    #[allow(dead_code)]
    pub fn delete_preset(&mut self, index: usize) {
        if index < self.presets.len() {
            self.presets.remove(index);
        }
    }
}
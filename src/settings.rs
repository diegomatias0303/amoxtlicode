//! AmoxliCode - Personalización (datos y persistencia)
//!
//! Guarda las preferencias del usuario (tema de colores, fondo, fuente,
//! opacidades, presets y sonidos) en un archivo JSON persistente.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::color::hex_to_rgb;

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

    /// Búsqueda zero-allocation de color de sintaxis para todos los scopes de
    /// tree-sitter y del analizador léxico.
    pub fn palette_color(&self, name: &str) -> Option<[u8; 3]> {
        let color = match self {
            ThemeName::OneDark => match name {
                "keyword" => [198, 120, 221],
                "string" | "string.special" => [152, 195, 121],
                "comment" => [130, 137, 151],
                "number" | "constant" | "constant.builtin" => [209, 154, 102],
                "function" | "function.builtin" | "constructor" => [97, 175, 239],
                "type" | "type.builtin" => [229, 192, 123],
                "operator" => [86, 182, 194],
                "tag" => [224, 108, 117],
                "attribute" | "property" => [209, 154, 102],
                "variable.builtin" => [229, 192, 123],
                "variable.parameter" => [224, 108, 117],
                "punctuation" | "punctuation.bracket" | "punctuation.delimiter" => [171, 178, 191],
                _ => return None,
            },
            ThemeName::Monokai => match name {
                "keyword" => [249, 38, 114],
                "string" | "string.special" => [230, 219, 116],
                "comment" => [117, 113, 94],
                "number" | "constant" | "constant.builtin" => [174, 129, 255],
                "function" | "function.builtin" | "constructor" => [166, 226, 46],
                "type" | "type.builtin" => [102, 217, 239],
                "operator" => [249, 38, 114],
                "tag" => [249, 38, 114],
                "attribute" | "property" => [166, 226, 46],
                "variable.builtin" => [253, 151, 31],
                "variable.parameter" => [253, 151, 31],
                "punctuation" | "punctuation.bracket" | "punctuation.delimiter" => [200, 200, 190],
                _ => return None,
            },
            ThemeName::Light => match name {
                "keyword" => [166, 38, 164],
                "string" | "string.special" => [80, 161, 79],
                "comment" => [140, 145, 155],
                "number" | "constant" | "constant.builtin" => [188, 105, 30],
                "function" | "function.builtin" | "constructor" => [64, 120, 242],
                "type" | "type.builtin" => [193, 132, 1],
                "operator" => [90, 90, 90],
                "tag" => [210, 55, 65],
                "attribute" | "property" => [152, 104, 1],
                "variable.builtin" => [193, 132, 1],
                "variable.parameter" => [180, 75, 50],
                "punctuation" | "punctuation.bracket" | "punctuation.delimiter" => [100, 105, 115],
                _ => return None,
            },
        };
        Some(color)
    }

    #[allow(dead_code)]
    pub fn palette(&self) -> HashMap<&'static str, [u8; 3]> {
        let keys = [
            "keyword", "string", "comment", "number", "function", "type", "operator",
        ];
        keys.iter()
            .filter_map(|&k| self.palette_color(k).map(|c| (k, c)))
            .collect()
    }
}

/// Una combinación de colores personalizados guardada con un nombre.
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
    pub custom_colors: HashMap<String, String>,
    pub font_family: String,
    #[serde(default)]
    pub rgb_gamer_mode: bool,
    #[serde(default = "default_editor_opacity")]
    pub editor_opacity: f32,
    #[serde(default = "default_marker_opacity")]
    pub marker_opacity: f32,
    #[serde(default = "default_menu_opacity")]
    pub menu_opacity: f32,
    #[serde(default = "default_topbar_opacity")]
    pub topbar_opacity: f32,
    #[serde(default = "default_terminal_opacity")]
    pub terminal_opacity: f32,
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

    #[serde(default = "default_terminal_font_scale")]
    pub terminal_font_scale: f32,

    #[serde(default = "default_true")]
    pub auto_close_pairs: bool,

    #[serde(default)]
    pub recent_files: Vec<PathBuf>,

    #[serde(default)]
    pub presets: Vec<ColorPreset>,
    #[serde(default)]
    pub sounds: HashMap<String, PathBuf>,
}

fn default_terminal_font_scale() -> f32 {
    1.0
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
            terminal_font_scale: 1.0,
            auto_close_pairs: true,
            recent_files: Vec::new(),
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
    /// Resuelve la ruta estable del archivo de configuración:
    /// 1. Si ya existe `amoxlicode_settings.json` en el directorio actual (modo dev/portable), úsalo.
    /// 2. Si existe junto al ejecutable, úsalo.
    /// 3. Si no, usa el directorio de configuración del usuario (`%APPDATA%\AmoxliCode\` en Windows
    ///    o `~/.config/amoxlicode/` en Linux) para no crear JSONs dispersos al abrir archivos desde el explorador.
    fn file_path() -> PathBuf {
        let local = PathBuf::from("amoxlicode_settings.json");
        if local.exists() {
            return local;
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let beside_exe = dir.join("amoxlicode_settings.json");
                if beside_exe.exists() {
                    return beside_exe;
                }
            }
        }
        #[cfg(target_os = "windows")]
        if let Ok(appdata) = std::env::var("APPDATA") {
            let dir = PathBuf::from(appdata).join("AmoxliCode");
            return dir.join("amoxlicode_settings.json");
        }
        #[cfg(not(target_os = "windows"))]
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("amoxlicode").join("amoxlicode_settings.json");
        } else if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config").join("amoxlicode").join("amoxlicode_settings.json");
        }
        local
    }

    pub fn exists_on_disk() -> bool {
        Self::file_path().exists()
    }

    fn sanitize(&mut self) {
        self.editor_opacity = self.editor_opacity.clamp(0.0, 1.0);
        self.marker_opacity = self.marker_opacity.clamp(0.0, 1.0);
        self.menu_opacity = self.menu_opacity.clamp(0.0, 1.0);
        self.topbar_opacity = self.topbar_opacity.clamp(0.0, 1.0);
        self.terminal_opacity = self.terminal_opacity.clamp(0.0, 1.0);
        self.scrollbar_opacity = self.scrollbar_opacity.clamp(0.0, 1.0);
        self.terminal_font_scale = self.terminal_font_scale.clamp(0.6, 2.5);
        self.recent_files.truncate(8);
    }

    pub fn push_recent_file(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(8);
    }

    pub fn toggle_auto_close_pairs(&mut self) {
        self.auto_close_pairs = !self.auto_close_pairs;
    }

    pub fn load() -> Self {
        let mut settings: Self = std::fs::read_to_string(Self::file_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        settings.sanitize();
        settings
    }

    pub fn save(&self) {
        let path = Self::file_path();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    log::error!("no se pudo guardar la configuración en {path:?}: {e:?}");
                }
            }
            Err(e) => log::error!("no se pudo serializar la configuración: {e:?}"),
        }
    }

    /// Color final para un highlight dado (zero-allocation en la ruta rápida).
    pub fn color_for(&self, highlight: Option<&str>) -> [u8; 3] {
        if let Some(name) = highlight {
            if let Some(hex) = self.custom_colors.get(name) {
                if let Some(rgb) = hex_to_rgb(hex) {
                    return rgb;
                }
            }
            // También respetamos sobreescrituras de scope padre (ej. "function" aplica a "function.builtin")
            if let Some((parent, _)) = name.split_once('.') {
                if let Some(hex) = self.custom_colors.get(parent) {
                    if let Some(rgb) = hex_to_rgb(hex) {
                        return rgb;
                    }
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
                    ThemeName::Light => [140, 145, 155],
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
            if let Some(theme_color) = self.theme.palette_color(name) {
                return theme_color;
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
            _ => 0.50,
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

    pub fn set_custom_color(&mut self, highlight: &str, color: [u8; 3]) {
        self.custom_colors
            .insert(highlight.to_string(), crate::color::rgb_to_hex(color));
    }

    pub fn reset_custom_colors(&mut self) {
        self.custom_colors.clear();
    }

    pub fn save_preset(&mut self, name: String) {
        self.presets.push(ColorPreset {
            name,
            colors: self.custom_colors.clone(),
        });
    }

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

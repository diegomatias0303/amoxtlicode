//! AmoxliCode - Motor de texto (Capa 1), ahora con soporte de múltiples
//! archivos abiertos a la vez (pestañas).
//!
//! Cada archivo abierto es un `Document` (su propio texto, cursor, scroll
//! y caché de resaltado). `TextState` guarda la lista de documentos más
//! todo lo que es compartido entre ellos (fuente, zoom, panel, selector
//! de color, barra de herramientas, etc.).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::settings::Settings;
use crate::syntax::{Language, SyntaxHighlighter};
use glyphon::cosmic_text::Wrap;
use glyphon::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer,
};
use wgpu::{Device, MultisampleState, Queue, TextureFormat};

/// Tamaño de fuente y separación entre líneas (en píxeles lógicos).
const FONT_SIZE: f32 = 20.0;
const LINE_HEIGHT: f32 = 26.0;
/// Margen desde la esquina superior izquierda de la ventana.
pub const PADDING: f32 = 24.0;
/// Ancho de la franja de números de línea, a la izquierda del código.
pub const GUTTER_WIDTH: f32 = 56.0;
pub const SCROLLBAR_WIDTH: f32 = 16.0;

const PANEL_ROW_HEIGHT: f32 = 24.0;
const PANEL_WIDTH: f32 = 340.0;

pub const TOOLBAR_HEIGHT: f32 = 30.0;
pub const TOOLBAR_FONT_SIZE: f32 = 14.0;

pub const TAB_HEIGHT: f32 = 28.0;
pub const TAB_FONT_SIZE: f32 = 14.0;
pub const TAB_SLOT_CHARS: usize = 20;
pub const MENU_ROW_HEIGHT: f32 = 22.0;

/// Todo el espacio ocupado arriba por la barra de herramientas + la
/// barra de pestañas. El área de texto y todos los overlays (panel,
/// selector, popup) empiezan después de esto.
pub const TOP_OFFSET: f32 = TOOLBAR_HEIGHT + TAB_HEIGHT;

/// Menús principales en la barra superior estilo Notepad++.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopMenu {
    Archivo,
    Editar,
    Buscar,
    Ver,
    Lenguaje,
    Configuracion,
}

/// Acciones invocadas desde los menús de la barra superior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum MenuAction {
    // Archivo
    New,
    Open,
    Save,
    SaveAs,
    CloseTab,
    SaveAll,
    Exit,

    // Editar
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Indent,
    Unindent,
    SelectAll,

    // Buscar
    Find,
    FindNext,

    // Ver
    ZoomIn,
    ZoomOut,
    ZoomReset,

    // Lenguaje
    SetLanguage(Language),

    // Editar
    ToggleBookmark,

    // Configuración
    SelectTheme(crate::settings::ThemeName),
    ChooseBackground,
    ClearBackground,
    ResetColors,
    SavePresetPrompt,
    OpenPanel,
    ToggleRgbGamer,

    // Terminal
    OpenTerminal,
}

pub type ToolbarAction = MenuAction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarHit {
    Menu(TopMenu),
    Action(MenuAction),
}

#[derive(Clone, Debug)]
pub struct MenuRow {
    pub label: String,
    pub shortcut: Option<&'static str>,
    pub action: Option<MenuAction>,
    pub is_checked: bool,
}

pub const TOOLBAR_ITEMS: [(&str, Option<TopMenu>, Option<MenuAction>, f32, f32); 7] = [
    ("Archivo", Some(TopMenu::Archivo), None, 12.0, 87.0),
    ("Editar", Some(TopMenu::Editar), None, 87.0, 154.0),
    ("Buscar", Some(TopMenu::Buscar), None, 154.0, 221.0),
    ("Ver", Some(TopMenu::Ver), None, 221.0, 263.0),
    ("Lenguaje", Some(TopMenu::Lenguaje), None, 263.0, 347.0),
    ("Configuración", Some(TopMenu::Configuracion), None, 347.0, 473.0),
    ("Terminal", None, Some(MenuAction::OpenTerminal), 473.0, 557.0),
];

/// Lo que puede pasar al hacerle clic a una fila del panel de
/// personalización.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelAction {
    SelectTheme(crate::settings::ThemeName),
    ChooseBackground,
    ClearBackground,
    CycleEditorOpacity,
    EditColor(&'static str),
    CycleMarkerOpacity,
    CycleMenuOpacity,
    CycleTopbarOpacity,
    CycleScrollbarOpacity,
    ToggleRgbGamer,
    ToggleRgbElement(crate::settings::RgbElement),
    ResetColors,
    SavePresetPrompt,
    LoadPreset(usize),
    AssignSound(&'static str),
    ClearSounds,
    Close,
}

/// Acciones para las que se puede asignar un sonido, con su etiqueta
/// legible para el panel.
pub const SOUND_ACTIONS: [(&str, &str); 7] = [
    ("keypress", "Al escribir (tecla normal)"),
    ("paste", "Pegar (Ctrl+V)"),
    ("cut", "Cortar (Ctrl+X)"),
    ("copy", "Copiar (Ctrl+C)"),
    ("undo", "Deshacer (Ctrl+Z)"),
    ("redo", "Rehacer (Ctrl+Y)"),
    ("select_all", "Seleccionar todo (Ctrl+A)"),
];

/// Para qué se está usando la cajita de texto genérica en este momento.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptHit {
    Drag,
    Button,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptPurpose {
    SavePreset,
    Find,
}

struct PanelRow {
    label: String,
    action: Option<PanelAction>,
    swatch: Option<[u8; 3]>,
}

/// Todo lo que pertenece a UN archivo abierto: su texto, su cursor, su
/// scroll, y el `Buffer` de cosmic-text donde se dibuja.
/// Qué tipo de edición fue la última, para agrupar pulsaciones seguidas
/// (ej. escribir "hola" cuenta como un solo paso de deshacer, no cuatro).
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    None,
    Insert,
    Delete,
    /// Siempre inicia un paso nuevo (Enter, pegar, borrar selección).
    Other,
}

/// Una "foto" del contenido para poder regresar a ella con deshacer.
struct UndoSnapshot {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
}

struct Document {
    buffer: Buffer,
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
    scroll_offset: usize,
    cached_full_text: String,
    cached_spans: Vec<(std::ops::Range<usize>, Option<&'static str>)>,
    file_path: Option<PathBuf>,
    untitled_name: String,
    /// Punto donde empezó la selección actual (línea, columna). `None`
    /// significa que no hay ninguna selección activa.
    selection_anchor: Option<(usize, usize)>,
    undo_stack: Vec<UndoSnapshot>,
    redo_stack: Vec<UndoSnapshot>,
    last_edit_kind: EditKind,
    /// true si hay cambios sin guardar desde el último guardado/apertura.
    dirty: bool,
    language: Language,
    bookmarks: std::collections::HashSet<usize>,
}

impl Document {
    fn new(
        font_system: &mut FontSystem,
        metrics: Metrics,
        untitled_name: String,
        language: Language,
    ) -> Self {
        let mut buffer = Buffer::new(font_system, metrics);
        buffer.set_wrap(font_system, Wrap::None);
        buffer.set_text(
            font_system,
            "",
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(font_system);
        Self {
            buffer,
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
            scroll_offset: 0,
            cached_full_text: String::new(),
            cached_spans: Vec::new(),
            file_path: None,
            untitled_name,
            selection_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit_kind: EditKind::None,
            dirty: false,
            language,
            bookmarks: std::collections::HashSet::new(),
        }
    }

    fn tab_title(&self) -> String {
        match &self.file_path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.untitled_name.clone()),
            None => self.untitled_name.clone(),
        }
    }
}

pub struct TextState {
    font_system: FontSystem,
    swash_cache: SwashCache,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    overlay_text_renderer: TextRenderer,

    documents: Vec<Document>,
    active: usize,
    next_untitled: usize,

    /// Un resaltador por cada lenguaje que se haya necesitado hasta
    /// ahora (se crea la primera vez que se abre un archivo de ese tipo,
    /// no de entrada para los seis lenguajes).
    syntax_cache: HashMap<Language, SyntaxHighlighter>,

    completion_buffer: Buffer,
    completion_visible: bool,
    completion_items: Vec<crate::lsp::CompletionItem>,
    completion_selected: usize,

    cursor_blink_on: bool,

    viewport_width: f32,
    viewport_height: f32,
    font_scale: f32,

    dirty: bool,
    metrics_dirty: bool,
    /// true mientras `paste_text` está insertando el texto pegado (evita
    /// que se guarde una foto de deshacer por cada letra pegada).
    paste_in_progress: bool,

    pub settings: Settings,

    gear_buffer: Buffer,
    panel_buffer: Buffer,
    panel_open: bool,
    panel_rows: Vec<PanelRow>,
    panel_scroll: usize,

    picker_buffer: Buffer,
    picker_open: bool,
    picker_target: Option<&'static str>,
    picker_hue: f32,
    picker_sat: f32,
    picker_val: f32,

    toolbar_buffer: Buffer,
    tab_buffer: Buffer,
    lang_badge_buffer: Buffer,

    // --- Notepad++ Menú Superior ---
    menu_buffer: Buffer,
    active_menu: Option<TopMenu>,
    menu_rows: Vec<MenuRow>,

    // --- Cajita de texto genérica (ej. nombrar un preset, buscar) ---
    prompt_buffer: Buffer,
    prompt_open: bool,
    prompt_text: String,
    prompt_label: String,
    prompt_purpose: Option<PromptPurpose>,
    pub prompt_x: Option<f32>,
    pub prompt_y: Option<f32>,
    pub prompt_dragging: bool,
    pub prompt_drag_offset: (f32, f32),

    pub tooltip_text: Option<String>,
    pub tooltip_expiration: f32,
    pub tooltip_pos: (f32, f32),
    pub tooltip_buffer: Buffer,
    pub last_search_query: String,

    // --- Números de línea ---
    line_numbers_buffer: Buffer,

    pub search_result_marker: Option<(usize, f32)>,
    pub anim_time: f32,
}

impl TextState {
    pub fn new(device: &Device, queue: &Queue, format: TextureFormat, width: u32, height: u32) -> Self {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let mut atlas = TextAtlas::new(device, queue, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, device, MultisampleState::default(), None);
        let overlay_text_renderer =
            TextRenderer::new(&mut atlas, device, MultisampleState::default(), None);

        let metrics = Metrics::new(FONT_SIZE, LINE_HEIGHT);
        let mut first_doc = Document::new(
            &mut font_system,
            metrics,
            "Sin título 1.txt".to_string(),
            Language::PlainText,
        );
        first_doc.buffer.set_size(
            &mut font_system,
            width as f32 - PADDING * 2.0 - GUTTER_WIDTH,
            height as f32 - TOP_OFFSET,
        );
        first_doc.lines = vec![String::new()];
        first_doc.cursor_line = 0;
        first_doc.cursor_col = 0;

        let mut completion_buffer =
            Buffer::new(&mut font_system, Metrics::new(FONT_SIZE * 0.85, LINE_HEIGHT * 0.85));
        completion_buffer.set_size(&mut font_system, 320.0, 300.0);

        let mut gear_buffer = Buffer::new(&mut font_system, Metrics::new(20.0, 24.0));
        gear_buffer.set_size(&mut font_system, 40.0, 30.0);
        gear_buffer.set_text(
            &mut font_system,
            "\u{2699}",
            Attrs::new().family(Family::Monospace).color(Color::rgb(220, 220, 225)),
            Shaping::Advanced,
        );
        gear_buffer.shape_until_scroll(&mut font_system);

        let mut panel_buffer = Buffer::new(&mut font_system, Metrics::new(14.0, PANEL_ROW_HEIGHT));
        panel_buffer.set_wrap(&mut font_system, Wrap::None);
        panel_buffer.set_size(&mut font_system, PANEL_WIDTH - 16.0, 600.0);

        let mut picker_buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        picker_buffer.set_size(&mut font_system, PANEL_WIDTH - 16.0, 80.0);

        let mut toolbar_buffer =
            Buffer::new(&mut font_system, Metrics::new(TOOLBAR_FONT_SIZE, TOOLBAR_HEIGHT));
        toolbar_buffer.set_size(&mut font_system, width as f32, TOOLBAR_HEIGHT);

        let mut tab_buffer = Buffer::new(&mut font_system, Metrics::new(TAB_FONT_SIZE, TAB_HEIGHT));
        tab_buffer.set_size(&mut font_system, width as f32, TAB_HEIGHT);

        let mut lang_badge_buffer =
            Buffer::new(&mut font_system, Metrics::new(TOOLBAR_FONT_SIZE, TOOLBAR_HEIGHT));
        lang_badge_buffer.set_size(&mut font_system, 180.0, TOOLBAR_HEIGHT);

        let mut menu_buffer = Buffer::new(&mut font_system, Metrics::new(13.0, MENU_ROW_HEIGHT));
        menu_buffer.set_size(&mut font_system, 340.0, 600.0);

        let mut prompt_buffer = Buffer::new(&mut font_system, Metrics::new(17.0, 24.0));
        prompt_buffer.set_size(&mut font_system, 320.0, 90.0);

        let mut tooltip_buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        tooltip_buffer.set_size(&mut font_system, 400.0, 50.0);

        let mut line_numbers_buffer =
            Buffer::new(&mut font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        line_numbers_buffer.set_size(
            &mut font_system,
            GUTTER_WIDTH - 8.0,
            height as f32 - TOP_OFFSET,
        );

        let settings = Settings::load();

        let mut state = Self {
            font_system,
            swash_cache,
            atlas,
            text_renderer,
            overlay_text_renderer,
            documents: vec![first_doc],
            active: 0,
            next_untitled: 2,
            syntax_cache: HashMap::new(),
            completion_buffer,
            completion_visible: false,
            completion_items: Vec::new(),
            completion_selected: 0,
            cursor_blink_on: true,
            viewport_width: width as f32,
            viewport_height: height as f32,
            font_scale: 1.0,
            dirty: true,
            metrics_dirty: false,
            paste_in_progress: false,
            settings,
            gear_buffer,
            panel_buffer,
            panel_open: false,
            panel_rows: Vec::new(),
            panel_scroll: 0,
            picker_buffer,
            picker_open: false,
            picker_target: None,
            picker_hue: 0.0,
            picker_sat: 0.0,
            picker_val: 1.0,
            toolbar_buffer,
            tab_buffer,
            lang_badge_buffer,
            menu_buffer,
            active_menu: None,
            menu_rows: Vec::new(),
            prompt_buffer,
            prompt_open: false,
            prompt_text: String::new(),
            prompt_label: String::new(),
            prompt_purpose: None,
            prompt_x: None,
            prompt_y: None,
            prompt_dragging: false,
            prompt_drag_offset: (0.0, 0.0),
            tooltip_text: None,
            tooltip_expiration: 0.0,
            tooltip_pos: (0.0, 0.0),
            tooltip_buffer,
            last_search_query: String::new(),
            line_numbers_buffer,
            search_result_marker: None,
            anim_time: 0.0,
        };
        state.rebuild_toolbar_buffer();
        state.rebuild_tab_buffer();
        state.rebuild_line_numbers();
        state
    }

    // ---------------------------------------------------------------
    // Pestañas / documentos
    // ---------------------------------------------------------------

    pub fn new_tab(&mut self) {
        let metrics = Metrics::new(self.eff_font_size(), self.eff_line_height());
        let name = format!("Sin título {}.txt", self.next_untitled);
        self.next_untitled += 1;
        let mut doc = Document::new(&mut self.font_system, metrics, name, Language::PlainText);
        doc.buffer.set_size(
            &mut self.font_system,
            self.viewport_width - PADDING * 2.0 - GUTTER_WIDTH,
            self.viewport_height - TOP_OFFSET,
        );
        self.documents.push(doc);
        self.active = self.documents.len() - 1;
        self.dirty = true;
        self.rebuild_tab_buffer();
        self.rebuild_toolbar_buffer();
    }

    pub fn open_file_as_tab(&mut self, path: PathBuf, text: &str) {
        let metrics = Metrics::new(self.eff_font_size(), self.eff_line_height());
        let name = format!("Sin título {}.txt", self.next_untitled);
        let language = path
            .extension()
            .and_then(|e| e.to_str())
            .map(Language::from_extension)
            .unwrap_or(Language::PlainText);
        let mut doc = Document::new(&mut self.font_system, metrics, name, language);
        doc.buffer.set_size(
            &mut self.font_system,
            self.viewport_width - PADDING * 2.0 - GUTTER_WIDTH,
            self.viewport_height - TOP_OFFSET,
        );
        let normalized = text.replace("\r\n", "\n");
        doc.lines = if normalized.is_empty() {
            vec![String::new()]
        } else {
            normalized.split('\n').map(String::from).collect()
        };
        doc.file_path = Some(path);

        self.documents.push(doc);
        self.active = self.documents.len() - 1;
        self.hide_completions();
        self.dirty = true;
        self.rebuild_tab_buffer();
        self.rebuild_toolbar_buffer();
    }

    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.documents.len() {
            return;
        }
        self.documents.remove(idx);
        if self.documents.is_empty() {
            let metrics = Metrics::new(self.eff_font_size(), self.eff_line_height());
            let name = format!("Sin título {}.txt", self.next_untitled);
            self.next_untitled += 1;
            let mut doc = Document::new(&mut self.font_system, metrics, name, Language::PlainText);
            doc.buffer.set_size(
                &mut self.font_system,
                self.viewport_width - PADDING * 2.0 - GUTTER_WIDTH,
                self.viewport_height - TOP_OFFSET,
            );
            self.documents.push(doc);
        }
        if self.active >= self.documents.len() {
            self.active = self.documents.len() - 1;
        }
        self.dirty = true;
        self.rebuild_tab_buffer();
        self.rebuild_toolbar_buffer();
    }

    pub fn switch_tab(&mut self, idx: usize) {
        if idx < self.documents.len() && idx != self.active {
            self.active = idx;
            self.metrics_dirty = true;
            self.dirty = true;
            self.hide_completions();
            self.rebuild_tab_buffer();
            self.rebuild_toolbar_buffer();
        }
    }

    pub fn tab_count(&self) -> usize {
        self.documents.len()
    }

    pub fn active_tab(&self) -> usize {
        self.active
    }

    pub fn active_file_path(&self) -> Option<PathBuf> {
        self.documents[self.active].file_path.clone()
    }

    pub fn set_active_file_path(&mut self, path: PathBuf) {
        self.documents[self.active].file_path = Some(path);
        self.rebuild_tab_buffer();
    }

    pub fn active_title(&self) -> String {
        self.documents[self.active].tab_title()
    }

    pub fn is_dirty(&self, idx: usize) -> bool {
        self.documents.get(idx).map(|d| d.dirty).unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn active_is_dirty(&self) -> bool {
        self.is_dirty(self.active)
    }

    pub fn any_dirty(&self) -> bool {
        (0..self.documents.len()).any(|i| self.is_dirty(i))
    }

    pub fn active_language(&self) -> Language {
        self.documents[self.active].language
    }

    pub fn set_active_language(&mut self, lang: Language) {
        let doc = &mut self.documents[self.active];
        if doc.language != lang {
            doc.language = lang;
            if doc.file_path.is_none() {
                let base = doc.untitled_name.split('.').next().unwrap_or("Sin título 1");
                doc.untitled_name = format!("{}.{}", base, lang.default_extension());
            }
            doc.cached_spans.clear();
            doc.cached_full_text.clear();
            self.dirty = true;
            self.rebuild_tab_buffer();
            self.rebuild_toolbar_buffer();
        }
    }

    pub fn active_uri(&self) -> String {
        let doc = &self.documents[self.active];
        if let Some(ref path) = doc.file_path {
            let s = path.to_string_lossy().replace('\\', "/");
            if s.starts_with('/') {
                format!("file://{s}")
            } else {
                format!("file:///{s}")
            }
        } else {
            let ext = doc.language.default_extension();
            format!("file:///amoxlicode/{}.{}", doc.untitled_name.replace(' ', "_"), ext)
        }
    }

    /// Marca la pestaña activa como "sin cambios pendientes" (se llama
    /// justo después de guardar con éxito).
    pub fn mark_saved(&mut self) {
        self.documents[self.active].dirty = false;
        self.rebuild_tab_buffer();
    }

    // ---------------------------------------------------------------
    // Barra de pestañas
    // ---------------------------------------------------------------

    pub fn tab_bar_rect(&self) -> (f32, f32, f32, f32) {
        (0.0, TOOLBAR_HEIGHT, self.viewport_width, TAB_HEIGHT)
    }

    pub fn tab_rect(&self, idx: usize) -> (f32, f32, f32, f32) {
        let char_width = TAB_FONT_SIZE * 0.6;
        let slot_width = char_width * TAB_SLOT_CHARS as f32;
        let x = 8.0 + idx as f32 * slot_width;
        let y = TOOLBAR_HEIGHT + 2.0;
        let w = slot_width - 3.0;
        let h = TAB_HEIGHT - 2.0;
        (x, y, w, h)
    }

    pub fn add_tab_rect(&self) -> (f32, f32, f32, f32) {
        let char_width = TAB_FONT_SIZE * 0.6;
        let slot_width = char_width * TAB_SLOT_CHARS as f32;
        let add_x = 8.0 + self.documents.len() as f32 * slot_width + 4.0;
        let add_y = TOOLBAR_HEIGHT + 4.0;
        (add_x, add_y, 24.0, TAB_HEIGHT - 6.0)
    }

    pub fn add_tab_hit_test(&self, x: f32, y: f32) -> bool {
        let (ax, ay, aw, ah) = self.add_tab_rect();
        x >= ax && x <= ax + aw && y >= ay && y <= ay + ah
    }

    pub fn tab_hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if y < TOOLBAR_HEIGHT || y > TOOLBAR_HEIGHT + TAB_HEIGHT {
            return None;
        }
        let char_width = TAB_FONT_SIZE * 0.6;
        let slot_width = char_width * TAB_SLOT_CHARS as f32;
        let rel_x = x - 8.0;
        if rel_x < 0.0 {
            return None;
        }
        let idx = (rel_x / slot_width) as usize;
        if idx >= self.documents.len() {
            return None;
        }
        let within_slot = rel_x - (idx as f32 * slot_width);
        if within_slot > slot_width - char_width * 2.0 {
            return None;
        }
        Some(idx)
    }

    pub fn tab_close_hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if y < TOOLBAR_HEIGHT || y > TOOLBAR_HEIGHT + TAB_HEIGHT {
            return None;
        }
        let char_width = TAB_FONT_SIZE * 0.6;
        let slot_width = char_width * TAB_SLOT_CHARS as f32;
        let rel_x = x - 8.0;
        if rel_x < 0.0 {
            return None;
        }
        let idx = (rel_x / slot_width) as usize;
        if idx >= self.documents.len() {
            return None;
        }
        let within_slot = rel_x - (idx as f32 * slot_width);
        if within_slot > slot_width - char_width * 2.2 {
            Some(idx)
        } else {
            None
        }
    }

    fn rebuild_tab_buffer(&mut self) {
        let base_attrs = Attrs::new().family(Family::Monospace);
        let active_color = Color::rgb(255, 255, 255);
        let inactive_color = Color::rgb(155, 160, 172);
        let dirty_color = Color::rgb(255, 185, 45); // Ámbar cálido para archivos modificados
        let close_color = Color::rgb(150, 155, 165);

        let mut spans: Vec<(String, Color)> = Vec::new();
        for (i, doc) in self.documents.iter().enumerate() {
            let color = if i == self.active { active_color } else { inactive_color };
            let title = doc.tab_title();
            let max_chars = TAB_SLOT_CHARS.saturating_sub(4);
            let truncated = if title.chars().count() > max_chars {
                title.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "â€¦"
            } else {
                title
            };

            if doc.dirty {
                spans.push((" \u{25cf}".to_string(), dirty_color));
                spans.push((format!(" {truncated}"), color));
            } else {
                spans.push((format!("   {truncated}"), color));
            }

            let current_len = if doc.dirty {
                2 + 1 + truncated.chars().count()
            } else {
                3 + truncated.chars().count()
            };
            let pad = TAB_SLOT_CHARS.saturating_sub(current_len + 2);
            if pad > 0 {
                spans.push((" ".repeat(pad), color));
            }
            spans.push((" \u{d7}".to_string(), close_color));
        }

        // Botón '+para crear pestaña nueva rápidamente
        spans.push(("  +  ".to_string(), Color::rgb(160, 170, 190)));

        let rich_spans: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|(s, c)| (s.as_str(), base_attrs.clone().color(*c)))
            .collect();

        self.tab_buffer
            .set_rich_text(&mut self.font_system, rich_spans, Shaping::Advanced);
        self.tab_buffer.shape_until_scroll(&mut self.font_system);
    }

    // ---------------------------------------------------------------
    // Zoom / tamaño efectivo
    // ---------------------------------------------------------------

    fn eff_font_size(&self) -> f32 {
        FONT_SIZE * self.font_scale
    }
    fn eff_line_height(&self) -> f32 {
        LINE_HEIGHT * self.font_scale
    }

    pub fn line_height(&self) -> f32 {
        self.eff_line_height()
    }

    fn visible_lines(&self) -> usize {
        (((self.viewport_height - TOP_OFFSET - PADDING) / self.eff_line_height()).floor()
            as usize)
            .max(1)
    }

    fn ensure_cursor_visible(&mut self) {
        let visible = self.visible_lines();
        let doc = &mut self.documents[self.active];
        if doc.cursor_line < doc.scroll_offset {
            doc.scroll_offset = doc.cursor_line;
        } else if doc.cursor_line >= doc.scroll_offset + visible {
            doc.scroll_offset = doc.cursor_line + 1 - visible;
        }
    }

    pub fn center_cursor_in_view(&mut self) {
        let visible = self.visible_lines();
        let doc = &mut self.documents[self.active];
        let half_visible = visible / 2;
        doc.scroll_offset = doc.cursor_line.saturating_sub(half_visible);
    }

    pub fn scroll(&mut self, delta_lines: i32) {
        let doc = &mut self.documents[self.active];
        let max_scroll = doc.lines.len().saturating_sub(1) as i32;
        let new_offset = (doc.scroll_offset as i32 + delta_lines).clamp(0, max_scroll.max(0));
        if new_offset as usize != doc.scroll_offset {
            doc.scroll_offset = new_offset as usize;
            self.dirty = true;
        }
    }

    pub fn zoom(&mut self, delta: f32) {
        self.font_scale = (self.font_scale + delta).clamp(0.5, 3.0);
        self.metrics_dirty = true;
    }

    pub fn zoom_reset(&mut self) {
        self.font_scale = 1.0;
        self.metrics_dirty = true;
    }

    fn apply_zoom_metrics(&mut self) {
        let font_size = self.eff_font_size();
        let line_height = self.eff_line_height();
        let viewport_width = self.viewport_width;
        let viewport_height = self.viewport_height;
        let metrics = Metrics::new(font_size, line_height);

        {
            let active = self.active;
            let font_system = &mut self.font_system;
            let doc = &mut self.documents[active];
            doc.buffer.set_metrics(font_system, metrics);
            doc.buffer.set_size(
                font_system,
                viewport_width - PADDING * 2.0 - GUTTER_WIDTH,
                viewport_height - TOP_OFFSET,
            );
        }

        self.line_numbers_buffer
            .set_metrics(&mut self.font_system, metrics);
        self.line_numbers_buffer.set_size(
            &mut self.font_system,
            GUTTER_WIDTH - 8.0,
            viewport_height - TOP_OFFSET,
        );

        let max_scroll = self.documents[self.active].lines.len().saturating_sub(1);
        let doc = &mut self.documents[self.active];
        doc.scroll_offset = doc.scroll_offset.min(max_scroll);

        self.dirty = true;
    }

    pub fn set_theme(&mut self, theme: crate::settings::ThemeName) {
        self.settings.theme = theme;
        self.settings.save();
        self.dirty = true;
    }

    pub fn cycle_theme(&mut self) {
        use crate::settings::ThemeName;
        let idx = ThemeName::ALL
            .iter()
            .position(|t| *t == self.settings.theme)
            .unwrap_or(0);
        let next = ThemeName::ALL[(idx + 1) % ThemeName::ALL.len()];
        self.set_theme(next);
    }

    pub fn background_color(&self) -> [f32; 3] {
        self.settings.theme.background_color()
    }

    // ---------------------------------------------------------------
    // Barra de herramientas superior estilo Notepad++
    // ---------------------------------------------------------------

    pub fn toolbar_rect(&self) -> (f32, f32, f32, f32) {
        (0.0, 0.0, self.viewport_width, TOOLBAR_HEIGHT)
    }

    pub fn active_menu(&self) -> Option<TopMenu> {
        self.active_menu
    }

    pub fn menu_open(&self) -> bool {
        self.active_menu.is_some()
    }

    pub fn open_menu(&mut self, menu: TopMenu) {
        self.active_menu = Some(menu);
        self.menu_rows = self.build_menu_rows(menu);
        self.rebuild_menu_buffer();
        self.rebuild_toolbar_buffer();
    }

    pub fn close_menu(&mut self) {
        if self.active_menu.is_some() {
            self.active_menu = None;
            self.menu_rows.clear();
            self.rebuild_toolbar_buffer();
        }
    }

    pub fn menu_rect(&self) -> (f32, f32, f32, f32) {
        let menu = match self.active_menu {
            Some(m) => m,
            None => return (0.0, 0.0, 0.0, 0.0),
        };
        let x = match menu {
            TopMenu::Archivo => 8.0,
            TopMenu::Editar => 70.0,
            TopMenu::Buscar => 130.0,
            TopMenu::Ver => 190.0,
            TopMenu::Lenguaje => 235.0_f32.min(self.viewport_width - 250.0).max(8.0),
            TopMenu::Configuracion => 315.0_f32.min(self.viewport_width - 340.0).max(8.0),
        };
        let w = match menu {
            TopMenu::Configuracion => 340.0,
            TopMenu::Lenguaje => 240.0,
            _ => 250.0,
        };
        let h = self.menu_rows.len() as f32 * MENU_ROW_HEIGHT + 12.0;
        (x, TOOLBAR_HEIGHT, w, h)
    }

    pub fn menu_contains(&self, x: f32, y: f32) -> bool {
        if !self.menu_open() {
            return false;
        }
        let (rx, ry, rw, rh) = self.menu_rect();
        x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
    }

    pub fn menu_hit_test(&self, x: f32, y: f32) -> Option<MenuAction> {
        if !self.menu_open() {
            return None;
        }
        let (rx, ry, rw, rh) = self.menu_rect();
        if x < rx || x > rx + rw || y < ry || y > ry + rh {
            return None;
        }
        let rel_y = y - ry - 6.0;
        if rel_y < 0.0 {
            return None;
        }
        let idx = (rel_y / MENU_ROW_HEIGHT) as usize;
        self.menu_rows.get(idx).and_then(|r| r.action)
    }

    pub fn menu_rows_count(&self) -> usize {
        self.menu_rows.len()
    }

    /// Geometría única para la insignia interactiva de lenguaje:
    /// devuelve (chip_x, chip_y, chip_w, chip_h, led_x, led_y, led_w, led_h).
    pub fn lang_badge_geometry(&self) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
        let name = self.active_language().short_name();
        let chars = name.chars().count() + 3; // + "  â–¾"
        let char_w = TOOLBAR_FONT_SIZE * 0.58;
        let text_w = chars as f32 * char_w;
        let bw = (text_w + 38.0).max(96.0);
        let bx = self.viewport_width - 45.0 - bw;
        let by = 3.0;
        let bh = TOOLBAR_HEIGHT - 6.0; // 24.0

        let lx = bx + 10.0;
        let ly = by + (bh - 6.0) / 2.0; // 12.0
        let lw = 6.0;
        let lh = 6.0;

        (bx, by, bw, bh, lx, ly, lw, lh)
    }

    pub fn lang_badge_text_pos(&self) -> (f32, f32) {
        let (bx, _, _, _, _, _, _, _) = self.lang_badge_geometry();
        (bx + 24.0, 0.0)
    }

    pub fn toolbar_hit_test(&self, x: f32, y: f32) -> Option<ToolbarHit> {
        if y < 0.0 || y > TOOLBAR_HEIGHT {
            return None;
        }
        let (bx, by, bw, bh, _, _, _, _) = self.lang_badge_geometry();
        if x >= bx && x <= bx + bw && y >= by && y <= by + bh {
            return Some(ToolbarHit::Menu(TopMenu::Lenguaje));
        }

        for (_, menu, action, x1, x2) in TOOLBAR_ITEMS {
            if x >= x1 && x < x2 {
                if let Some(m) = menu {
                    return Some(ToolbarHit::Menu(m));
                } else if let Some(a) = action {
                    return Some(ToolbarHit::Action(a));
                }
            }
        }
        None
    }

    fn rebuild_toolbar_buffer(&mut self) {
        let base_attrs = Attrs::new().family(Family::Monospace);
        let [tr, tg, tb] = self.settings.color_for(Some("topbar_text"));
        let normal_color = Color::rgb(tr, tg, tb);
        let accent = self.settings.topbar_accent_rgb();
        let active_color = Color::rgb(
            (accent[0] * 255.0) as u8,
            (accent[1] * 255.0) as u8,
            (accent[2] * 255.0) as u8,
        );

        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        spans.push((" Archivo ", base_attrs.color(if self.active_menu == Some(TopMenu::Archivo) { active_color } else { normal_color })));
        spans.push((" Editar ", base_attrs.color(if self.active_menu == Some(TopMenu::Editar) { active_color } else { normal_color })));
        spans.push((" Buscar ", base_attrs.color(if self.active_menu == Some(TopMenu::Buscar) { active_color } else { normal_color })));
        spans.push((" Ver ", base_attrs.color(if self.active_menu == Some(TopMenu::Ver) { active_color } else { normal_color })));
        spans.push((" Lenguaje ", base_attrs.color(if self.active_menu == Some(TopMenu::Lenguaje) { active_color } else { normal_color })));
        spans.push((" Configuración ", base_attrs.color(if self.active_menu == Some(TopMenu::Configuracion) { active_color } else { normal_color })));
        spans.push((" Terminal ", base_attrs.color(normal_color)));

        self.toolbar_buffer.set_rich_text(&mut self.font_system, spans, Shaping::Advanced);
        self.toolbar_buffer.shape_until_scroll(&mut self.font_system);

        let badge_text = format!("{}  â–¾", self.active_language().short_name());
        let (_, _, bw, _, _, _, _, _) = self.lang_badge_geometry();
        self.lang_badge_buffer.set_size(&mut self.font_system, bw, TOOLBAR_HEIGHT);
        self.lang_badge_buffer.set_text(
            &mut self.font_system,
            &badge_text,
            Attrs::new().family(Family::Monospace).color(normal_color),
            Shaping::Advanced,
        );
        self.lang_badge_buffer.shape_until_scroll(&mut self.font_system);
    }

    fn rebuild_menu_buffer(&mut self) {
        let (_rx, _ry, rw, _rh) = self.menu_rect();
        self.menu_buffer.set_size(&mut self.font_system, rw.max(200.0), 800.0);

        let total_chars = ((rw - 24.0) / (13.0 * 0.6)) as usize;
        let mut spans: Vec<(String, Color)> = Vec::new();
        
        let [tr, tg, tb] = self.settings.color_for(Some("topbar_text"));
        let normal_color = Color::rgb(tr, tg, tb);

        for row in &self.menu_rows {
            let color = if row.is_checked {
                Color::rgb(100, 210, 255)
            } else {
                normal_color
            };
            if let Some(sc) = row.shortcut {
                let pad_count = total_chars.saturating_sub(row.label.chars().count() + sc.chars().count());
                spans.push((row.label.clone(), color));
                spans.push((" ".repeat(pad_count.max(2)), Color::rgb(140, 140, 140)));
                spans.push((format!("{sc}\n"), Color::rgb(140, 145, 155)));
            } else {
                spans.push((format!("{}\n", row.label), color));
            }
        }

        let base_attrs = Attrs::new().family(Family::Monospace);
        let rich: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|(s, c)| (s.as_str(), base_attrs.clone().color(*c)))
            .collect();
        self.menu_buffer.set_rich_text(&mut self.font_system, rich, Shaping::Advanced);
        self.menu_buffer.shape_until_scroll(&mut self.font_system);
    }

    fn build_menu_rows(&self, menu: TopMenu) -> Vec<MenuRow> {
        match menu {
            TopMenu::Archivo => vec![
                MenuRow { label: " Nuevo".to_string(), shortcut: Some("Ctrl+N"), action: Some(MenuAction::New), is_checked: false },
                MenuRow { label: " Abrir...".to_string(), shortcut: Some("Ctrl+O"), action: Some(MenuAction::Open), is_checked: false },
                MenuRow { label: " Guardar".to_string(), shortcut: Some("Ctrl+S"), action: Some(MenuAction::Save), is_checked: false },
                MenuRow { label: " Guardar como...".to_string(), shortcut: Some("Ctrl+Shift+S"), action: Some(MenuAction::SaveAs), is_checked: false },
                MenuRow { label: " Guardar todo".to_string(), shortcut: None, action: Some(MenuAction::SaveAll), is_checked: false },
                MenuRow { label: " Cerrar pestaña".to_string(), shortcut: Some("Ctrl+W"), action: Some(MenuAction::CloseTab), is_checked: false },
                MenuRow { label: " Salir".to_string(), shortcut: Some("Alt+F4"), action: Some(MenuAction::Exit), is_checked: false },
            ],
            TopMenu::Editar => vec![
                MenuRow { label: " Deshacer".to_string(), shortcut: Some("Ctrl+Z"), action: Some(MenuAction::Undo), is_checked: false },
                MenuRow { label: " Rehacer".to_string(), shortcut: Some("Ctrl+Y"), action: Some(MenuAction::Redo), is_checked: false },
                MenuRow { label: " Cortar".to_string(), shortcut: Some("Ctrl+X"), action: Some(MenuAction::Cut), is_checked: false },
                MenuRow { label: " Copiar".to_string(), shortcut: Some("Ctrl+C"), action: Some(MenuAction::Copy), is_checked: false },
                MenuRow { label: " Pegar".to_string(), shortcut: Some("Ctrl+V"), action: Some(MenuAction::Paste), is_checked: false },
                MenuRow { label: " Indentar".to_string(), shortcut: Some("Tab"), action: Some(MenuAction::Indent), is_checked: false },
                MenuRow { label: " Desindentar".to_string(), shortcut: Some("Shift+Tab"), action: Some(MenuAction::Unindent), is_checked: false },
                MenuRow { label: " Alternar marcador de línea".to_string(), shortcut: Some("F2"), action: Some(MenuAction::ToggleBookmark), is_checked: false },
                MenuRow { label: " Seleccionar todo".to_string(), shortcut: Some("Ctrl+A"), action: Some(MenuAction::SelectAll), is_checked: false },
            ],
            TopMenu::Buscar => vec![
                MenuRow { label: " Buscar en archivo...".to_string(), shortcut: Some("Ctrl+F"), action: Some(MenuAction::Find), is_checked: false },
                MenuRow { label: " Buscar siguiente".to_string(), shortcut: Some("F3"), action: Some(MenuAction::FindNext), is_checked: false },
            ],
            TopMenu::Ver => vec![
                MenuRow { label: " Acercar zoom".to_string(), shortcut: Some("Ctrl++"), action: Some(MenuAction::ZoomIn), is_checked: false },
                MenuRow { label: " Alejar zoom".to_string(), shortcut: Some("Ctrl+-"), action: Some(MenuAction::ZoomOut), is_checked: false },
                MenuRow { label: " Restablecer zoom".to_string(), shortcut: Some("Ctrl+0"), action: Some(MenuAction::ZoomReset), is_checked: false },
            ],
            TopMenu::Lenguaje => {
                let current = self.active_language();
                Language::ALL_POPULAR
                    .iter()
                    .map(|&lang| {
                        let is_checked = lang == current;
                        let prefix = if is_checked { " âœ“ " } else { "   " };
                        let ext = lang.default_extension();
                        let label = format!("{prefix}{} (.{})", lang.display_name(), ext);
                        MenuRow {
                            label,
                            shortcut: None,
                            action: Some(MenuAction::SetLanguage(lang)),
                            is_checked,
                        }
                    })
                    .collect()
            }
            TopMenu::Configuracion => vec![
                MenuRow {
                    label: if self.settings.rgb_gamer_mode {
                        " 🎮 Modo RGB Gamer [ACTIVADO]".to_string()
                    } else {
                        " 🎮 Modo RGB Gamer [DESACTIVADO]".to_string()
                    },
                    shortcut: Some("F8"),
                    action: Some(MenuAction::ToggleRgbGamer),
                    is_checked: self.settings.rgb_gamer_mode,
                },
                MenuRow { label: " Panel de personalización...".to_string(), shortcut: None, action: Some(MenuAction::OpenPanel), is_checked: false },
                MenuRow { label: " Cambiar fondo (video/imagen)...".to_string(), shortcut: None, action: Some(MenuAction::ChooseBackground), is_checked: false },
                MenuRow { label: " Quitar fondo".to_string(), shortcut: None, action: Some(MenuAction::ClearBackground), is_checked: false },
                MenuRow { label: " Restablecer colores del tema".to_string(), shortcut: None, action: Some(MenuAction::ResetColors), is_checked: false },
                MenuRow { label: " Guardar tema como preset...".to_string(), shortcut: None, action: Some(MenuAction::SavePresetPrompt), is_checked: false },
            ],
        }
    }

    // ---------------------------------------------------------------
    // Panel de personalización
    // ---------------------------------------------------------------

    pub fn panel_open(&self) -> bool {
        self.panel_open
    }

    pub fn toggle_panel(&mut self) {
        self.panel_open = !self.panel_open;
        if self.panel_open {
            self.panel_scroll = 0;
            self.rebuild_panel_buffer();
        }
    }

    pub fn close_panel(&mut self) {
        self.panel_open = false;
    }

    pub fn gear_rect(&self) -> (f32, f32, f32, f32) {
        (self.viewport_width - 40.0, 4.0, 32.0, TOOLBAR_HEIGHT - 8.0)
    }

    pub fn gear_hit(&self, x: f32, y: f32) -> bool {
        let (rx, ry, rw, rh) = self.gear_rect();
        x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
    }

    pub fn panel_visible_rows(&self) -> usize {
        let max_h = (self.viewport_height - TOP_OFFSET - 20.0).max(100.0);
        ((max_h - 16.0) / PANEL_ROW_HEIGHT).floor() as usize
    }

    pub fn panel_rect(&self) -> (f32, f32, f32, f32) {
        let max_h = (self.viewport_height - TOP_OFFSET - 20.0).max(100.0);
        let needed_h = self.panel_rows.len() as f32 * PANEL_ROW_HEIGHT + 16.0;
        let h = needed_h.min(max_h);
        let x = self.viewport_width - PANEL_WIDTH - 12.0;
        let y = TOP_OFFSET + 8.0;
        (x, y, PANEL_WIDTH, h)
    }

    pub fn panel_contains(&self, x: f32, y: f32) -> bool {
        if !self.panel_open {
            return false;
        }
        let (rx, ry, rw, rh) = self.panel_rect();
        x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
    }

    pub fn panel_hit_test(&self, x: f32, y: f32) -> Option<PanelAction> {
        if !self.panel_open {
            return None;
        }
        let (px, py, pw, ph) = self.panel_rect();
        if x < px || x > px + pw || y < py + 8.0 || y > py + ph - 8.0 {
            return None;
        }
        let row_in_view = ((y - py - 8.0) / PANEL_ROW_HEIGHT) as usize;
        let idx = self.panel_scroll + row_in_view;
        self.panel_rows.get(idx).and_then(|row| row.action)
    }

    pub fn panel_hover_row_rect(&self, x: f32, y: f32) -> Option<(f32, f32, f32, f32)> {
        if !self.panel_open {
            return None;
        }
        let (px, py, pw, ph) = self.panel_rect();
        if x < px || x > px + pw || y < py + 8.0 || y > py + ph - 8.0 {
            return None;
        }
        let row_in_view = ((y - py - 8.0) / PANEL_ROW_HEIGHT) as usize;
        let idx = self.panel_scroll + row_in_view;
        if let Some(row) = self.panel_rows.get(idx) {
            if row.action.is_some() {
                let ry = py + 8.0 + row_in_view as f32 * PANEL_ROW_HEIGHT;
                return Some((px + 4.0, ry, pw - 8.0, PANEL_ROW_HEIGHT));
            }
        }
        None
    }

    pub fn scroll_panel(&mut self, delta: i32) {
        if !self.panel_open {
            return;
        }
        let visible = self.panel_visible_rows();
        let max_scroll = self.panel_rows.len().saturating_sub(visible);
        let new_scroll = (self.panel_scroll as i32 + delta).clamp(0, max_scroll as i32);
        if new_scroll as usize != self.panel_scroll {
            self.panel_scroll = new_scroll as usize;
            self.rebuild_panel_buffer();
        }
    }

    pub fn reset_custom_colors(&mut self) {
        self.settings.reset_custom_colors();
        self.settings.save();
        self.dirty = true;
        self.rebuild_panel_buffer();
        self.rebuild_toolbar_buffer();
        self.rebuild_tab_buffer();
    }

    // ---------------------------------------------------------------
    // Cajita de texto genérica (ej. nombrar un preset)
    // ---------------------------------------------------------------

    pub fn prompt_open(&self) -> bool {
        self.prompt_open
    }

    pub fn open_prompt(&mut self, purpose: PromptPurpose, label: &str) {
        self.prompt_open = true;
        if purpose != PromptPurpose::Find {
            self.prompt_text.clear();
        } else if self.prompt_text.is_empty() {
            self.prompt_text = self.last_search_query.clone();
        }
        self.prompt_label = label.to_string();
        self.prompt_purpose = Some(purpose);
        if purpose != PromptPurpose::Find {
            self.prompt_x = None;
            self.prompt_y = None;
        } else if self.prompt_x.is_none() {
            self.prompt_x = Some(self.viewport_width - 340.0 - 24.0);
            self.prompt_y = Some(TOP_OFFSET + 8.0);
        }
        self.rebuild_prompt_buffer();
    }

    pub fn close_prompt(&mut self) {
        self.prompt_open = false;
        self.prompt_purpose = None;
    }

    pub fn prompt_push_char(&mut self, c: char) {
        // Límite razonable para que no se salga de la cajita.
        if self.prompt_text.chars().count() < 40 {
            self.prompt_text.push(c);
            self.rebuild_prompt_buffer();
        }
    }

    pub fn prompt_backspace(&mut self) {
        self.prompt_text.pop();
        self.rebuild_prompt_buffer();
    }

    /// Confirma la cajita (Enter): devuelve para qué se estaba usando y
    /// el texto escrito, y la cierra.
    pub fn prompt_confirm(&mut self) -> Option<(PromptPurpose, String)> {
        let purpose = self.prompt_purpose?;
        let text = self.prompt_text.clone();
        if purpose != PromptPurpose::Find {
            self.close_prompt();
        }
        Some((purpose, text))
    }

    pub fn get_prompt_text(&self) -> &str {
        &self.prompt_text
    }

    /// Busca la siguiente aparición del texto dado (case-insensitive) a partir de la
    /// posición del cursor. Si lo encuentra, mueve el cursor, selecciona la coincidencia
    /// y devuelve true. Si no, devuelve false.
    pub fn find_next(&mut self, query: &str) -> bool {
        if query.is_empty() {
            return false;
        }
        self.last_search_query = query.to_string();
        let doc = &mut self.documents[self.active];
        let total_lines = doc.lines.len();
        if total_lines == 0 {
            return false;
        }

        let query_lower = query.to_lowercase();
        let cur_line = doc.cursor_line;
        let cur_col = doc.cursor_col;

        for offset in 0..total_lines {
            let line_idx = (cur_line + offset) % total_lines;
            let line_text = &doc.lines[line_idx];
            let line_text_lower = line_text.to_lowercase();

            let search_start_byte = if offset == 0 {
                let chars: Vec<char> = line_text.chars().collect();
                if cur_col < chars.len() {
                    chars[..cur_col].iter().map(|c| c.len_utf8()).sum()
                } else {
                    line_text.len()
                }
            } else {
                0
            };

            if search_start_byte < line_text.len() {
                if let Some(rel_byte) = line_text_lower[search_start_byte..].find(&query_lower) {
                    let match_byte = search_start_byte + rel_byte;
                    let match_col = line_text[..match_byte].chars().count();
                    let match_chars_len = query.chars().count();

                    doc.cursor_line = line_idx;
                    doc.cursor_col = match_col + match_chars_len;
                    doc.selection_anchor = Some((line_idx, match_col));
                    self.search_result_marker = Some((line_idx, self.anim_time + 3.0));
                    self.center_cursor_in_view();
                    self.dirty = true;
                    return true;
                }
            }
        }

        // Si no encontró hacia adelante, buscar desde el inicio de la línea actual hasta cur_col
        let line_text = &doc.lines[cur_line];
        let line_text_lower = line_text.to_lowercase();
        if let Some(match_byte) = line_text_lower.find(&query_lower) {
            let match_col = line_text[..match_byte].chars().count();
            let match_chars_len = query.chars().count();
            if match_col < cur_col {
                doc.cursor_line = cur_line;
                doc.cursor_col = match_col + match_chars_len;
                doc.selection_anchor = Some((cur_line, match_col));
                self.search_result_marker = Some((cur_line, self.anim_time + 3.0));
                self.center_cursor_in_view();
                self.dirty = true;
                return true;
            }
        }

        false
    }

    pub fn prompt_rect(&self) -> (f32, f32, f32, f32) {
        let w = 340.0;
        let h = if self.prompt_purpose == Some(PromptPurpose::Find) { 140.0 } else { 110.0 };
        let x = self.prompt_x.unwrap_or_else(|| {
            if self.prompt_purpose == Some(PromptPurpose::Find) {
                self.viewport_width - w - 24.0
            } else {
                (self.viewport_width - w) / 2.0
            }
        });
        let y = self.prompt_y.unwrap_or_else(|| {
            if self.prompt_purpose == Some(PromptPurpose::Find) {
                TOP_OFFSET + 8.0
            } else {
                TOP_OFFSET + 60.0
            }
        });
        (x, y, w, h)
    }

    fn rebuild_prompt_buffer(&mut self) {
        let (text, h) = if self.prompt_purpose == Some(PromptPurpose::Find) {
            (format!("{}\n{}_{}\n\n[ Mostrar siguiente resultado ]", self.prompt_label, self.prompt_text, if self.anim_time % 1.0 < 0.5 { "|" } else { " " }), 140.0)
        } else {
            (format!("{}\n{}_", self.prompt_label, self.prompt_text), 90.0)
        };
        self.prompt_buffer.set_size(&mut self.font_system, 320.0, h);
        self.prompt_buffer.set_text(
            &mut self.font_system,
            &text,
            Attrs::new().family(Family::Monospace).color(Color::rgb(230, 230, 230)),
            Shaping::Advanced,
        );
        self.prompt_buffer.shape_until_scroll(&mut self.font_system);
    }

    pub fn prompt_hit_test(&self, mx: f32, my: f32) -> Option<PromptHit> {
        if !self.prompt_open { return None; }
        let (px, py, pw, ph) = self.prompt_rect();
        if mx >= px && mx <= px + pw && my >= py && my <= py + ph {
            if self.prompt_purpose == Some(PromptPurpose::Find) {
                // Button is near the bottom
                if my > py + ph - 40.0 {
                    return Some(PromptHit::Button);
                }
            }
            // Header/background dragging
            return Some(PromptHit::Drag);
        }
        None
    }

    pub fn load_preset(&mut self, index: usize) {
        self.settings.apply_preset(index);
        self.settings.save();
        self.dirty = true;
        self.rebuild_panel_buffer();
    }

    /// Refresca el contenido del panel (ej. después de guardar un preset
    /// nuevo desde el prompt), solo si está abierto.
    pub fn refresh_panel(&mut self) {
        if self.panel_open {
            self.rebuild_panel_buffer();
        }
    }

    fn build_panel_rows(&self) -> Vec<PanelRow> {
        use crate::settings::ThemeName;

        let header = |text: &str| PanelRow {
            label: text.to_string(),
            action: None,
            swatch: None,
        };
        let item = |text: &str, action: PanelAction| PanelRow {
            label: format!("   {text}"),
            action: Some(action),
            swatch: None,
        };

        let mut rows = vec![header("TEMA")];
        for theme in ThemeName::ALL {
            let mark = if theme == self.settings.theme { "> " } else { "  " };
            rows.push(item(
                &format!("{mark}{}", theme.label()),
                PanelAction::SelectTheme(theme),
            ));
        }

        rows.push(header("FONDO Y OPACIDAD"));
        rows.push(item("Elegir video/imagen...", PanelAction::ChooseBackground));
        rows.push(item("Quitar fondo", PanelAction::ClearBackground));
        let op_pct = (self.settings.editor_opacity * 100.0).round() as u32;
        let op_str = if op_pct >= 99 { "100% (Sólido)".to_string() } else if op_pct == 0 { "0% (Invisible)".to_string() } else { format!("{op_pct}%") };
        rows.push(item(&format!("Opacidad fondo: {op_str}"), PanelAction::CycleEditorOpacity));

        rows.push(header("COLORES"));
        let tokens: [(&str, &'static str); 7] = [
            ("Números de línea", "line_numbers"),
            ("Palabra clave", "keyword"),
            ("Texto/string", "string"),
            ("Comentario", "comment"),
            ("Número", "number"),
            ("Función", "function"),
            ("Tipo", "type"),
        ];
        for (label, name) in tokens {
            let color = self.settings.color_for(Some(name));
            rows.push(PanelRow {
                label: format!("   {label}"),
                action: Some(PanelAction::EditColor(name)),
                swatch: Some(color),
            });
        }

        rows.push(header("MARCADORES DE LÍNEA (Dev-C++)"));
        let mark_color = self.settings.color_for(Some("line_marker"));
        rows.push(PanelRow {
            label: "   Color marcador".to_string(),
            action: Some(PanelAction::EditColor("line_marker")),
            swatch: Some(mark_color),
        });
        let mark_op_pct = (self.settings.marker_opacity * 100.0).round() as u32;
        rows.push(item(&format!("Opacidad marcador: {mark_op_pct}%"), PanelAction::CycleMarkerOpacity));

        rows.push(header("MENÃšS DESPLEGABLES"));
        let menu_color = self.settings.color_for(Some("menu_bg"));
        rows.push(PanelRow {
            label: "   Color fondo menús".to_string(),
            action: Some(PanelAction::EditColor("menu_bg")),
            swatch: Some(menu_color),
        });
        let menu_op_pct = (self.settings.menu_opacity * 100.0).round() as u32;
        let menu_op_str = if menu_op_pct >= 99 { "100% (Sólido)".to_string() } else if menu_op_pct == 0 { "0% (Invisible)".to_string() } else { format!("{menu_op_pct}%") };
        rows.push(item(&format!("Opacidad menús: {menu_op_str}"), PanelAction::CycleMenuOpacity));

        rows.push(header("SCROLLBAR (IZQUIERDA)"));
        let scroll_color = self.settings.color_for(Some("scrollbar"));
        rows.push(PanelRow {
            label: "   Color scrollbar".to_string(),
            action: Some(PanelAction::EditColor("scrollbar")),
            swatch: Some(scroll_color),
        });
        let scroll_op_pct = (self.settings.scrollbar_opacity * 100.0).round() as u32;
        rows.push(item(&format!("Opacidad scrollbar: {scroll_op_pct}%"), PanelAction::CycleScrollbarOpacity));

        rows.push(header("EFECTOS RGB GAMER"));
        let rgb_label = if self.settings.rgb_gamer_mode {
            "🎮 Maestro RGB: [ACTIVADO]"
        } else {
            "🎮 Maestro RGB: [DESACTIVADO]"
        };
        rows.push(item(rgb_label, PanelAction::ToggleRgbGamer));

        for elem in crate::settings::RgbElement::ALL {
            let state_str = if self.settings.is_rgb_element_enabled(elem) { "ON" } else { "OFF" };
            rows.push(item(&format!("  🎮 {}: [{state_str}]", elem.label()), PanelAction::ToggleRgbElement(elem)));
        }

        rows.push(header("BARRA SUPERIOR"));
        let topbar_op_pct = (self.settings.topbar_opacity * 100.0).round() as u32;
        let topbar_op_str = if topbar_op_pct >= 99 { "100% (Sólido)".to_string() } else if topbar_op_pct == 0 { "0% (Invisible)".to_string() } else { format!("{topbar_op_pct}%") };
        rows.push(item(&format!("Opacidad barra superior: {topbar_op_str}"), PanelAction::CycleTopbarOpacity));
        let topbar_tokens: [(&str, &'static str); 4] = [
            ("Acento barra / pestañas", "topbar_accent"),
            ("Fondo barra superior", "topbar_bg"),
            ("Texto de sistema/menús", "topbar_text"),
            ("Pestaña activa", "tab_active"),
        ];
        for (label, name) in topbar_tokens {
            let color = self.settings.color_for(Some(name));
            rows.push(PanelRow {
                label: format!("   {label}"),
                action: Some(PanelAction::EditColor(name)),
                swatch: Some(color),
            });
        }

        rows.push(item("Restablecer colores", PanelAction::ResetColors));
        rows.push(item("Guardar como preset...", PanelAction::SavePresetPrompt));

        if !self.settings.presets.is_empty() {
            rows.push(header("PRESETS GUARDADOS"));
            for (i, preset) in self.settings.presets.iter().enumerate() {
                rows.push(item(&preset.name, PanelAction::LoadPreset(i)));
            }
        }

        rows.push(header("SONIDOS"));
        for (key, label) in SOUND_ACTIONS {
            let mark = if self.settings.sounds.contains_key(key) {
                "\u{2713} "
            } else {
                "  "
            };
            rows.push(item(&format!("{mark}{label}"), PanelAction::AssignSound(key)));
        }
        rows.push(item("Quitar todos los sonidos", PanelAction::ClearSounds));

        rows.push(header(""));
        rows.push(item("Cerrar", PanelAction::Close));

        rows
    }

    // ---------------------------------------------------------------
    // Selector de color
    // ---------------------------------------------------------------

    pub fn picker_open(&self) -> bool {
        self.picker_open
    }

    pub fn picker_hue(&self) -> f32 {
        self.picker_hue
    }

    pub fn picker_sat(&self) -> f32 {
        self.picker_sat
    }

    pub fn picker_val(&self) -> f32 {
        self.picker_val
    }

    pub fn picker_color_rgb(&self) -> [f32; 3] {
        let [r, g, b] = crate::color::hsv_to_rgb(self.picker_hue, self.picker_sat, self.picker_val);
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
    }

    pub fn set_anim_time(&mut self, t: f32) {
        self.anim_time = t;
        if let Some((_, expiration)) = self.search_result_marker {
            if t > expiration {
                self.search_result_marker = None;
                self.dirty = true;
            }
        }
    }

    pub fn toggle_rgb_gamer_mode(&mut self) {
        self.settings.toggle_rgb_gamer_mode();
        self.settings.save();
        self.dirty = true;
        self.rebuild_panel_buffer();
        self.rebuild_toolbar_buffer();
        self.rebuild();
    }

    pub fn toggle_rgb_element(&mut self, elem: crate::settings::RgbElement) {
        self.settings.toggle_rgb_element(elem);
        self.settings.save();
        self.dirty = true;
        self.rebuild_panel_buffer();
        self.rebuild_toolbar_buffer();
        self.rebuild();
    }

    pub fn toggle_bookmark(&mut self, line: usize) {
        let doc = &mut self.documents[self.active];
        if line < doc.lines.len() {
            if doc.bookmarks.contains(&line) {
                doc.bookmarks.remove(&line);
            } else {
                doc.bookmarks.insert(line);
            }
            self.dirty = true;
        }
    }

    pub fn toggle_current_line_bookmark(&mut self) {
        let line = self.documents[self.active].cursor_line;
        self.toggle_bookmark(line);
    }

    #[allow(dead_code)]
    pub fn is_bookmarked(&self, line: usize) -> bool {
        self.documents[self.active].bookmarks.contains(&line)
    }

    pub fn visible_bookmarks(&self) -> Vec<(usize, f32)> {
        let doc = &self.documents[self.active];
        let visible = self.visible_lines();
        let start = doc.scroll_offset;
        let end = (start + visible + 1).min(doc.lines.len());
        let mut res = Vec::new();
        for line in start..end {
            if doc.bookmarks.contains(&line) {
                let visible_idx = line - start;
                let y = TOP_OFFSET + PADDING + visible_idx as f32 * self.eff_line_height();
                res.push((line, y));
            }
        }
        res
    }

    pub fn visible_search_marker(&self) -> Option<(f32, f32)> {
        let (line, expiration) = self.search_result_marker?;
        let doc = &self.documents[self.active];
        let start = doc.scroll_offset;
        let visible = self.visible_lines();
        let end = (start + visible + 1).min(doc.lines.len());
        if line >= start && line < end {
            let visible_idx = line - start;
            let y = TOP_OFFSET + PADDING + visible_idx as f32 * self.eff_line_height();
            let remaining = expiration - self.anim_time;
            let alpha = if remaining < 1.0 { remaining.max(0.0) } else { 1.0 };
            Some((y, alpha))
        } else {
            None
        }
    }

    pub fn scrollbar_rect(&self) -> (f32, f32, f32, f32) {
        (self.viewport_width - SCROLLBAR_WIDTH, TOP_OFFSET, SCROLLBAR_WIDTH, (self.viewport_height - TOP_OFFSET).max(0.0))
    }

    pub fn scrollbar_thumb_rect(&self) -> (f32, f32, f32, f32) {
        let doc = &self.documents[self.active];
        let total_lines = doc.lines.len();
        let visible = self.visible_lines();
        let track_h = (self.viewport_height - TOP_OFFSET).max(10.0);
        let thumb_h = (track_h * (visible as f32 / total_lines.max(1) as f32)).clamp(20.0, track_h);
        let max_scroll = total_lines.saturating_sub(visible);
        let ratio = if max_scroll == 0 {
            0.0
        } else {
            (doc.scroll_offset as f32 / max_scroll as f32).clamp(0.0, 1.0)
        };
        let thumb_y = TOP_OFFSET + (track_h - thumb_h) * ratio;
        (self.viewport_width - SCROLLBAR_WIDTH + 1.0, thumb_y, SCROLLBAR_WIDTH - 2.0, thumb_h)
    }

    pub fn scrollbar_hit_test(&self, x: f32, y: f32) -> bool {
        x >= self.viewport_width - SCROLLBAR_WIDTH && x <= self.viewport_width && y >= TOP_OFFSET && y <= self.viewport_height
    }

    pub fn scrollbar_scroll_to_y(&mut self, y: f32) {
        let track_h = (self.viewport_height - TOP_OFFSET).max(10.0);
        let rel_y = (y - TOP_OFFSET).clamp(0.0, track_h);
        let ratio = rel_y / track_h;
        let visible = self.visible_lines();
        let doc = &mut self.documents[self.active];
        let max_scroll = doc.lines.len().saturating_sub(visible);
        let target_line = (ratio * max_scroll as f32).round() as usize;
        if target_line != doc.scroll_offset {
            doc.scroll_offset = target_line.min(max_scroll);
            self.dirty = true;
        }
    }

    pub fn gutter_hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > (PADDING + GUTTER_WIDTH) || y < TOP_OFFSET + PADDING {
            return None;
        }
        let doc = &self.documents[self.active];
        let rel_y = y - (TOP_OFFSET + PADDING);
        let line_idx = doc.scroll_offset + (rel_y / self.eff_line_height()) as usize;
        if line_idx < doc.lines.len() {
            Some(line_idx)
        } else {
            None
        }
    }

    pub fn open_picker(&mut self, highlight: &'static str) {
        let rgb = self.settings.color_for(Some(highlight));
        let (h, s, v) = crate::color::rgb_to_hsv(rgb);
        self.picker_target = Some(highlight);
        self.picker_hue = h;
        self.picker_sat = s;
        self.picker_val = v;
        self.picker_open = true;
        self.rebuild_picker_buffer();
    }

    pub fn close_picker(&mut self) {
        self.picker_open = false;
        self.picker_target = None;
    }

    pub fn picker_rect(&self) -> (f32, f32, f32, f32) {
        let w = 240.0;
        let h = 250.0;
        let x = self.viewport_width - PANEL_WIDTH - 12.0 - w - 12.0;
        let y = TOP_OFFSET + 8.0;
        (x, y, w, h)
    }

    pub fn picker_sv_rect(&self) -> (f32, f32, f32, f32) {
        let (px, py, pw, _ph) = self.picker_rect();
        (px + 12.0, py + 36.0, pw - 24.0, 140.0)
    }

    pub fn picker_hue_rect(&self) -> (f32, f32, f32, f32) {
        let (px, _py, pw, _ph) = self.picker_rect();
        let (_sx, sy, _sw, sh) = self.picker_sv_rect();
        (px + 12.0, sy + sh + 14.0, pw - 24.0, 22.0)
    }

    pub fn picker_hit_sv(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        if !self.picker_open {
            return None;
        }
        let (rx, ry, rw, rh) = self.picker_sv_rect();
        if x < rx || x > rx + rw || y < ry || y > ry + rh {
            return None;
        }
        let s = ((x - rx) / rw).clamp(0.0, 1.0);
        let v = 1.0 - ((y - ry) / rh).clamp(0.0, 1.0);
        Some((s, v))
    }

    pub fn picker_hit_hue(&self, x: f32, y: f32) -> Option<f32> {
        if !self.picker_open {
            return None;
        }
        let (rx, ry, rw, rh) = self.picker_hue_rect();
        if x < rx || x > rx + rw || y < ry || y > ry + rh {
            return None;
        }
        Some(((x - rx) / rw).clamp(0.0, 1.0) * 360.0)
    }

    pub fn picker_contains(&self, x: f32, y: f32) -> bool {
        if !self.picker_open {
            return false;
        }
        let (rx, ry, rw, rh) = self.picker_rect();
        x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
    }

    pub fn picker_set_sv(&mut self, s: f32, v: f32) {
        self.picker_sat = s;
        self.picker_val = v;
        self.commit_picker_color();
    }

    pub fn picker_set_hue(&mut self, h: f32) {
        self.picker_hue = h;
        self.commit_picker_color();
    }

    fn commit_picker_color(&mut self) {
        if let Some(name) = self.picker_target {
            let rgb = crate::color::hsv_to_rgb(self.picker_hue, self.picker_sat, self.picker_val);
            self.settings.set_custom_color(name, rgb);
            self.settings.save();
            self.dirty = true;
            self.rebuild_panel_buffer();
            self.rebuild_picker_buffer();
            self.rebuild_toolbar_buffer();
            self.rebuild_tab_buffer();
        }
    }

    fn rebuild_picker_buffer(&mut self) {
        let rgb = crate::color::hsv_to_rgb(self.picker_hue, self.picker_sat, self.picker_val);
        let hex = crate::color::rgb_to_hex(rgb);
        let label = self.picker_target.unwrap_or("");
        let text = format!("{label}\n{hex}");
        self.picker_buffer.set_text(
            &mut self.font_system,
            &text,
            Attrs::new().family(Family::Monospace).color(Color::rgb(220, 220, 225)),
            Shaping::Advanced,
        );
        self.picker_buffer.shape_until_scroll(&mut self.font_system);
    }

    pub fn rebuild_panel_buffer(&mut self) {
        self.panel_rows = self.build_panel_rows();

        let visible = self.panel_visible_rows();
        let max_scroll = self.panel_rows.len().saturating_sub(visible);
        if self.panel_scroll > max_scroll {
            self.panel_scroll = max_scroll;
        }

        let start = self.panel_scroll;
        let end = (start + visible + 1).min(self.panel_rows.len());

        let base_attrs = Attrs::new().family(Family::Monospace);
        let header_color = Color::rgb(140, 140, 150);
        let [tr, tg, tb] = self.settings.color_for(Some("topbar_text"));
        let normal_color = Color::rgb(tr, tg, tb);

        let mut spans: Vec<(String, Color)> = Vec::new();
        for (i, row) in self.panel_rows[start..end].iter().enumerate() {
            let color = if row.action.is_none() { header_color } else { normal_color };
            spans.push((row.label.clone(), color));
            if let Some([r, g, b]) = row.swatch {
                spans.push((" \u{25A0}".to_string(), Color::rgb(r, g, b)));
            }
            if i + 1 < (end - start) {
                spans.push(("\n".to_string(), normal_color));
            }
        }

        let rich_spans: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|(s, c)| (s.as_str(), base_attrs.clone().color(*c)))
            .collect();

        self.panel_buffer.set_wrap(&mut self.font_system, Wrap::None);
        let (_px, _py, pw, ph) = self.panel_rect();
        self.panel_buffer.set_size(&mut self.font_system, pw - 16.0, ph);
        self.panel_buffer
            .set_rich_text(&mut self.font_system, rich_spans, Shaping::Advanced);
        self.panel_buffer.shape_until_scroll(&mut self.font_system);
    }

    // ---------------------------------------------------------------
    // Contenido del documento activo
    // ---------------------------------------------------------------

    #[allow(dead_code)]
    pub fn load_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n");
        let doc = &mut self.documents[self.active];
        doc.lines = if normalized.is_empty() {
            vec![String::new()]
        } else {
            normalized.split('\n').map(String::from).collect()
        };
        doc.cursor_line = 0;
        doc.cursor_col = 0;
        doc.scroll_offset = 0;
        doc.selection_anchor = None;
        self.hide_completions();
        self.dirty = true;
    }

    pub fn cursor_position(&self) -> (usize, usize) {
        let doc = &self.documents[self.active];
        (doc.cursor_line, doc.cursor_col)
    }

    pub fn full_text(&self) -> String {
        self.documents[self.active].lines.join("\n")
    }

    pub fn completion_visible(&self) -> bool {
        self.completion_visible
    }

    pub fn show_completions(&mut self, items: &[crate::lsp::CompletionItem], selected: usize) {
        if items.is_empty() {
            self.hide_completions();
            return;
        }
        self.completion_items = items.to_vec();
        self.completion_selected = selected.min(self.completion_items.len() - 1);
        self.completion_visible = true;
        self.rebuild_completion_buffer();
    }

    pub fn hide_completions(&mut self) {
        self.completion_visible = false;
        self.completion_items.clear();
    }

    pub fn completion_move_selection(&mut self, delta: i32) {
        if self.completion_items.is_empty() {
            return;
        }
        let len = self.completion_items.len() as i32;
        let idx = (self.completion_selected as i32 + delta).clamp(0, len - 1);
        self.completion_selected = idx as usize;
        self.rebuild_completion_buffer();
    }

    pub fn completion_accept(&mut self) {
        if let Some(item) = self.completion_items.get(self.completion_selected).cloned() {
            for c in item.insert_text.chars() {
                self.insert_char(c);
            }
        }
        self.hide_completions();
    }

    fn rebuild_completion_buffer(&mut self) {
        let base_attrs = Attrs::new().family(Family::Monospace);
        let lines: Vec<String> = self
            .completion_items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let marker = if i == self.completion_selected { "> " } else { "  " };
                format!("{marker}{}", item.label)
            })
            .collect();
        let text = lines.join("\n");

        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        let mut offset = 0usize;
        for (i, line) in lines.iter().enumerate() {
            let end = offset + line.len();
            let color = if i == self.completion_selected {
                Color::rgb(255, 255, 255)
            } else {
                Color::rgb(160, 160, 170)
            };
            spans.push((&text[offset..end], base_attrs.clone().color(color)));
            offset = end;
            if i + 1 < lines.len() {
                spans.push(("\n", base_attrs.clone()));
                offset += 1;
            }
        }

        self.completion_buffer
            .set_rich_text(&mut self.font_system, spans, Shaping::Advanced);
        self.completion_buffer.shape_until_scroll(&mut self.font_system);
    }

    pub fn set_cursor_blink(&mut self, visible: bool) {
        self.cursor_blink_on = visible;
    }

    fn cursor_pixel_pos(&self) -> (f32, f32) {
        let doc = &self.documents[self.active];
        let approx_char_width = self.eff_font_size() * 0.6;
        let x = PADDING + GUTTER_WIDTH + doc.cursor_col as f32 * approx_char_width;
        let visible_line = doc.cursor_line.saturating_sub(doc.scroll_offset);
        let y = TOP_OFFSET + PADDING + visible_line as f32 * self.eff_line_height();
        (x, y)
    }

    /// Rectángulo (x, y, ancho, alto) del cursor visual: una barrita
    /// delgada tipo Word/Notepad, no un bloque grueso tipo terminal.
    pub fn cursor_rect(&self) -> (f32, f32, f32, f32) {
        let (x, y) = self.cursor_pixel_pos();
        (x, y, 2.0, self.eff_line_height() * 0.85)
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_blink_on
    }

    /// Color del cursor: el mismo color de texto "normal" del tema
    /// activo, para que siempre contraste bien con el fondo.
    pub fn cursor_color(&self) -> [f32; 3] {
        if self.settings.is_rgb_active(crate::settings::RgbElement::Cursor) {
            crate::color::chroma_rgb_f32(self.anim_time, 24.0)
        } else {
            let [r, g, b] = self.settings.theme.default_text_color();
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
        }
    }

    fn completion_anchor_px(&self) -> (f32, f32) {
        let doc = &self.documents[self.active];
        let approx_char_width = self.eff_font_size() * 0.6;
        let x = PADDING + GUTTER_WIDTH + doc.cursor_col as f32 * approx_char_width;
        let visible_line = doc.cursor_line.saturating_sub(doc.scroll_offset);
        let y = TOP_OFFSET + PADDING + (visible_line as f32 + 1.0) * self.eff_line_height();
        (x, y)
    }

    // ---------------------------------------------------------------
    // Deshacer / rehacer
    // ---------------------------------------------------------------

    /// Guarda una "foto" del estado actual ANTES de aplicar una edición,
    /// pero solo si hace falta: pulsaciones seguidas del mismo tipo (ej.
    /// escribir varias letras seguidas) se agrupan en un solo paso.
    fn push_undo_if_needed(&mut self, kind: EditKind) {
        if self.paste_in_progress {
            // Mientras se pega texto, ya guardamos UNA sola foto al
            // principio (en paste_text); no queremos una por cada letra.
            return;
        }
        let active = self.active;
        let should_push = {
            let doc = &self.documents[active];
            kind == EditKind::Other || doc.last_edit_kind != kind
        };
        if should_push {
            let doc = &mut self.documents[active];
            doc.undo_stack.push(UndoSnapshot {
                lines: doc.lines.clone(),
                cursor_line: doc.cursor_line,
                cursor_col: doc.cursor_col,
            });
            if doc.undo_stack.len() > 200 {
                doc.undo_stack.remove(0);
            }
            doc.redo_stack.clear();
        }
        self.documents[active].last_edit_kind = kind;
    }

    pub fn undo(&mut self) {
        let active = self.active;
        let Some(prev) = self.documents[active].undo_stack.pop() else {
            return;
        };
        let doc = &mut self.documents[active];
        doc.redo_stack.push(UndoSnapshot {
            lines: doc.lines.clone(),
            cursor_line: doc.cursor_line,
            cursor_col: doc.cursor_col,
        });
        doc.lines = prev.lines;
        doc.cursor_line = prev.cursor_line.min(doc.lines.len().saturating_sub(1));
        doc.cursor_col = prev.cursor_col;
        doc.selection_anchor = None;
        doc.last_edit_kind = EditKind::None;
        doc.dirty = true;
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    pub fn redo(&mut self) {
        let active = self.active;
        let Some(next) = self.documents[active].redo_stack.pop() else {
            return;
        };
        let doc = &mut self.documents[active];
        doc.undo_stack.push(UndoSnapshot {
            lines: doc.lines.clone(),
            cursor_line: doc.cursor_line,
            cursor_col: doc.cursor_col,
        });
        doc.lines = next.lines;
        doc.cursor_line = next.cursor_line.min(doc.lines.len().saturating_sub(1));
        doc.cursor_col = next.cursor_col;
        doc.selection_anchor = None;
        doc.last_edit_kind = EditKind::None;
        doc.dirty = true;
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    pub fn insert_char(&mut self, c: char) {
        if self.documents[self.active].selection_anchor.is_some() {
            self.delete_selection();
        }
        self.push_undo_if_needed(EditKind::Insert);
        let doc = &mut self.documents[self.active];
        let line = &mut doc.lines[doc.cursor_line];
        let mut chars: Vec<char> = line.chars().collect();
        let at = doc.cursor_col.min(chars.len());
        chars.insert(at, c);
        *line = chars.into_iter().collect();
        doc.cursor_col = at + 1;
        doc.dirty = true;
        self.dirty = true;
    }

    pub fn insert_newline(&mut self) {
        if self.documents[self.active].selection_anchor.is_some() {
            self.delete_selection();
        }
        self.push_undo_if_needed(EditKind::Other);
        let doc = &mut self.documents[self.active];
        let chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
        let at = doc.cursor_col.min(chars.len());
        let before: String = chars[..at].iter().collect();
        let after: String = chars[at..].iter().collect();

        // Extraer la sangría previa (espacios y tabulaciones)
        let mut indent = String::new();
        for c in before.chars() {
            if c == ' ' || c == '\t' {
                indent.push(c);
            } else {
                break;
            }
        }

        // Si la línea antes del cursor termina con un delimitador de apertura, aumentar un nivel (4 espacios)
        let trimmed_before = before.trim_end();
        if trimmed_before.ends_with('{')
            || trimmed_before.ends_with(':')
            || trimmed_before.ends_with('[')
            || trimmed_before.ends_with('(')
        {
            indent.push_str("    ");
        }

        let new_line = format!("{}{}", indent, after);
        let new_col = indent.chars().count();

        doc.lines[doc.cursor_line] = before;
        doc.lines.insert(doc.cursor_line + 1, new_line);
        doc.cursor_line += 1;
        doc.cursor_col = new_col;
        doc.dirty = true;
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    /// Indenta la línea o selección actual con 4 espacios (Tab).
    pub fn indent(&mut self) {
        self.push_undo_if_needed(EditKind::Other);
        let sel = self.selection_range();
        let doc = &mut self.documents[self.active];
        if let Some((start, end)) = sel {
            let start_line = start.0;
            let end_line = if end.1 == 0 && end.0 > start.0 {
                end.0 - 1
            } else {
                end.0
            };
            for line_idx in start_line..=end_line {
                if line_idx < doc.lines.len() {
                    doc.lines[line_idx] = format!("    {}", doc.lines[line_idx]);
                }
            }
            if let Some((anchor_line, anchor_col)) = doc.selection_anchor.as_mut() {
                if *anchor_line >= start_line && *anchor_line <= end_line {
                    *anchor_col += 4;
                }
            }
            if doc.cursor_line >= start_line && doc.cursor_line <= end_line {
                doc.cursor_col += 4;
            }
            doc.dirty = true;
            self.dirty = true;
        } else {
            let line = &mut doc.lines[doc.cursor_line];
            let mut chars: Vec<char> = line.chars().collect();
            let at = doc.cursor_col.min(chars.len());
            for _ in 0..4 {
                chars.insert(at, ' ');
            }
            *line = chars.into_iter().collect();
            doc.cursor_col = at + 4;
            doc.dirty = true;
            self.dirty = true;
        }
    }

    /// Desindenta la línea o selección actual (Shift+Tab) removiendo hasta 4 espacios iniciales.
    pub fn unindent(&mut self) {
        self.push_undo_if_needed(EditKind::Other);
        let sel = self.selection_range();
        let doc = &mut self.documents[self.active];
        if let Some((start, end)) = sel {
            let start_line = start.0;
            let end_line = if end.1 == 0 && end.0 > start.0 {
                end.0 - 1
            } else {
                end.0
            };
            let mut cursor_shift = 0;
            let mut anchor_shift = 0;
            for line_idx in start_line..=end_line {
                if line_idx < doc.lines.len() {
                    let line = &doc.lines[line_idx];
                    let to_remove = if line.starts_with('\t') {
                        1
                    } else {
                        line.chars().take(4).take_while(|&c| c == ' ').count()
                    };
                    if to_remove > 0 {
                        doc.lines[line_idx] = line[to_remove..].to_string();
                        if line_idx == doc.cursor_line {
                            cursor_shift = to_remove;
                        }
                        if let Some((anchor_line, _)) = doc.selection_anchor {
                            if line_idx == anchor_line {
                                anchor_shift = to_remove;
                            }
                        }
                    }
                }
            }
            doc.cursor_col = doc.cursor_col.saturating_sub(cursor_shift);
            if let Some((_, anchor_col)) = doc.selection_anchor.as_mut() {
                *anchor_col = anchor_col.saturating_sub(anchor_shift);
            }
            doc.dirty = true;
            self.dirty = true;
        } else {
            let line = &doc.lines[doc.cursor_line];
            let to_remove = if line.starts_with('\t') {
                1
            } else {
                line.chars().take(4).take_while(|&c| c == ' ').count()
            };
            if to_remove > 0 {
                doc.lines[doc.cursor_line] = line[to_remove..].to_string();
                doc.cursor_col = doc.cursor_col.saturating_sub(to_remove);
                doc.dirty = true;
                self.dirty = true;
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.documents[self.active].selection_anchor.is_some() {
            self.delete_selection();
            return;
        }
        self.push_undo_if_needed(EditKind::Delete);
        let doc = &mut self.documents[self.active];
        if doc.cursor_col > 0 {
            let line = &mut doc.lines[doc.cursor_line];
            let mut chars: Vec<char> = line.chars().collect();
            chars.remove(doc.cursor_col - 1);
            *line = chars.into_iter().collect();
            doc.cursor_col -= 1;
        } else if doc.cursor_line > 0 {
            let current_line = doc.lines.remove(doc.cursor_line);
            doc.cursor_line -= 1;
            doc.cursor_col = doc.lines[doc.cursor_line].chars().count();
            doc.lines[doc.cursor_line].push_str(&current_line);
        }
        doc.dirty = true;
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    /// Como `backspace`, pero borra el carácter DESPUÃ‰S del cursor (la
    /// tecla "Supr"/"Delete", no "Backspace").
    pub fn delete_forward(&mut self) {
        if self.documents[self.active].selection_anchor.is_some() {
            self.delete_selection();
            return;
        }
        self.push_undo_if_needed(EditKind::Delete);
        let doc = &mut self.documents[self.active];
        let line_len = doc.lines[doc.cursor_line].chars().count();
        if doc.cursor_col < line_len {
            let mut chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
            chars.remove(doc.cursor_col);
            doc.lines[doc.cursor_line] = chars.into_iter().collect();
        } else if doc.cursor_line + 1 < doc.lines.len() {
            let next_line = doc.lines.remove(doc.cursor_line + 1);
            doc.lines[doc.cursor_line].push_str(&next_line);
        }
        doc.dirty = true;
        self.dirty = true;
    }

    // ---------------------------------------------------------------
    // Selección, clic del mouse y portapapeles
    // ---------------------------------------------------------------

    /// Â¿El punto (x, y) cae dentro del área de texto (no en la barra de
    /// herramientas, pestañas, panel abierto o selector abierto)?
    pub fn is_in_text_area(&self, x: f32, y: f32) -> bool {
        y >= TOP_OFFSET
            && !self.panel_contains(x, y)
            && !self.picker_contains(x, y)
            && !self.menu_contains(x, y)
    }

    /// Convierte una posición en píxeles a (línea, columna) del
    /// documento activo, usando el mismo supuesto de ancho monoespaciado
    /// que el resto del editor.
    fn cursor_from_click(&self, x: f32, y: f32) -> (usize, usize) {
        let doc = &self.documents[self.active];
        let char_width = self.eff_font_size() * 0.6;

        let rel_y = (y - TOP_OFFSET - PADDING).max(0.0);
        let line_in_view = (rel_y / self.eff_line_height()) as usize;
        let line = (doc.scroll_offset + line_in_view).min(doc.lines.len().saturating_sub(1));

        let rel_x = (x - PADDING - GUTTER_WIDTH).max(0.0);
        let line_len = doc.lines[line].chars().count();
        let col = ((rel_x / char_width).round() as usize).min(line_len);

        (line, col)
    }

    /// Clic simple: coloca el cursor ahí y arranca una selección nueva
    /// (que quedará vacía si el usuario no arrastra el mouse).
    pub fn click_place_cursor(&mut self, x: f32, y: f32) {
        let (line, col) = self.cursor_from_click(x, y);
        let doc = &mut self.documents[self.active];
        doc.cursor_line = line;
        doc.cursor_col = col;
        doc.selection_anchor = Some((line, col));
        doc.last_edit_kind = EditKind::None;
        self.dirty = true;
    }

    pub fn goto_definition_under_cursor(&mut self, mouse_x: f32, mouse_y: f32) {
        let doc = &mut self.documents[self.active];
        let line = doc.cursor_line;
        let col = doc.cursor_col;
        if line >= doc.lines.len() { return; }
        let text = &doc.lines[line];

        let mut start = col;
        let mut end = col;
        let chars: Vec<char> = text.chars().collect();
        if col >= chars.len() { return; }

        if !chars[col].is_alphanumeric() && chars[col] != '_' {
            return;
        }

        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }

        let word: String = chars[start..end].iter().collect();
        if word.is_empty() { return; }

        let patterns = [
            format!("class {word}"),
            format!("struct {word}"),
            format!("fn {word}"),
            format!("def {word}"),
            format!("let {word}"),
            format!("let mut {word}"),
            format!("var {word}"),
            format!("const {word}"),
            format!("enum {word}"),
            format!("function {word}"),
            format!("{word}: "),
            format!("{word} = "),
        ];

        let mut found_line = None;
        let mut found_col = None;
        for (i, l) in doc.lines.iter().enumerate() {
            for p in &patterns {
                if let Some(idx) = l.find(p) {
                    found_line = Some(i);
                    let pre_chars = l[..idx].chars().count();
                    found_col = Some(pre_chars);
                    break;
                }
            }
            if found_line.is_some() { break; }
        }

        if let (Some(fl), Some(fc)) = (found_line, found_col) {
            doc.cursor_line = fl;
            doc.cursor_col = fc;
            doc.selection_anchor = Some((fl, fc));
            self.ensure_cursor_visible();
            self.dirty = true;
        } else {
            let msg = format!("No se encontró la declaración de '{}'", word);
            self.tooltip_text = Some(msg.clone());
            self.tooltip_expiration = self.anim_time + 2.0;
            self.tooltip_pos = (mouse_x, mouse_y - 30.0);
            
            self.tooltip_buffer.set_text(
                &mut self.font_system,
                &msg,
                Attrs::new().family(Family::SansSerif).color(Color::rgb(255, 255, 255)),
                Shaping::Advanced,
            );
            self.tooltip_buffer.set_size(&mut self.font_system, 400.0, 50.0);
            self.tooltip_buffer.shape_until_scroll(&mut self.font_system);
        }
    }

    /// Mientras se arrastra el mouse con el botón izquierdo presionado:
    /// mueve el cursor sin tocar el ancla de la selección.
    pub fn drag_extend_selection(&mut self, x: f32, y: f32) {
        let (line, col) = self.cursor_from_click(x, y);
        let doc = &mut self.documents[self.active];
        doc.cursor_line = line;
        doc.cursor_col = col;
        self.dirty = true;
    }

    pub fn select_all(&mut self) {
        let doc = &mut self.documents[self.active];
        doc.selection_anchor = Some((0, 0));
        doc.cursor_line = doc.lines.len().saturating_sub(1);
        doc.cursor_col = doc.lines[doc.cursor_line].chars().count();
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    /// Rango normalizado (inicio <= fin) de la selección actual, o
    /// `None` si no hay ninguna (o si el ancla y el cursor coinciden).
    fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let doc = &self.documents[self.active];
        let anchor = doc.selection_anchor?;
        let cursor = (doc.cursor_line, doc.cursor_col);
        if anchor == cursor {
            return None;
        }
        Some(if anchor <= cursor { (anchor, cursor) } else { (cursor, anchor) })
    }

    /// Rectángulos (en píxeles) a resaltar para mostrar la selección
    /// actual, uno por cada línea visible que toque.
    pub fn selection_rects(&self) -> Vec<(f32, f32, f32, f32)> {
        let Some((start, end)) = self.selection_range() else {
            return Vec::new();
        };
        let doc = &self.documents[self.active];
        let char_width = self.eff_font_size() * 0.6;
        let line_height = self.eff_line_height();

        let mut rects = Vec::new();
        for line in start.0..=end.0 {
            if line < doc.scroll_offset {
                continue;
            }
            let visible_line = line - doc.scroll_offset;
            let y = TOP_OFFSET + PADDING + visible_line as f32 * line_height;
            if y > self.viewport_height {
                break;
            }
            let line_len = doc.lines.get(line).map(|l| l.chars().count()).unwrap_or(0);
            let col_start = if line == start.0 { start.1 } else { 0 };
            let col_end = if line == end.0 { end.1 } else { line_len };
            let x = PADDING + GUTTER_WIDTH + col_start as f32 * char_width;
            let w = (col_end.saturating_sub(col_start)) as f32 * char_width;
            let w = if w > 0.0 { w } else { char_width * 0.4 };
            rects.push((x, y, w, line_height));
        }
        rects
    }

    /// El texto actualmente seleccionado, si hay alguno.
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        let doc = &self.documents[self.active];

        if start.0 == end.0 {
            let chars: Vec<char> = doc.lines[start.0].chars().collect();
            let s = start.1.min(chars.len());
            let e = end.1.min(chars.len());
            return Some(chars[s..e].iter().collect());
        }

        let mut out = String::new();
        let first_chars: Vec<char> = doc.lines[start.0].chars().collect();
        let s = start.1.min(first_chars.len());
        out.extend(first_chars[s..].iter());
        out.push('\n');
        for line in (start.0 + 1)..end.0 {
            out.push_str(&doc.lines[line]);
            out.push('\n');
        }
        let last_chars: Vec<char> = doc.lines[end.0].chars().collect();
        let e = end.1.min(last_chars.len());
        out.extend(last_chars[..e].iter());
        Some(out)
    }

    /// Borra el texto seleccionado y deja el cursor donde empezaba.
    pub fn delete_selection(&mut self) {
        let Some((start, end)) = self.selection_range() else {
            self.documents[self.active].selection_anchor = None;
            return;
        };
        self.push_undo_if_needed(EditKind::Other);
        let doc = &mut self.documents[self.active];

        if start.0 == end.0 {
            let chars: Vec<char> = doc.lines[start.0].chars().collect();
            let s = start.1.min(chars.len());
            let e = end.1.min(chars.len());
            let mut new_line = String::new();
            new_line.extend(chars[..s].iter());
            new_line.extend(chars[e..].iter());
            doc.lines[start.0] = new_line;
        } else {
            let first_chars: Vec<char> = doc.lines[start.0].chars().collect();
            let last_chars: Vec<char> = doc.lines[end.0].chars().collect();
            let s = start.1.min(first_chars.len());
            let e = end.1.min(last_chars.len());
            let mut merged = String::new();
            merged.extend(first_chars[..s].iter());
            merged.extend(last_chars[e..].iter());
            doc.lines.splice(start.0..=end.0, std::iter::once(merged));
        }

        doc.cursor_line = start.0;
        doc.cursor_col = start.1;
        doc.selection_anchor = None;
        doc.dirty = true;
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    /// Inserta texto (potencialmente con saltos de línea) en la posición
    /// del cursor, como al pegar del portapapeles.
    /// Inserta texto (potencialmente con saltos de línea) en la posición
    /// del cursor, como al pegar del portapapeles. Cuenta como UN SOLO
    /// paso de deshacer, sin importar qué tan largo sea el texto pegado.
    pub fn paste_text(&mut self, text: &str) {
        if self.documents[self.active].selection_anchor.is_some() {
            self.delete_selection();
        }
        // Una sola foto para todo el pegado (antes de que empiece).
        self.push_undo_if_needed(EditKind::Other);

        self.paste_in_progress = true;
        for c in text.replace("\r\n", "\n").chars() {
            if c == '\n' {
                self.insert_newline();
            } else {
                self.insert_char(c);
            }
        }
        self.paste_in_progress = false;
    }

    pub fn move_cursor(&mut self, dx: i32, dy: i32, extend: bool) {
        let doc = &mut self.documents[self.active];
        doc.last_edit_kind = EditKind::None;

        if extend {
            if doc.selection_anchor.is_none() {
                doc.selection_anchor = Some((doc.cursor_line, doc.cursor_col));
            }
        } else {
            doc.selection_anchor = None;
        }

        if dy > 0 && doc.cursor_line + 1 < doc.lines.len() {
            doc.cursor_line += 1;
            doc.cursor_col = doc.cursor_col.min(doc.lines[doc.cursor_line].chars().count());
        } else if dy < 0 && doc.cursor_line > 0 {
            doc.cursor_line -= 1;
            doc.cursor_col = doc.cursor_col.min(doc.lines[doc.cursor_line].chars().count());
        }

        if dx > 0 {
            let len = doc.lines[doc.cursor_line].chars().count();
            if doc.cursor_col < len {
                doc.cursor_col += 1;
            } else if doc.cursor_line + 1 < doc.lines.len() {
                doc.cursor_line += 1;
                doc.cursor_col = 0;
            }
        } else if dx < 0 {
            if doc.cursor_col > 0 {
                doc.cursor_col -= 1;
            } else if doc.cursor_line > 0 {
                doc.cursor_line -= 1;
                doc.cursor_col = doc.lines[doc.cursor_line].chars().count();
            }
        }

        self.ensure_cursor_visible();
        self.dirty = true;
    }

    fn rebuild(&mut self) {
        let active = self.active;
        let full_text = self.documents[active].lines.join("\n");

        if full_text != self.documents[active].cached_full_text {
            let language = self.documents[active].language;
            let highlighter = self
                .syntax_cache
                .entry(language)
                .or_insert_with(|| SyntaxHighlighter::new(language));
            let spans = highlighter.highlight(&full_text);
            let doc = &mut self.documents[active];
            doc.cached_spans = spans;
            doc.cached_full_text = full_text.clone();
        }

        let visible = self.visible_lines();
        let doc = &self.documents[active];
        let start_line = doc.scroll_offset.min(doc.lines.len().saturating_sub(1));
        let end_line = (start_line + visible).min(doc.lines.len());
        let start_byte: usize = doc.lines[..start_line].iter().map(|l| l.len() + 1).sum();
        let visible_len: usize = doc.lines[start_line..end_line]
            .iter()
            .map(|l| l.len() + 1)
            .sum::<usize>()
            .saturating_sub(1);
        let end_byte = (start_byte + visible_len).min(full_text.len());
        let spans_full = doc.cached_spans.clone();

        let base_attrs = Attrs::new().family(Family::Monospace);
        let mut rich_spans: Vec<(&str, Attrs)> = Vec::new();
        let is_gamer = self.settings.is_rgb_active(crate::settings::RgbElement::Syntax);
        for (range, name) in &spans_full {
            let s = range.start.max(start_byte);
            let e = range.end.min(end_byte);
            if s >= e {
                continue;
            }
            let piece = &full_text[s..e];
            let [r, g, b] = if is_gamer && name.is_some() {
                crate::color::chroma_rgb(self.anim_time, s as f32)
            } else {
                self.settings.color_for(*name)
            };
            let attrs = base_attrs.clone().color(Color::rgb(r, g, b));
            rich_spans.push((piece, attrs));
        }

        let font_system = &mut self.font_system;
        let doc = &mut self.documents[active];
        doc.buffer.set_rich_text(font_system, rich_spans, Shaping::Advanced);
        doc.buffer.shape_until_scroll(font_system);

        self.rebuild_line_numbers();
    }

    /// Vuelve a escribir la columna de números de línea (uno por cada
    /// línea de código que esté actualmente visible), alineados a la
    /// derecha, con el mismo alto de línea que el código para que
    /// coincidan exactamente.
    fn rebuild_line_numbers(&mut self) {
        let doc = &self.documents[self.active];
        let visible = self.visible_lines();
        let start = doc.scroll_offset;
        let end = (start + visible + 1).min(doc.lines.len());

        let base_attrs = Attrs::new().family(Family::Monospace);
        let ln_rgb = self.settings.color_for(Some("line_numbers"));
        let normal_color = Color::rgb(ln_rgb[0], ln_rgb[1], ln_rgb[2]);
        let is_rgb = self.settings.is_rgb_active(crate::settings::RgbElement::LineNumbers);
        let active_color = if is_rgb {
            let [r, g, b] = crate::color::chroma_rgb_f32(self.anim_time, 0.0);
            Color::rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
        } else {
            match self.settings.theme {
                crate::settings::ThemeName::Light => Color::rgb(20, 20, 25),
                _ => Color::rgb(245, 245, 255),
            }
        };

        let mut lines_data: Vec<(String, Attrs)> = Vec::new();
        for i in start..end {
            let num_str = format!("{:>4}
", i + 1);
            let color = if is_rgb {
                let [r, g, b] = crate::color::chroma_rgb_f32(self.anim_time, i as f32 * 3.0);
                if i == doc.cursor_line {
                    Color::rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
                } else {
                    Color::rgb((r * 150.0) as u8, (g * 150.0) as u8, (b * 150.0) as u8)
                }
            } else if i == doc.cursor_line {
                active_color
            } else {
                normal_color
            };
            lines_data.push((num_str, base_attrs.clone().color(color)));
        }

        let rich_spans: Vec<(&str, Attrs)> = lines_data
            .iter()
            .map(|(s, a)| (s.as_str(), a.clone()))
            .collect();

        self.line_numbers_buffer
            .set_rich_text(&mut self.font_system, rich_spans, Shaping::Advanced);
        self.line_numbers_buffer.shape_until_scroll(&mut self.font_system);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.viewport_width = width as f32;
        self.viewport_height = height as f32;
        let w = width as f32 - PADDING * 2.0 - GUTTER_WIDTH;
        let h = height as f32 - TOP_OFFSET;
        let doc = &mut self.documents[self.active];
        doc.buffer.set_size(&mut self.font_system, w, h);
        self.toolbar_buffer
            .set_size(&mut self.font_system, width as f32, TOOLBAR_HEIGHT);
        self.tab_buffer
            .set_size(&mut self.font_system, width as f32, TAB_HEIGHT);
        self.line_numbers_buffer
            .set_size(&mut self.font_system, GUTTER_WIDTH - 8.0, h);
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    pub fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        width: u32,
        height: u32,
    ) -> Result<(), glyphon::PrepareError> {
        if self.metrics_dirty {
            self.apply_zoom_metrics();
            self.metrics_dirty = false;
        }
        if self.dirty || self.settings.rgb_gamer_mode {
            self.rebuild();
            self.dirty = false;
        }

        let [dr, dg, db] = self.settings.theme.default_text_color();
        let mut overlay_areas = Vec::new();
        let mut base_areas = vec![
            TextArea {
                buffer: &self.line_numbers_buffer,
                left: 4.0,
                top: TOP_OFFSET + PADDING,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: TOP_OFFSET as i32,
                    right: (PADDING + GUTTER_WIDTH - 8.0) as i32,
                    bottom: height as i32,
                },
                default_color: Color::rgb(110, 114, 128),
            },
            TextArea {
            buffer: &self.documents[self.active].buffer,
            left: PADDING + GUTTER_WIDTH,
            top: TOP_OFFSET + PADDING,
            scale: 1.0,
            bounds: TextBounds {
                left: (PADDING + GUTTER_WIDTH - 4.0) as i32,
                top: TOP_OFFSET as i32,
                right: width as i32,
                bottom: height as i32,
            },
            default_color: Color::rgb(dr, dg, db),
            },
        ];

        if self.completion_visible {
            let (x, y) = self.completion_anchor_px();
            base_areas.push(TextArea {
                buffer: &self.completion_buffer,
                left: x,
                top: y,
                scale: 1.0,
                bounds: TextBounds {
                    left: x as i32,
                    top: y as i32,
                    right: (x + 320.0) as i32,
                    bottom: (y + 300.0) as i32,
                },
                default_color: Color::rgb(230, 230, 230),
            });
        }

        base_areas.push(TextArea {
            buffer: &self.toolbar_buffer,
            left: 12.0,
            top: 0.0,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: (self.viewport_width - 48.0) as i32,
                bottom: TOOLBAR_HEIGHT as i32,
            },
            default_color: Color::rgb(220, 220, 225),
        });

        base_areas.push(TextArea {
            buffer: &self.tab_buffer,
            left: 8.0,
            top: TOOLBAR_HEIGHT + 4.0,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: TOOLBAR_HEIGHT as i32,
                right: self.viewport_width as i32,
                bottom: (TOOLBAR_HEIGHT + TAB_HEIGHT) as i32,
            },
            default_color: Color::rgb(220, 220, 225),
        });

        let (gx, gy, gw, gh) = self.gear_rect();
        base_areas.push(TextArea {
            buffer: &self.gear_buffer,
            left: gx,
            top: gy,
            scale: 1.0,
            bounds: TextBounds {
                left: gx as i32,
                top: gy as i32,
                right: (gx + gw) as i32,
                bottom: (gy + gh) as i32,
            },
            default_color: Color::rgb(220, 220, 225),
        });

        let (_bx, _by, _bw, _bh, _, _, _, _) = self.lang_badge_geometry();
        let (tx, ty) = self.lang_badge_text_pos();
        let [tr, tg, tb] = self.settings.color_for(Some("topbar_text"));
        base_areas.push(TextArea {
            buffer: &self.lang_badge_buffer,
            left: tx,
            top: ty,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: self.viewport_width as i32,
                bottom: TOOLBAR_HEIGHT as i32,
            },
            default_color: Color::rgb(tr, tg, tb),
        });

        if self.panel_open {
            let (px, py, pw, ph) = self.panel_rect();
            overlay_areas.push(TextArea {
                buffer: &self.panel_buffer,
                left: px + 8.0,
                top: py + 8.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: px as i32,
                    top: py as i32,
                    right: (px + pw) as i32,
                    bottom: (py + ph) as i32,
                },
                default_color: Color::rgb(220, 220, 225),
            });
        }

        if self.picker_open {
            let (hx, hy, _hw, hh) = self.picker_hue_rect();
            let text_top = hy + hh + 10.0;
            overlay_areas.push(TextArea {
                buffer: &self.picker_buffer,
                left: hx,
                top: text_top,
                scale: 1.0,
                bounds: TextBounds {
                    left: hx as i32,
                    top: text_top as i32,
                    right: (hx + 220.0) as i32,
                    bottom: (text_top + 60.0) as i32,
                },
                default_color: Color::rgb(220, 220, 225),
            });
        }

        if self.prompt_open {
            let (px, py, pw, ph) = self.prompt_rect();
            overlay_areas.push(TextArea {
                buffer: &self.prompt_buffer,
                left: px + 16.0,
                top: py + 16.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: px as i32,
                    top: py as i32,
                    right: (px + pw) as i32,
                    bottom: (py + ph) as i32,
                },
                default_color: Color::rgb(230, 230, 230),
            });
        }

        if self.menu_open() {
            let (mx, my, mw, mh) = self.menu_rect();
            overlay_areas.push(TextArea {
                buffer: &self.menu_buffer,
                left: mx + 8.0,
                top: my + 6.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: mx as i32,
                    top: my as i32,
                    right: (mx + mw) as i32,
                    bottom: (my + mh) as i32,
                },
                default_color: Color::rgb(225, 225, 230),
            });
        }

        if self.anim_time < self.tooltip_expiration {
            let (tx, ty) = self.tooltip_pos;
            overlay_areas.push(TextArea {
                buffer: &self.tooltip_buffer,
                left: tx,
                top: ty,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                },
                default_color: Color::rgb(255, 255, 255),
            });
            self.dirty = true; // force refresh while tooltip is visible so timer evaluates
        } else if self.tooltip_text.is_some() {
            self.tooltip_text = None;
        }

        self.text_renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            Resolution { width, height },
            base_areas,
            &mut self.swash_cache,
        ).unwrap();

        self.overlay_text_renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            Resolution { width, height },
            overlay_areas,
            &mut self.swash_cache,
        )
    }

    pub fn render<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
    ) -> Result<(), glyphon::RenderError> {
        self.text_renderer.render(&self.atlas, pass)
    }

    pub fn render_overlay<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
    ) -> Result<(), glyphon::RenderError> {
        self.overlay_text_renderer.render(&self.atlas, pass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_editor() -> Option<TextState> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor::default(),
            None,
        ))
        .ok()?;

        let mut state = TextState::new(&device, &queue, TextureFormat::Bgra8UnormSrgb, 800, 600);
        // Limpiar el buffer inicial a un documento en blanco para pruebas
        state.documents[state.active].lines = vec![String::new()];
        state.documents[state.active].cursor_line = 0;
        state.documents[state.active].cursor_col = 0;
        state.documents[state.active].undo_stack.clear();
        state.documents[state.active].redo_stack.clear();
        state.settings = crate::settings::Settings::default();
        Some(state)
    }

    #[test]
    fn test_insert_char_and_undo() {
        let Some(mut editor) = create_test_editor() else { return; };
        for c in "hello".chars() {
            editor.insert_char(c);
        }
        assert_eq!(editor.full_text(), "hello");
        assert_eq!(editor.cursor_position(), (0, 5));

        editor.undo();
        assert_eq!(editor.full_text(), "");
        assert_eq!(editor.cursor_position(), (0, 0));

        editor.redo();
        assert_eq!(editor.full_text(), "hello");
        assert_eq!(editor.cursor_position(), (0, 5));
    }

    #[test]
    fn test_smart_autoindent_with_braces() {
        let Some(mut editor) = create_test_editor() else { return; };
        for c in "    fn run() {".chars() {
            editor.insert_char(c);
        }
        editor.insert_newline();
        // Debe conservar los 4 espacios base + 4 espacios por abrir llave '{'
        assert_eq!(editor.cursor_position(), (1, 8));
        let lines = &editor.documents[editor.active].lines;
        assert_eq!(lines[0], "    fn run() {");
        assert_eq!(lines[1], "        ");
    }

    #[test]
    fn test_indent_unindent_single_line() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.indent();
        assert_eq!(editor.full_text(), "    ");

        for c in "test".chars() {
            editor.insert_char(c);
        }
        assert_eq!(editor.full_text(), "    test");
        
        editor.unindent();
        assert_eq!(editor.full_text(), "test");
    }

    #[test]
    fn test_indent_unindent_selection() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.documents[editor.active].lines = vec!["primera".into(), "segunda".into()];
        editor.documents[editor.active].cursor_line = 1;
        editor.documents[editor.active].cursor_col = 7;
        editor.documents[editor.active].selection_anchor = Some((0, 0));

        editor.indent();
        let lines = &editor.documents[editor.active].lines;
        assert_eq!(lines[0], "    primera");
        assert_eq!(lines[1], "    segunda");

        editor.unindent();
        let lines = &editor.documents[editor.active].lines;
        assert_eq!(lines[0], "primera");
        assert_eq!(lines[1], "segunda");
    }

    #[test]
    fn test_find_next() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.documents[editor.active].lines = vec![
            "const int foo = 10;".into(),
            "int bar = 20;".into(),
            "foo += bar;".into(),
        ];
        editor.documents[editor.active].cursor_line = 0;
        editor.documents[editor.active].cursor_col = 0;

        // Primera aparición: en línea 0
        let found = editor.find_next("foo");
        assert!(found);
        assert_eq!(editor.documents[editor.active].cursor_line, 0);
        assert_eq!(editor.documents[editor.active].cursor_col, 13); // final de 'fooen "const int foo"
        assert_eq!(editor.documents[editor.active].selection_anchor, Some((0, 10)));

        // Segunda aparición: en línea 2
        let found2 = editor.find_next("foo");
        assert!(found2);
        assert_eq!(editor.documents[editor.active].cursor_line, 2);
        assert_eq!(editor.documents[editor.active].cursor_col, 3);
        assert_eq!(editor.documents[editor.active].selection_anchor, Some((2, 0)));

        // Buscar texto que no existe
        let not_found = editor.find_next("inexistente");
        assert!(!not_found);
    }

    #[test]
    fn test_backspace_and_delete_forward() {
        let Some(mut editor) = create_test_editor() else { return; };
        for c in "abc".chars() {
            editor.insert_char(c);
        }
        editor.backspace();
        assert_eq!(editor.full_text(), "ab");

        editor.move_cursor(-1, 0, false);
        editor.move_cursor(-1, 0, false);
        assert_eq!(editor.cursor_position(), (0, 0));
        editor.delete_forward();
        assert_eq!(editor.full_text(), "b");
    }

    #[test]
    fn test_notepad_menu_and_txt_language_switching() {
        let Some(mut editor) = create_test_editor() else { return; };
        // Verificamos que el documento inicial sea .txt y PlainText
        assert_eq!(editor.documents[0].language, Language::PlainText);
        assert_eq!(editor.documents[0].untitled_name, "Sin título 1.txt");

        // Crear una nueva pestaña debe ser Sin título 2.txt con PlainText
        editor.new_tab();
        assert_eq!(editor.tab_count(), 2);
        assert_eq!(editor.active_tab(), 1);
        assert_eq!(editor.active_language(), Language::PlainText);
        assert_eq!(editor.active_title(), "Sin título 2.txt");

        // Cambiar el lenguaje a Python debe actualizar el título a Sin título 2.py
        editor.set_active_language(Language::Python);
        assert_eq!(editor.active_language(), Language::Python);
        assert_eq!(editor.active_title(), "Sin título 2.py");

        // Abrir el menú Lenguaje debe tener 18 lenguajes y marcar Python como seleccionado
        editor.open_menu(TopMenu::Lenguaje);
        assert!(editor.menu_open());
        assert_eq!(editor.menu_rows.len(), 18);
        let py_row = editor.menu_rows.iter().find(|r| r.action == Some(MenuAction::SetLanguage(Language::Python))).unwrap();
        assert!(py_row.is_checked);
        assert!(py_row.label.contains("âœ“"));

        let rust_row = editor.menu_rows.iter().find(|r| r.action == Some(MenuAction::SetLanguage(Language::Rust))).unwrap();
        assert!(!rust_row.is_checked);

        editor.close_menu();
        assert!(!editor.menu_open());
    }

    #[test]
    fn test_topbar_geometry_and_add_tab_hit_test() {
        let Some(mut editor) = create_test_editor() else { return; };
        // Pestaña inicial (índice 0)
        let rect0 = editor.tab_rect(0);
        assert_eq!(rect0.0, 8.0);
        assert_eq!(rect0.1, TOOLBAR_HEIGHT + 2.0);
        assert!(rect0.2 > 0.0);

        // Crear segunda pestaña
        editor.new_tab();
        let rect1 = editor.tab_rect(1);
        assert!(rect1.0 > rect0.0 + rect0.2);

        // Botón '+para agregar pestañas
        let add_rect = editor.add_tab_rect();
        assert!(add_rect.0 > rect1.0 + rect1.2);
        assert_eq!(add_rect.2, 24.0);

        // Hit testing en el botón '+'
        let mid_x = add_rect.0 + add_rect.2 / 2.0;
        let mid_y = add_rect.1 + add_rect.3 / 2.0;
        assert!(editor.add_tab_hit_test(mid_x, mid_y));

        // Fuera del botón '+'
        assert!(!editor.add_tab_hit_test(mid_x, 10.0)); // dentro de toolbar, no tabs
        assert!(!editor.add_tab_hit_test(add_rect.0 + add_rect.2 + 50.0, mid_y)); // a la derecha del botón
    }

    #[test]
    fn test_topbar_customizable_colors() {
        let mut settings = crate::settings::Settings::default();
        // Verificar colores por defecto en OneDark
        let default_accent = settings.topbar_accent_rgb();
        assert!(default_accent[0] > 0.0 && default_accent[2] > 0.0);

        // Cambiar acento a rojo puro [255, 0, 0]
        settings.set_custom_color("topbar_accent", [255, 0, 0]);
        let custom_accent = settings.topbar_accent_rgb();
        assert_eq!(custom_accent, [1.0, 0.0, 0.0]);

        // Cambiar fondo de barra superior a azul oscuro [10, 20, 40]
        settings.set_custom_color("topbar_bg", [10, 20, 40]);
        let custom_bg = settings.topbar_bg_rgb();
        assert_eq!(custom_bg, [10.0 / 255.0, 20.0 / 255.0, 40.0 / 255.0]);

        // Cambiar pestaña activa
        settings.set_custom_color("tab_active", [30, 35, 45]);
        let custom_tab = settings.tab_active_rgb();
        assert_eq!(custom_tab, [30.0 / 255.0, 35.0 / 255.0, 45.0 / 255.0]);

        // Reset
        settings.reset_custom_colors();
        assert_eq!(settings.topbar_accent_rgb(), default_accent);
    }

    #[test]
    fn test_topbar_text_customization() {
        let mut settings = crate::settings::Settings::default();
        let default_text = settings.topbar_text_rgb();
        assert!(default_text[0] > 0.0);

        // Cambiar texto de la barra superior a verde neón [50, 255, 100]
        settings.set_custom_color("topbar_text", [50, 255, 100]);
        let custom_text = settings.topbar_text_rgb();
        assert_eq!(custom_text, [50.0 / 255.0, 255.0 / 255.0, 100.0 / 255.0]);

        settings.reset_custom_colors();
        assert_eq!(settings.topbar_text_rgb(), default_text);
    }

    #[test]
    fn test_rgb_gamer_mode_and_chroma_colors() {
        let Some(mut editor) = create_test_editor() else { return; };
        assert!(!editor.settings.rgb_gamer_mode);

        let initial_cursor = editor.cursor_color();

        // Activar modo Gamer
        editor.toggle_rgb_gamer_mode();
        assert!(editor.settings.rgb_gamer_mode);

        editor.set_anim_time(1.5);
        let gamer_cursor = editor.cursor_color();
        assert_ne!(gamer_cursor, initial_cursor);

        // Verificar que chroma genera colores RGB válidos en el rango [0, 1]
        let chroma1 = crate::color::chroma_rgb_f32(1.5, 0.0);
        let chroma2 = crate::color::chroma_rgb_f32(1.5, 20.0);
        assert!(chroma1[0] >= 0.0 && chroma1[0] <= 1.0);
        assert!(chroma1[1] >= 0.0 && chroma1[1] <= 1.0);
        assert!(chroma1[2] >= 0.0 && chroma1[2] <= 1.0);
        assert_ne!(chroma1, chroma2); // Desplazamiento de fase horizontal
    }

    #[test]
    fn test_picker_indicators_and_coordinates() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.open_picker("keyword");
        assert!(editor.picker_open());

        editor.picker_set_sv(0.65, 0.85);
        assert!((editor.picker_sat() - 0.65).abs() < 1e-4);
        assert!((editor.picker_val() - 0.85).abs() < 1e-4);

        editor.picker_set_hue(180.0);
        assert!((editor.picker_hue() - 180.0).abs() < 1e-4);

        let rgb = editor.picker_color_rgb();
        assert!(rgb[0] >= 0.0 && rgb[0] <= 1.0);
        assert!(rgb[1] >= 0.0 && rgb[1] <= 1.0);
        assert!(rgb[2] >= 0.0 && rgb[2] <= 1.0);

        editor.close_picker();
        assert!(!editor.picker_open());
    }

    #[test]
    fn test_editor_and_marker_opacity_cycling() {
        let mut settings = crate::settings::Settings::default();
        assert_eq!(settings.editor_opacity, 0.85);
        assert_eq!(settings.marker_opacity, 0.30);

        // Ciclar opacidad del fondo de código: 0.85 -> 0.92 -> 1.0 -> 0.50 -> 0.70 -> 0.85
        settings.cycle_editor_opacity();
        assert_eq!(settings.editor_opacity, 1.0);
        settings.cycle_editor_opacity();
        assert_eq!(settings.editor_opacity, 0.0);
        settings.cycle_editor_opacity();
        assert_eq!(settings.editor_opacity, 0.25);
        settings.cycle_editor_opacity();
        assert_eq!(settings.editor_opacity, 0.50);
        settings.cycle_editor_opacity();
        assert_eq!(settings.editor_opacity, 0.75);

        // Ciclar opacidad del marcador: 0.30 -> 0.45 -> 0.65 -> 0.85 -> 0.15 -> 0.30
        settings.cycle_marker_opacity();
        assert_eq!(settings.marker_opacity, 0.45);
        settings.cycle_marker_opacity();
        assert_eq!(settings.marker_opacity, 0.65);
        settings.cycle_marker_opacity();
        assert_eq!(settings.marker_opacity, 0.85);
        settings.cycle_marker_opacity();
        assert_eq!(settings.marker_opacity, 0.15);
        settings.cycle_marker_opacity();
        assert_eq!(settings.marker_opacity, 0.30);
    }

    #[test]
    fn test_bookmarks_toggle_and_gutter_hit_test() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.documents[editor.active].lines = vec![
            "linea 1".into(),
            "linea 2".into(),
            "linea 3".into(),
            "linea 4".into(),
        ];

        // Inicialmente sin marcadores
        assert!(!editor.is_bookmarked(0));
        assert!(!editor.is_bookmarked(1));
        assert!(editor.visible_bookmarks().is_empty());

        // Alternar marcador en la línea 1
        editor.toggle_bookmark(1);
        assert!(editor.is_bookmarked(1));
        assert_eq!(editor.visible_bookmarks().len(), 1);
        assert_eq!(editor.visible_bookmarks()[0].0, 1);

        // Alternar marcador en la línea actual con el cursor (línea 0)
        editor.documents[editor.active].cursor_line = 0;
        editor.toggle_current_line_bookmark();
        assert!(editor.is_bookmarked(0));
        assert_eq!(editor.visible_bookmarks().len(), 2);

        // Desactivar marcador volviendo a alternar
        editor.toggle_bookmark(1);
        assert!(!editor.is_bookmarked(1));
        assert_eq!(editor.visible_bookmarks().len(), 1);

        // Hit testing en el gutter para la línea 2
        let line_h = editor.line_height();
        let gutter_x = 20.0; // dentro de gutter (PADDING + GUTTER_WIDTH = 80.0)
        let gutter_y = TOP_OFFSET + PADDING + 2.0 * line_h + 5.0; // sobre línea 2
        let hit = editor.gutter_hit_test(gutter_x, gutter_y);
        assert_eq!(hit, Some(2));

        // Fuera del gutter (área de código)
        let code_x = 120.0;
        assert_eq!(editor.gutter_hit_test(code_x, gutter_y), None);
    }

    #[test]
    fn test_lang_badge_geometry_consistency() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.set_active_language(Language::Rust);
        let (bx, by, bw, bh, lx, ly, lw, lh) = editor.lang_badge_geometry();
        let (tx, ty) = editor.lang_badge_text_pos();

        // El texto y el LED deben estar estrictamente DENTRO del chip del badge
        assert!(tx > bx, "El texto debe comenzar después del borde izquierdo del chip");
        assert!(tx + 30.0 < bx + bw, "El texto debe caber completamente dentro del ancho del chip");
        assert!(lx >= bx, "El LED debe estar dentro del chip horizontalmente");
        assert!(lx + lw <= bx + bw, "El LED debe terminar antes del borde derecho");
        assert!(ly >= by && ly + lh <= by + bh, "El LED debe estar centrado verticalmente en el chip");
        assert_eq!(ty, 0.0);
    }

    #[test]
    fn test_panel_hit_test_and_scrolling() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.toggle_panel();
        assert!(editor.panel_open());

        let (px, py, pw, _ph) = editor.panel_rect();
        let mid_x = px + pw / 2.0;

        // Buscar el índice de la fila "Opacidad marcador"
        let marker_op_idx = editor
            .panel_rows
            .iter()
            .position(|r| r.action == Some(PanelAction::CycleMarkerOpacity))
            .expect("Debe existir la fila de opacidad del marcador");

        // Calcular coordenada Y exacta para esa fila
        let row_y = py + 8.0 + marker_op_idx as f32 * PANEL_ROW_HEIGHT + PANEL_ROW_HEIGHT / 2.0;

        // El hit test debe devolver exactamente CycleMarkerOpacity
        let action = editor.panel_hit_test(mid_x, row_y);
        assert_eq!(action, Some(PanelAction::CycleMarkerOpacity));

        // Hover rect debe existir
        let hover = editor.panel_hover_row_rect(mid_x, row_y);
        assert!(hover.is_some());

        // Buscar el índice de la fila "Opacidad fondo"
        let editor_op_idx = editor
            .panel_rows
            .iter()
            .position(|r| r.action == Some(PanelAction::CycleEditorOpacity))
            .expect("Debe existir la fila de opacidad del fondo");
        let editor_row_y = py + 8.0 + editor_op_idx as f32 * PANEL_ROW_HEIGHT + PANEL_ROW_HEIGHT / 2.0;
        assert_eq!(editor.panel_hit_test(mid_x, editor_row_y), Some(PanelAction::CycleEditorOpacity));

        // Probar scroll del panel
        assert_eq!(editor.panel_scroll, 0);
        editor.scroll_panel(3);
        assert_eq!(editor.panel_scroll, 3);
        // Al scrollear 3 filas hacia abajo, la fila que estaba en marker_op_idx ahora está 3 filas más arriba en vista
        let scrolled_row_y = py + 8.0 + (marker_op_idx - 3) as f32 * PANEL_ROW_HEIGHT + PANEL_ROW_HEIGHT / 2.0;
        assert_eq!(editor.panel_hit_test(mid_x, scrolled_row_y), Some(PanelAction::CycleMarkerOpacity));

        editor.close_panel();
        assert!(!editor.panel_open());
    }

    #[test]
    fn test_menu_and_scrollbar_opacity_and_colors() {
        let mut settings = crate::settings::Settings::default();

        // 1. Probar ciclado de opacidad de menús
        let initial_menu_op = settings.menu_opacity;
        assert!((initial_menu_op - 0.96).abs() < 0.01);
        settings.cycle_menu_opacity();
        assert!((settings.menu_opacity - 1.0).abs() < 0.01);
        settings.cycle_menu_opacity();
        assert!((settings.menu_opacity - 0.50).abs() < 0.01);
        settings.cycle_menu_opacity();
        assert!((settings.menu_opacity - 0.70).abs() < 0.01);

        // 2. Probar ciclado de opacidad de scrollbar
        let initial_sb_op = settings.scrollbar_opacity;
        assert!((initial_sb_op - 0.75).abs() < 0.01);
        settings.cycle_scrollbar_opacity();
        assert!((settings.scrollbar_opacity - 0.90).abs() < 0.01);
        settings.cycle_scrollbar_opacity();
        assert!((settings.scrollbar_opacity - 1.0).abs() < 0.01);
        settings.cycle_scrollbar_opacity();
        assert!((settings.scrollbar_opacity - 0.30).abs() < 0.01);

        // 3. Probar colores de menú y scrollbar por defecto y personalizados
        let menu_rgb = settings.menu_bg_rgb();
        assert!(menu_rgb[0] >= 0.0 && menu_rgb[0] <= 1.0);
        let sb_rgb = settings.scrollbar_color_rgb();
        assert!(sb_rgb[0] >= 0.0 && sb_rgb[0] <= 1.0);

        // Asignar color personalizado
        settings.set_custom_color("menu_bg", [30, 40, 50]);
        settings.set_custom_color("scrollbar", [200, 100, 50]);
        assert_eq!(settings.menu_bg_rgb(), [30.0 / 255.0, 40.0 / 255.0, 50.0 / 255.0]);
        assert_eq!(settings.scrollbar_color_rgb(), [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0]);
    }

    #[test]
    fn test_right_scrollbar_hit_test_and_scrolling() {
        let Some(mut editor) = create_test_editor() else { return; };

        // 1. Rectángulo de la scrollbar en el extremo derecho
        let (sx, sy, sw, sh) = editor.scrollbar_rect();
        assert_eq!(sx, editor.viewport_width - SCROLLBAR_WIDTH);
        assert_eq!(sy, TOP_OFFSET);
        assert_eq!(sw, SCROLLBAR_WIDTH);
        assert!(sh > 0.0);

        // 2. Hit test de la scrollbar
        let expected_x = editor.viewport_width - SCROLLBAR_WIDTH / 2.0;
        assert!(editor.scrollbar_hit_test(expected_x, TOP_OFFSET + 50.0));
        assert!(editor.scrollbar_hit_test(editor.viewport_width, TOP_OFFSET + 50.0));
        assert!(!editor.scrollbar_hit_test(editor.viewport_width - SCROLLBAR_WIDTH - 1.0, TOP_OFFSET + 50.0));
        assert!(!editor.scrollbar_hit_test(expected_x, 5.0)); // en topbar

        // 3. gutter_hit_test no debe ser activado dentro del ancho de la scrollbar
        assert_eq!(editor.gutter_hit_test(expected_x, TOP_OFFSET + 20.0), None);

        // 4. Scrolling mediante scrollbar
        // Llenar el documento con 100 líneas
        let doc = &mut editor.documents[editor.active];
        doc.lines = (0..100).map(|i| format!("line {i}")).collect();
        assert_eq!(doc.scroll_offset, 0);

        let (th_x, th_y, th_w, th_h) = editor.scrollbar_thumb_rect();
        assert_eq!(th_x, editor.viewport_width - SCROLLBAR_WIDTH + 1.0);
        assert_eq!(th_w, SCROLLBAR_WIDTH - 2.0);
        assert!(th_h >= 20.0);
        assert_eq!(th_y, TOP_OFFSET);

        // Arrastrar la scrollbar a la mitad de la pantalla
        let track_h = (editor.viewport_height - TOP_OFFSET).max(10.0);
        let mid_y = TOP_OFFSET + track_h / 2.0;
        editor.scrollbar_scroll_to_y(mid_y);

        let new_offset = editor.documents[editor.active].scroll_offset;
        assert!(new_offset > 0, "Al mover el scrollbar a la mitad debe haber scroll");

        // Al moverlo al final
        editor.scrollbar_scroll_to_y(TOP_OFFSET + track_h);
        let max_offset = editor.documents[editor.active].scroll_offset;
        assert!(max_offset >= new_offset);
    }

    #[test]
    fn test_selective_rgb_gamer_mode() {
        use crate::settings::RgbElement;

        let Some(mut editor) = create_test_editor() else { return; };
        assert!(!editor.settings.rgb_gamer_mode);

        // Cuando el maestro está apagado, ningún elemento está activo
        for elem in RgbElement::ALL {
            assert!(!editor.settings.is_rgb_active(elem), "Elemento {elem:?} no debe estar activo con maestro apagado");
        }

        // Encender el modo maestro RGB
        editor.toggle_rgb_gamer_mode();
        assert!(editor.settings.rgb_gamer_mode);

        // Elementos con default ON
        assert!(editor.settings.is_rgb_active(RgbElement::Syntax));
        assert!(editor.settings.is_rgb_active(RgbElement::Topbar));
        assert!(editor.settings.is_rgb_active(RgbElement::Cursor));

        // Elementos con default OFF
        assert!(!editor.settings.is_rgb_active(RgbElement::Menus));
        assert!(!editor.settings.is_rgb_active(RgbElement::Scrollbar));
        assert!(!editor.settings.is_rgb_active(RgbElement::Markers));

        // Alternar selectivamente los nuevos elementos agregados
        editor.toggle_rgb_element(RgbElement::Menus);
        assert!(editor.settings.is_rgb_active(RgbElement::Menus));

        editor.toggle_rgb_element(RgbElement::Scrollbar);
        assert!(editor.settings.is_rgb_active(RgbElement::Scrollbar));

        editor.toggle_rgb_element(RgbElement::Markers);
        assert!(editor.settings.is_rgb_active(RgbElement::Markers));

        // Apagar individualmente la sintaxis
        editor.toggle_rgb_element(RgbElement::Syntax);
        assert!(!editor.settings.is_rgb_active(RgbElement::Syntax));

        // Apagar el interruptor maestro: todo debe ser inactivo
        editor.toggle_rgb_gamer_mode();
        assert!(!editor.settings.rgb_gamer_mode);
        assert!(!editor.settings.is_rgb_active(RgbElement::Menus));
        assert!(!editor.settings.is_rgb_active(RgbElement::Scrollbar));
        assert!(!editor.settings.is_rgb_active(RgbElement::Markers));

        // Al volver a encender el maestro, debe recordar la configuración selectiva personalizada
        editor.toggle_rgb_gamer_mode();
        assert!(editor.settings.rgb_gamer_mode);
        assert!(editor.settings.is_rgb_active(RgbElement::Menus));
        assert!(editor.settings.is_rgb_active(RgbElement::Scrollbar));
        assert!(editor.settings.is_rgb_active(RgbElement::Markers));
        assert!(!editor.settings.is_rgb_active(RgbElement::Syntax));

        // Verificar que el panel de configuración contenga todas las opciones
        editor.toggle_panel();
        assert!(editor.panel_rows.iter().any(|r| r.action == Some(PanelAction::CycleMenuOpacity)));
        assert!(editor.panel_rows.iter().any(|r| r.action == Some(PanelAction::CycleScrollbarOpacity)));
        assert!(editor.panel_rows.iter().any(|r| r.action == Some(PanelAction::EditColor("menu_bg"))));
        assert!(editor.panel_rows.iter().any(|r| r.action == Some(PanelAction::EditColor("scrollbar"))));
        for elem in RgbElement::ALL {
            assert!(editor.panel_rows.iter().any(|r| r.action == Some(PanelAction::ToggleRgbElement(elem))));
        }
    }
}

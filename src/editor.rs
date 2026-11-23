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
#[allow(dead_code)]
pub const TERMINAL_HEIGHT: f32 = 250.0;
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
    OpenRecent(usize),
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
    DuplicateLine,
    ToggleComment,
    MoveLineUp,
    MoveLineDown,
    SelectAll,

    // Buscar
    Find,
    FindNext,
    Replace,
    GoToLine,

    // Ver
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleTerminal,

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
    RunCode,
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

pub type ToolbarItem = (&'static str, Option<TopMenu>, Option<MenuAction>, f32, f32);

pub const TOOLBAR_ITEMS: [ToolbarItem; 8] = [
    ("Archivo", Some(TopMenu::Archivo), None, 12.0, 88.0),
    ("Editar", Some(TopMenu::Editar), None, 88.0, 156.0),
    ("Buscar", Some(TopMenu::Buscar), None, 156.0, 224.0),
    ("Ver", Some(TopMenu::Ver), None, 224.0, 266.0),
    ("Lenguaje", Some(TopMenu::Lenguaje), None, 266.0, 350.0),
    ("Configuración", Some(TopMenu::Configuracion), None, 350.0, 476.0),
    ("▶ Ejecutar", None, Some(MenuAction::RunCode), 476.0, 578.0),
    (">_ Terminal", None, Some(MenuAction::ToggleTerminal), 578.0, 688.0),
];

/// Elementos interactivos del panel dock inferior de ejecución y terminal
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TerminalHit {
    ResizeHandle,
    TabOutput,
    TabShell,
    BtnRun,
    BtnStop,
    BtnClear,
    BtnZoomOut,
    BtnZoomReset,
    BtnZoomIn,
    BtnClose,
    Content,
    Scrollbar(f32),
    Input,
}

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
    CycleTerminalOpacity,
    CycleScrollbarOpacity,
    ToggleRgbGamer,
    ToggleRgbElement(crate::settings::RgbElement),
    ToggleAutoClosePairs,
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
    SecondaryButton,
    Field(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptPurpose {
    SavePreset,
    Find,
    Replace,
    GoToLine,
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
    scroll_x: f32,
    uses_crlf: bool,
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
            scroll_x: 0.0,
            uses_crlf: false,
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

    pub viewport_width: f32,
    pub viewport_height: f32,
    font_scale: f32,
    cached_char_width: f32,

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
    tab_scroll_x: f32,
    lang_badge_buffer: Buffer,

    // --- Notepad++ Menú Superior ---
    menu_buffer: Buffer,
    active_menu: Option<TopMenu>,
    menu_rows: Vec<MenuRow>,

    // --- Cajita de texto genérica (ej. nombrar un preset, buscar, reemplazar) ---
    prompt_buffer: Buffer,
    prompt_open: bool,
    prompt_text: String,
    prompt_replace_text: String,
    prompt_field: usize,
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
    pub last_search_status: String,
    status_buffer: Buffer,

    // --- Números de línea ---
    line_numbers_buffer: Buffer,

    pub search_result_marker: Option<(usize, f32)>,
    pub anim_time: f32,
    pub terminal: crate::terminal::TerminalState,
    pub terminal_header_buffer: Buffer,
    pub terminal_btn_run_buffer: Buffer,
    pub terminal_btn_stop_buffer: Buffer,
    pub terminal_btn_clear_buffer: Buffer,
    pub terminal_btn_zoom_out_buffer: Buffer,
    pub terminal_btn_zoom_lbl_buffer: Buffer,
    pub terminal_btn_zoom_in_buffer: Buffer,
    pub terminal_btn_close_buffer: Buffer,
    pub terminal_buffer: Buffer,
    pub terminal_input_buffer: Buffer,
    pub terminal_focused: bool,
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
        let term_metrics = Metrics::new(13.0, 18.0);
        let terminal_header_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        let terminal_btn_run_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        let terminal_btn_stop_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        let terminal_btn_clear_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        let terminal_btn_zoom_out_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        let terminal_btn_zoom_lbl_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        let terminal_btn_zoom_in_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        let terminal_btn_close_buffer = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        let terminal_buffer = Buffer::new(&mut font_system, term_metrics);
        let terminal_input_buffer = Buffer::new(&mut font_system, term_metrics);
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
        tab_buffer.set_wrap(&mut font_system, Wrap::None);
        tab_buffer.set_size(&mut font_system, (width as f32).max(2000.0), TAB_HEIGHT);

        let mut lang_badge_buffer =
            Buffer::new(&mut font_system, Metrics::new(TOOLBAR_FONT_SIZE, TOOLBAR_HEIGHT));
        lang_badge_buffer.set_size(&mut font_system, 180.0, TOOLBAR_HEIGHT);

        let mut menu_buffer = Buffer::new(&mut font_system, Metrics::new(13.0, MENU_ROW_HEIGHT));
        menu_buffer.set_size(&mut font_system, 340.0, 600.0);

        let mut prompt_buffer = Buffer::new(&mut font_system, Metrics::new(17.0, 24.0));
        prompt_buffer.set_size(&mut font_system, 320.0, 90.0);

        let mut tooltip_buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        tooltip_buffer.set_size(&mut font_system, 400.0, 50.0);

        let mut status_buffer =
            Buffer::new(&mut font_system, Metrics::new(12.0, TOOLBAR_HEIGHT));
        status_buffer.set_size(&mut font_system, 220.0, TOOLBAR_HEIGHT);

        let mut line_numbers_buffer =
            Buffer::new(&mut font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        line_numbers_buffer.set_wrap(&mut font_system, Wrap::None);
        line_numbers_buffer.set_size(
            &mut font_system,
            PADDING + GUTTER_WIDTH - 18.0,
            height as f32 - TOP_OFFSET,
        );

        let settings = Settings::load();
        let cached_char_width = Self::measure_char_width(&mut font_system, FONT_SIZE, LINE_HEIGHT);

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
            cached_char_width,
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
            tab_scroll_x: 0.0,
            lang_badge_buffer,
            menu_buffer,
            active_menu: None,
            menu_rows: Vec::new(),
            prompt_buffer,
            prompt_open: false,
            prompt_text: String::new(),
            prompt_replace_text: String::new(),
            prompt_field: 0,
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
            last_search_status: String::new(),
            status_buffer,
            line_numbers_buffer,
            search_result_marker: None,
            anim_time: 0.0,
            terminal: crate::terminal::TerminalState::new(),
            terminal_header_buffer,
            terminal_btn_run_buffer,
            terminal_btn_stop_buffer,
            terminal_btn_clear_buffer,
            terminal_btn_zoom_out_buffer,
            terminal_btn_zoom_lbl_buffer,
            terminal_btn_zoom_in_buffer,
            terminal_btn_close_buffer,
            terminal_buffer,
            terminal_input_buffer,
            terminal_focused: false,
        };
        state.terminal.font_scale = state.settings.terminal_font_scale;
        state.rebuild_toolbar_buffer();
        state.rebuild_tab_buffer();
        state.rebuild_line_numbers();
        state.rebuild_status_buffer();
        state.rebuild_terminal_buffer();
        state
    }

    /// Marca el documento activo como modificado y actualiza la pestaña solo
    /// cuando cambia de limpio -> sucio (para que el punto ámbar ● aparezca al instante).
    fn mark_active_dirty(&mut self) {
        let was_dirty = self.documents[self.active].dirty;
        self.documents[self.active].dirty = true;
        self.dirty = true;
        if !was_dirty {
            self.rebuild_tab_buffer();
        }
    }

    pub fn editor_bottom(&self) -> f32 {
        if self.terminal.is_open {
            (self.viewport_height - self.terminal.height).max(TOP_OFFSET + 40.0)
        } else {
            self.viewport_height
        }
    }

    pub fn has_active_animation(&self) -> bool {
        self.anim_time < self.tooltip_expiration || self.search_result_marker.is_some()
    }

    fn rebuild_status_buffer(&mut self) {
        let doc = &self.documents[self.active];
        let ln = doc.cursor_line + 1;
        let col = doc.cursor_col + 1;
        let zoom = (self.font_scale * 100.0).round() as u32;
        let text = if let Some(sel) = self.selected_text() {
            let sel_len = sel.chars().count();
            format!("Ln {ln}, Col {col} ({sel_len} sel) · {zoom}%")
        } else {
            format!("Ln {ln}, Col {col} · {zoom}%")
        };
        let [tr, tg, tb] = self.settings.color_for(Some("topbar_text"));
        let dim = Color::rgb(
            ((tr as u16 * 3) / 4) as u8,
            ((tg as u16 * 3) / 4) as u8,
            ((tb as u16 * 3) / 4) as u8,
        );
        self.status_buffer.set_text(
            &mut self.font_system,
            &text,
            Attrs::new().family(Family::Monospace).color(dim),
            Shaping::Advanced,
        );
        self.status_buffer.shape_until_scroll(&mut self.font_system);
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
        // Si el archivo ya está abierto en otra pestaña, cambiamos a ella en vez de duplicarla.
        if let Some(existing_idx) = self
            .documents
            .iter()
            .position(|d| d.file_path.as_ref() == Some(&path))
        {
            self.active = existing_idx;
            self.metrics_dirty = true;
            self.dirty = true;
            self.hide_completions();
            self.rebuild_tab_buffer();
            self.rebuild_toolbar_buffer();
            return;
        }

        let metrics = Metrics::new(self.eff_font_size(), self.eff_line_height());
        let name = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("Sin título {}.txt", self.next_untitled));
        let language = path
            .extension()
            .and_then(|e| e.to_str())
            .map(Language::from_extension)
            .unwrap_or(Language::PlainText);
        let mut doc = Document::new(&mut self.font_system, metrics, name, language);
        let editor_h = (self.editor_bottom() - TOP_OFFSET).max(40.0);
        doc.buffer.set_size(
            &mut self.font_system,
            (self.viewport_width - PADDING * 2.0 - GUTTER_WIDTH).max(40.0),
            editor_h,
        );
        doc.uses_crlf = text.contains("\r\n");
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        doc.lines = if normalized.is_empty() {
            vec![String::new()]
        } else {
            normalized.split('\n').map(String::from).collect()
        };
        doc.file_path = Some(path);

        let is_first_virgin = self.documents.len() == 1
            && self.documents[0].file_path.is_none()
            && !self.documents[0].dirty
            && (self.documents[0].lines.is_empty()
                || (self.documents[0].lines.len() == 1 && self.documents[0].lines[0].is_empty()));

        if is_first_virgin {
            self.documents[0] = doc;
            self.active = 0;
        } else {
            self.documents.push(doc);
            self.active = self.documents.len() - 1;
        }

        self.hide_completions();
        self.dirty = true;
        self.rebuild_tab_buffer();
        self.rebuild_toolbar_buffer();
    }

    pub fn close_tab(&mut self, idx: usize) -> Option<(String, Language)> {
        if idx >= self.documents.len() {
            return None;
        }
        let closed_info = {
            let doc = &self.documents[idx];
            let uri = if let Some(ref path) = doc.file_path {
                let s = path.to_string_lossy().replace('\\', "/");
                if s.starts_with('/') {
                    format!("file://{s}")
                } else {
                    format!("file:///{s}")
                }
            } else {
                let ext = doc.language.default_extension();
                format!("file:///amoxlicode/{}.{}", doc.untitled_name.replace(' ', "_"), ext)
            };
            (uri, doc.language)
        };
        self.documents.remove(idx);
        if self.documents.is_empty() {
            let metrics = Metrics::new(self.eff_font_size(), self.eff_line_height());
            let name = format!("Sin título {}.txt", self.next_untitled);
            self.next_untitled += 1;
            let mut doc = Document::new(&mut self.font_system, metrics, name, Language::PlainText);
            let editor_h = (self.editor_bottom() - TOP_OFFSET).max(40.0);
            doc.buffer.set_size(
                &mut self.font_system,
                (self.viewport_width - PADDING * 2.0 - GUTTER_WIDTH).max(40.0),
                editor_h,
            );
            self.documents.push(doc);
            self.active = 0;
        } else if idx < self.active {
            // Preservar la misma pestaña activa cuando se cierra una anterior.
            self.active -= 1;
        } else if self.active >= self.documents.len() {
            self.active = self.documents.len() - 1;
        }
        self.metrics_dirty = true;
        self.dirty = true;
        self.hide_completions();
        self.rebuild_tab_buffer();
        self.rebuild_toolbar_buffer();
        Some(closed_info)
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
    // Barra de pestañas (adaptable + scroll horizontal)
    // ---------------------------------------------------------------

    pub fn tab_bar_rect(&self) -> (f32, f32, f32, f32) {
        (0.0, TOOLBAR_HEIGHT, self.viewport_width, TAB_HEIGHT)
    }

    pub fn tab_bar_contains(&self, x: f32, y: f32) -> bool {
        x >= 0.0 && x <= self.viewport_width && (TOOLBAR_HEIGHT..=TOOLBAR_HEIGHT + TAB_HEIGHT).contains(&y)
    }

    fn effective_tab_slot_chars(&self) -> usize {
        let count = self.documents.len().max(1);
        let char_w = TAB_FONT_SIZE * 0.6;
        let avail = (self.viewport_width - 48.0).max(160.0);
        let max_chars = (avail / (count as f32 * char_w)).floor() as usize;
        max_chars.clamp(12, TAB_SLOT_CHARS)
    }

    fn tab_slot_width(&self) -> f32 {
        (TAB_FONT_SIZE * 0.6) * self.effective_tab_slot_chars() as f32
    }

    fn max_tab_scroll_x(&self) -> f32 {
        let total = 8.0 + self.documents.len() as f32 * self.tab_slot_width() + 40.0;
        (total - self.viewport_width).max(0.0)
    }

    fn ensure_active_tab_visible(&mut self) {
        let slot_w = self.tab_slot_width();
        let tab_left = 8.0 + self.active as f32 * slot_w;
        let tab_right = tab_left + slot_w;
        let max_scroll = self.max_tab_scroll_x();
        if self.tab_scroll_x > max_scroll {
            self.tab_scroll_x = max_scroll;
        }
        if tab_left - self.tab_scroll_x < 8.0 {
            self.tab_scroll_x = (tab_left - 8.0).max(0.0);
        } else if tab_right - self.tab_scroll_x > self.viewport_width - 36.0 {
            self.tab_scroll_x = (tab_right - (self.viewport_width - 36.0)).clamp(0.0, max_scroll);
        }
    }

    pub fn scroll_tabs(&mut self, delta_px: f32) {
        let max_scroll = self.max_tab_scroll_x();
        self.tab_scroll_x = (self.tab_scroll_x + delta_px).clamp(0.0, max_scroll);
    }

    pub fn tab_rect(&self, idx: usize) -> (f32, f32, f32, f32) {
        let slot_width = self.tab_slot_width();
        let x = 8.0 + idx as f32 * slot_width - self.tab_scroll_x;
        let y = TOOLBAR_HEIGHT + 2.0;
        let w = slot_width - 3.0;
        let h = TAB_HEIGHT - 2.0;
        (x, y, w, h)
    }

    pub fn add_tab_rect(&self) -> (f32, f32, f32, f32) {
        let slot_width = self.tab_slot_width();
        let add_x = 8.0 + self.documents.len() as f32 * slot_width - self.tab_scroll_x + 4.0;
        let add_y = TOOLBAR_HEIGHT + 4.0;
        (add_x, add_y, 24.0, TAB_HEIGHT - 6.0)
    }

    pub fn add_tab_hit_test(&self, x: f32, y: f32) -> bool {
        let (ax, ay, aw, ah) = self.add_tab_rect();
        x >= ax && x <= ax + aw && y >= ay && y <= ay + ah
    }

    pub fn tab_hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if !(TOOLBAR_HEIGHT..=TOOLBAR_HEIGHT + TAB_HEIGHT).contains(&y) {
            return None;
        }
        let char_width = TAB_FONT_SIZE * 0.6;
        let slot_width = self.tab_slot_width();
        let rel_x = x - 8.0 + self.tab_scroll_x;
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
        if !(TOOLBAR_HEIGHT..=TOOLBAR_HEIGHT + TAB_HEIGHT).contains(&y) {
            return None;
        }
        let char_width = TAB_FONT_SIZE * 0.6;
        let slot_width = self.tab_slot_width();
        let rel_x = x - 8.0 + self.tab_scroll_x;
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

    /// Devuelve el índice de pestaña bajo (x, y) incluyendo el botón '×'
    /// (se usa para cerrar pestañas con clic central del ratón).
    pub fn tab_any_hit_test(&self, x: f32, y: f32) -> Option<usize> {
        if !(TOOLBAR_HEIGHT..=TOOLBAR_HEIGHT + TAB_HEIGHT).contains(&y) {
            return None;
        }
        let slot_width = self.tab_slot_width();
        let rel_x = x - 8.0 + self.tab_scroll_x;
        if rel_x < 0.0 {
            return None;
        }
        let idx = (rel_x / slot_width) as usize;
        if idx < self.documents.len() {
            Some(idx)
        } else {
            None
        }
    }

    fn rebuild_tab_buffer(&mut self) {
        self.ensure_active_tab_visible();
        let slot_chars = self.effective_tab_slot_chars();
        let total_w = 8.0 + self.documents.len() as f32 * self.tab_slot_width() + 80.0;
        self.tab_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        self.tab_buffer
            .set_size(&mut self.font_system, total_w.max(self.viewport_width), TAB_HEIGHT);

        let base_attrs = Attrs::new().family(Family::Monospace);
        let active_color = Color::rgb(255, 255, 255);
        let inactive_color = Color::rgb(155, 160, 172);
        let dirty_color = Color::rgb(255, 185, 45); // Ámbar cálido para archivos modificados
        let close_color = Color::rgb(150, 155, 165);

        let mut spans: Vec<(String, Color)> = Vec::new();
        for (i, doc) in self.documents.iter().enumerate() {
            let color = if i == self.active { active_color } else { inactive_color };
            let title = doc.tab_title();
            let max_chars = slot_chars.saturating_sub(4);
            let truncated = if title.chars().count() > max_chars {
                title.chars().take(max_chars.saturating_sub(1)).collect::<String>() + "\u{2026}"
            } else {
                title
            };

            if doc.dirty {
                spans.push((" \u{25cf}".to_string(), dirty_color));
                spans.push((format!(" {truncated}"), color));
            } else {
                spans.push((format!("   {truncated}"), color));
            }

            let current_len = 3 + truncated.chars().count();
            let pad = slot_chars.saturating_sub(current_len + 2);
            if pad > 0 {
                spans.push((" ".repeat(pad), color));
            }
            spans.push((" \u{d7}".to_string(), close_color));
        }

        // Botón '+' para crear pestaña nueva rápidamente
        spans.push(("  +  ".to_string(), Color::rgb(160, 170, 190)));

        let rich_spans: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|(s, c)| (s.as_str(), base_attrs.color(*c)))
            .collect();

        self.tab_buffer
            .set_rich_text(&mut self.font_system, rich_spans, Shaping::Advanced);
        self.tab_buffer.shape_until_scroll(&mut self.font_system);
    }

    // ---------------------------------------------------------------
    // Zoom / tamaño efectivo / scroll vertical y horizontal
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

    pub fn char_width(&self) -> f32 {
        if self.cached_char_width > 0.0 {
            self.cached_char_width
        } else {
            self.eff_font_size() * 0.5859375
        }
    }

    fn code_viewport_width(&self) -> f32 {
        (self.viewport_width - PADDING - GUTTER_WIDTH - SCROLLBAR_WIDTH - 8.0).max(80.0)
    }

    fn measure_char_width(font_system: &mut FontSystem, font_size: f32, line_height: f32) -> f32 {
        let mut buf = Buffer::new(font_system, Metrics::new(font_size, line_height));
        buf.set_text(
            font_system,
            "MMMMMMMMMM",
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        buf.shape_until_scroll(font_system);
        for run in buf.layout_runs() {
            if let Some(glyph) = run.glyphs.first() {
                if glyph.w > 0.0 {
                    return glyph.w;
                }
            }
        }
        font_size * 0.5859375
    }

    fn visible_lines(&self) -> usize {
        (((self.editor_bottom() - TOP_OFFSET - PADDING) / self.eff_line_height()).floor()
            as usize)
            .max(1)
    }

    fn ensure_cursor_visible(&mut self) {
        let visible = self.visible_lines();
        let char_w = self.char_width();
        let view_w = self.code_viewport_width();
        let doc = &mut self.documents[self.active];
        if doc.cursor_line < doc.scroll_offset {
            doc.scroll_offset = doc.cursor_line;
        } else if doc.cursor_line >= doc.scroll_offset + visible {
            doc.scroll_offset = doc.cursor_line + 1 - visible;
        }
        let cursor_rel_x = doc.cursor_col as f32 * char_w;
        let margin = (char_w * 3.0).min(view_w * 0.25);
        if doc.cursor_col == 0 {
            doc.scroll_x = 0.0;
        } else if cursor_rel_x < doc.scroll_x + margin {
            doc.scroll_x = (cursor_rel_x - margin).max(0.0);
        } else if cursor_rel_x > doc.scroll_x + view_w - margin {
            doc.scroll_x = (cursor_rel_x - view_w + margin).max(0.0);
        }
    }

    pub fn center_cursor_in_view(&mut self) {
        let visible = self.visible_lines();
        let doc = &mut self.documents[self.active];
        let half_visible = visible / 2;
        doc.scroll_offset = doc.cursor_line.saturating_sub(half_visible);
        self.ensure_cursor_visible();
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

    pub fn scroll_horizontal(&mut self, delta_px: f32) {
        let char_w = self.char_width();
        let view_w = self.code_viewport_width();
        let visible = self.visible_lines();
        let doc = &mut self.documents[self.active];
        let start = doc.scroll_offset.min(doc.lines.len().saturating_sub(1));
        let end = (start + visible).min(doc.lines.len());
        let max_chars = doc.lines[start..end]
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        let content_w = max_chars as f32 * char_w + 48.0;
        let max_scroll_x = (content_w - view_w).max(0.0);
        let new_x = (doc.scroll_x + delta_px).clamp(0.0, max_scroll_x);
        if (new_x - doc.scroll_x).abs() > 0.01 {
            doc.scroll_x = new_x;
            self.dirty = true;
        }
    }

    #[allow(dead_code)]
    pub fn active_scroll_x(&self) -> f32 {
        self.documents[self.active].scroll_x
    }

    pub fn zoom(&mut self, delta: f32) {
        self.font_scale = (self.font_scale + delta).clamp(0.5, 3.0);
        self.apply_zoom_metrics();
    }

    pub fn zoom_reset(&mut self) {
        self.font_scale = 1.0;
        self.apply_zoom_metrics();
    }

    fn gutter_text_width(&self) -> f32 {
        PADDING + GUTTER_WIDTH - 18.0
    }

    /// Calcula las métricas de los números de línea garantizando SIEMPRE el
    /// mismo `line_height` que el código (alineación vertical 1:1) y escalando
    /// suavemente `font_size` si hay zoom extremo o 5+ dígitos para que nunca
    /// se envuelvan ni se recorten.
    fn line_number_metrics(&self) -> Metrics {
        let line_h = self.eff_line_height();
        let total_lines = self.documents.get(self.active).map(|d| d.lines.len()).unwrap_or(1);
        let digits = total_lines.max(1).to_string().len().max(4) as f32;
        let max_font_for_gutter = (self.gutter_text_width() - 4.0) / (digits * 0.62);
        let font_size = self.eff_font_size().min(max_font_for_gutter).max(8.0);
        Metrics::new(font_size, line_h)
    }

    fn apply_zoom_metrics(&mut self) {
        let font_size = self.eff_font_size();
        let line_height = self.eff_line_height();
        self.cached_char_width = Self::measure_char_width(&mut self.font_system, font_size, line_height);
        let viewport_width = self.viewport_width;
        let viewport_height = self.viewport_height;
        let term_h = if self.terminal.is_open { self.terminal.height } else { 0.0 };
        let h = (viewport_height - TOP_OFFSET - term_h).max(40.0);
        let metrics = Metrics::new(font_size, line_height);
        let ln_metrics = self.line_number_metrics();
        let ln_width = self.gutter_text_width();

        {
            let active = self.active;
            let font_system = &mut self.font_system;
            let doc = &mut self.documents[active];
            doc.buffer.set_metrics(font_system, metrics);
            doc.buffer.set_size(
                font_system,
                viewport_width - PADDING * 2.0 - GUTTER_WIDTH,
                h,
            );
        }

        self.line_numbers_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        self.line_numbers_buffer
            .set_metrics(&mut self.font_system, ln_metrics);
        self.line_numbers_buffer
            .set_size(&mut self.font_system, ln_width, h);

        let max_scroll = self.documents[self.active].lines.len().saturating_sub(1);
        let doc = &mut self.documents[self.active];
        doc.scroll_offset = doc.scroll_offset.min(max_scroll);

        self.dirty = true;
    }

    pub fn set_theme(&mut self, theme: crate::settings::ThemeName) {
        self.settings.theme = theme;
        self.settings.save();
        self.dirty = true;
        self.rebuild_toolbar_buffer();
        self.rebuild_tab_buffer();
        self.rebuild_status_buffer();
        if self.panel_open {
            self.rebuild_panel_buffer();
        }
        if self.menu_open() {
            self.rebuild_menu_buffer();
        }
        if self.terminal.is_open {
            self.rebuild_terminal_buffer();
        }
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

    pub fn terminal_rect(&self) -> (f32, f32, f32, f32) {
        let h = self.terminal.height;
        let y = self.viewport_height - h;
        (0.0, y, self.viewport_width, h)
    }

    pub fn terminal_header_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, _th) = self.terminal_rect();
        (0.0, ty, tw, 28.0)
    }

    pub fn terminal_content_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, th) = self.terminal_rect();
        let h = (th - 28.0 - 28.0).max(10.0);
        (0.0, ty + 28.0, tw, h)
    }

    pub fn terminal_input_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, th) = self.terminal_rect();
        (0.0, ty + th - 28.0, tw, 28.0)
    }

    pub fn terminal_tab_output_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, _tw, _th) = self.terminal_rect();
        (8.0, ty + 3.0, 95.0, 22.0)
    }

    pub fn terminal_tab_shell_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, _tw, _th) = self.terminal_rect();
        (108.0, ty + 3.0, 105.0, 22.0)
    }

    pub fn terminal_btn_run_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, _th) = self.terminal_rect();
        (tw - 402.0, ty + 3.0, 82.0, 22.0)
    }

    pub fn terminal_btn_stop_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, _th) = self.terminal_rect();
        (tw - 312.0, ty + 3.0, 74.0, 22.0)
    }

    pub fn terminal_btn_clear_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, _th) = self.terminal_rect();
        (tw - 230.0, ty + 3.0, 72.0, 22.0)
    }

    pub fn terminal_btn_zoom_out_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, _th) = self.terminal_rect();
        (tw - 150.0, ty + 3.0, 26.0, 22.0)
    }

    pub fn terminal_btn_zoom_label_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, _th) = self.terminal_rect();
        (tw - 120.0, ty + 3.0, 48.0, 22.0)
    }

    pub fn terminal_btn_zoom_in_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, _th) = self.terminal_rect();
        (tw - 68.0, ty + 3.0, 26.0, 22.0)
    }

    pub fn terminal_btn_close_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, _th) = self.terminal_rect();
        (tw - 36.0, ty + 3.0, 28.0, 22.0)
    }

    pub fn terminal_scrollbar_rect(&self) -> (f32, f32, f32, f32) {
        let (_tx, ty, tw, th) = self.terminal_content_rect();
        (tw - 10.0, ty, 6.0, th)
    }

    pub fn terminal_hit_test(&self, mx: f32, my: f32) -> Option<TerminalHit> {
        if !self.terminal.is_open {
            return None;
        }
        let (_tx, ty, tw, th) = self.terminal_rect();
        if my < ty - 4.0 || my > ty + th || mx < 0.0 || mx > tw {
            return None;
        }

        // 1. Manija de redimensionamiento (franja de 6px en el borde superior)
        if my >= ty - 4.0 && my <= ty + 4.0 {
            return Some(TerminalHit::ResizeHandle);
        }

        // 2. Barra de pestañas y botones (Header)
        if my >= ty && my < ty + 28.0 {
            let in_rect = |(rx, ry, rw, rh): (f32, f32, f32, f32)| {
                mx >= rx && mx <= rx + rw && my >= ry && my <= ry + rh
            };
            if in_rect(self.terminal_btn_close_rect()) {
                return Some(TerminalHit::BtnClose);
            }
            if in_rect(self.terminal_btn_zoom_in_rect()) {
                return Some(TerminalHit::BtnZoomIn);
            }
            if in_rect(self.terminal_btn_zoom_label_rect()) {
                return Some(TerminalHit::BtnZoomReset);
            }
            if in_rect(self.terminal_btn_zoom_out_rect()) {
                return Some(TerminalHit::BtnZoomOut);
            }
            if in_rect(self.terminal_btn_clear_rect()) {
                return Some(TerminalHit::BtnClear);
            }
            if in_rect(self.terminal_btn_stop_rect()) {
                return Some(TerminalHit::BtnStop);
            }
            if in_rect(self.terminal_btn_run_rect()) {
                return Some(TerminalHit::BtnRun);
            }
            if in_rect(self.terminal_tab_output_rect()) {
                return Some(TerminalHit::TabOutput);
            }
            if in_rect(self.terminal_tab_shell_rect()) {
                return Some(TerminalHit::TabShell);
            }
            return Some(TerminalHit::Content);
        }

        // 3. Barra de entrada inferior (Input)
        let (_ix, iy, _iw, ih) = self.terminal_input_rect();
        if my >= iy && my <= iy + ih {
            return Some(TerminalHit::Input);
        }

        // 4. Barra de desplazamiento vertical (Scrollbar)
        let (sx, sy, sw, sh) = self.terminal_scrollbar_rect();
        if mx >= sx - 4.0 && mx <= sx + sw + 4.0 && my >= sy && my <= sy + sh {
            let ratio = ((my - sy) / sh).clamp(0.0, 1.0);
            return Some(TerminalHit::Scrollbar(ratio));
        }

        Some(TerminalHit::Content)
    }

    pub fn update_viewport_sizes(&mut self) {
        let w = self.viewport_width - PADDING * 2.0 - GUTTER_WIDTH;
        let term_h = if self.terminal.is_open { self.terminal.height } else { 0.0 };
        let h = (self.viewport_height - TOP_OFFSET - term_h).max(40.0);
        let active = self.active;
        let ln_w = self.gutter_text_width();
        self.documents[active].buffer.set_size(&mut self.font_system, w, h);
        self.line_numbers_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        self.line_numbers_buffer.set_size(&mut self.font_system, ln_w, h);
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    pub fn set_terminal_height(&mut self, height: f32) {
        let clamped = height.clamp(
            crate::terminal::TerminalState::MIN_HEIGHT,
            (self.viewport_height - 100.0).max(120.0),
        );
        self.terminal.height = clamped;
        self.update_viewport_sizes();
        self.rebuild_terminal_buffer();
    }

    pub fn toggle_terminal(&mut self) {
        self.terminal.is_open = !self.terminal.is_open;
        self.terminal_focused = self.terminal.is_open;
        self.update_viewport_sizes();
        if self.terminal.is_open {
            self.rebuild_terminal_buffer();
        }
        self.dirty = true;
    }

    pub fn terminal_scroll(&mut self, delta: i32) {
        let (_cx, _cy, _cw, ch) = self.terminal_content_rect();
        let visible_lines = (ch / self.terminal.eff_line_height()).floor() as usize;
        self.terminal.scroll(delta, visible_lines);
        self.rebuild_terminal_buffer();
    }

    /// Modifica el zoom de la terminal integrada y lo guarda en las configuraciones.
    pub fn terminal_zoom(&mut self, delta: f32) {
        self.terminal.zoom(delta);
        self.settings.terminal_font_scale = self.terminal.font_scale;
        self.settings.save();
        self.rebuild_terminal_buffer();
        self.dirty = true;
    }

    /// Restablece el zoom de la terminal al 100%.
    pub fn terminal_zoom_reset(&mut self) {
        self.terminal.zoom_reset();
        self.settings.terminal_font_scale = self.terminal.font_scale;
        self.settings.save();
        self.rebuild_terminal_buffer();
        self.dirty = true;
    }

    pub fn rebuild_terminal_buffer(&mut self) {
        if !self.terminal.is_open {
            return;
        }

        let font_size = self.terminal.eff_font_size();
        let line_height = self.terminal.eff_line_height();
        let term_metrics = Metrics::new(font_size, line_height);

        // --- 1. Header Buffer (Pestañas) ---
        let (_hx, _hy, hw, hh) = self.terminal_header_rect();
        self.terminal_header_buffer
            .set_metrics(&mut self.font_system, Metrics::new(12.0, 16.0));
        self.terminal_header_buffer
            .set_size(&mut self.font_system, hw, hh);

        let base_attrs = Attrs::new().family(Family::Monospace);
        let normal_tc = self.settings.color_for(Some("topbar_text"));
        let normal_color = Color::rgb(normal_tc[0], normal_tc[1], normal_tc[2]);
        let accent = self.settings.topbar_accent_rgb();
        let accent_color = Color::rgb(
            (accent[0] * 255.0) as u8,
            (accent[1] * 255.0) as u8,
            (accent[2] * 255.0) as u8,
        );
        let green_color = Color::rgb(80, 220, 120);

        let out_active = self.terminal.active_tab == crate::terminal::TerminalTab::Output;
        let shell_active = self.terminal.active_tab == crate::terminal::TerminalTab::Shell;

        let mut header_spans: Vec<(&str, Attrs)> = Vec::new();
        header_spans.push((" ", base_attrs));

        if self.terminal.is_running {
            header_spans.push(("▶ Salida ● ", base_attrs.color(green_color)));
        } else {
            header_spans.push((
                "▶ Salida ",
                base_attrs.color(if out_active { accent_color } else { normal_color }),
            ));
        }

        header_spans.push((
            "   💻 Terminal ",
            base_attrs.color(if shell_active { accent_color } else { normal_color }),
        ));

        self.terminal_header_buffer
            .set_rich_text(&mut self.font_system, header_spans, Shaping::Advanced);
        self.terminal_header_buffer
            .shape_until_scroll(&mut self.font_system);

        // --- 2. Action Buttons Buffers (Botones a la derecha: Run, Stop, Clear, Zoom, Close) ---
        let run_rect = self.terminal_btn_run_rect();
        self.terminal_btn_run_buffer
            .set_metrics(&mut self.font_system, Metrics::new(12.0, 16.0));
        self.terminal_btn_run_buffer
            .set_size(&mut self.font_system, run_rect.2, run_rect.3);
        self.terminal_btn_run_buffer.set_text(
            &mut self.font_system,
            "Ejecutar",
            base_attrs.color(Color::rgb(240, 255, 240)),
            Shaping::Advanced,
        );
        self.terminal_btn_run_buffer
            .shape_until_scroll(&mut self.font_system);

        let stop_rect = self.terminal_btn_stop_rect();
        self.terminal_btn_stop_buffer
            .set_metrics(&mut self.font_system, Metrics::new(12.0, 16.0));
        self.terminal_btn_stop_buffer
            .set_size(&mut self.font_system, stop_rect.2, stop_rect.3);
        let stop_color = if self.terminal.is_running {
            Color::rgb(255, 240, 240)
        } else {
            Color::rgb(180, 180, 185)
        };
        self.terminal_btn_stop_buffer.set_text(
            &mut self.font_system,
            "Detener",
            base_attrs.color(stop_color),
            Shaping::Advanced,
        );
        self.terminal_btn_stop_buffer
            .shape_until_scroll(&mut self.font_system);

        let clear_rect = self.terminal_btn_clear_rect();
        self.terminal_btn_clear_buffer
            .set_metrics(&mut self.font_system, Metrics::new(12.0, 16.0));
        self.terminal_btn_clear_buffer
            .set_size(&mut self.font_system, clear_rect.2, clear_rect.3);
        self.terminal_btn_clear_buffer.set_text(
            &mut self.font_system,
            "Limpiar",
            base_attrs.color(Color::rgb(230, 235, 245)),
            Shaping::Advanced,
        );
        self.terminal_btn_clear_buffer
            .shape_until_scroll(&mut self.font_system);

        let z_out_rect = self.terminal_btn_zoom_out_rect();
        self.terminal_btn_zoom_out_buffer
            .set_metrics(&mut self.font_system, Metrics::new(12.0, 16.0));
        self.terminal_btn_zoom_out_buffer
            .set_size(&mut self.font_system, z_out_rect.2, z_out_rect.3);
        self.terminal_btn_zoom_out_buffer.set_text(
            &mut self.font_system,
            "-",
            base_attrs.color(Color::rgb(230, 235, 245)),
            Shaping::Advanced,
        );
        self.terminal_btn_zoom_out_buffer
            .shape_until_scroll(&mut self.font_system);

        let z_lbl_rect = self.terminal_btn_zoom_label_rect();
        let zoom_pct = (self.terminal.font_scale * 100.0).round() as u32;
        let zoom_str = format!("{zoom_pct}%");
        self.terminal_btn_zoom_lbl_buffer
            .set_metrics(&mut self.font_system, Metrics::new(12.0, 16.0));
        self.terminal_btn_zoom_lbl_buffer
            .set_size(&mut self.font_system, z_lbl_rect.2, z_lbl_rect.3);
        self.terminal_btn_zoom_lbl_buffer.set_text(
            &mut self.font_system,
            &zoom_str,
            base_attrs.color(accent_color),
            Shaping::Advanced,
        );
        self.terminal_btn_zoom_lbl_buffer
            .shape_until_scroll(&mut self.font_system);

        let z_in_rect = self.terminal_btn_zoom_in_rect();
        self.terminal_btn_zoom_in_buffer
            .set_metrics(&mut self.font_system, Metrics::new(12.0, 16.0));
        self.terminal_btn_zoom_in_buffer
            .set_size(&mut self.font_system, z_in_rect.2, z_in_rect.3);
        self.terminal_btn_zoom_in_buffer.set_text(
            &mut self.font_system,
            "+",
            base_attrs.color(Color::rgb(230, 235, 245)),
            Shaping::Advanced,
        );
        self.terminal_btn_zoom_in_buffer
            .shape_until_scroll(&mut self.font_system);

        let close_rect = self.terminal_btn_close_rect();
        self.terminal_btn_close_buffer
            .set_metrics(&mut self.font_system, Metrics::new(12.0, 16.0));
        self.terminal_btn_close_buffer
            .set_size(&mut self.font_system, close_rect.2, close_rect.3);
        self.terminal_btn_close_buffer.set_text(
            &mut self.font_system,
            "✕",
            base_attrs.color(Color::rgb(255, 220, 220)),
            Shaping::Advanced,
        );
        self.terminal_btn_close_buffer
            .shape_until_scroll(&mut self.font_system);

        // --- 3. Output Buffer (Líneas de texto con scroll virtual) ---
        let (_cx, _cy, cw, ch) = self.terminal_content_rect();
        self.terminal_buffer
            .set_metrics(&mut self.font_system, term_metrics);
        self.terminal_buffer
            .set_size(&mut self.font_system, cw - PADDING * 2.0, ch);

        let visible_lines = (ch / line_height).floor() as usize;
        let total_lines = self.terminal.active_lines().len();
        let current_scroll = self.terminal.active_scroll();
        let max_scroll = total_lines.saturating_sub(visible_lines);
        let start = current_scroll.min(max_scroll);
        self.terminal.set_active_scroll(start);

        let lines = self.terminal.active_lines();
        let end = (start + visible_lines + 1).min(total_lines);

        let display_content = if lines.is_empty() {
            match self.terminal.active_tab {
                crate::terminal::TerminalTab::Output => {
                    "No hay salida todavía. Presiona F5 o haz clic en [Ejecutar] para compilar y correr el archivo activo."
                }
                crate::terminal::TerminalTab::Shell => "Terminal interactiva iniciada.",
            }
            .to_string()
        } else {
            lines[start..end].join("\n")
        };

        let is_rgb = self.settings.is_rgb_active(crate::settings::RgbElement::Terminal);
        let text_color = if is_rgb {
            let [r, g, b] = crate::color::chroma_rgb_f32(self.anim_time, 0.0);
            Color::rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
        } else {
            let tc_u8 = self.settings.color_for(Some("terminal_text"));
            Color::rgb(tc_u8[0], tc_u8[1], tc_u8[2])
        };

        self.terminal_buffer.set_text(
            &mut self.font_system,
            &display_content,
            base_attrs.color(text_color),
            Shaping::Advanced,
        );
        self.terminal_buffer
            .shape_until_scroll(&mut self.font_system);

        // --- 4. Input Buffer (Barra de entrada inferior) ---
        let (_ix, _iy, iw, ih) = self.terminal_input_rect();
        self.terminal_input_buffer
            .set_metrics(&mut self.font_system, term_metrics);
        self.terminal_input_buffer
            .set_size(&mut self.font_system, iw - PADDING * 2.0, ih);

        let prompt_prefix = match self.terminal.active_tab {
            crate::terminal::TerminalTab::Output => {
                if self.terminal.is_running {
                    "stdin > "
                } else {
                    "> "
                }
            }
            crate::terminal::TerminalTab::Shell => "cmd > ",
        };

        let input_display = if self.terminal.input_line.is_empty() && !self.terminal_focused {
            format!("{prompt_prefix}(haz clic aquí para interactuar o enviar stdin)")
        } else {
            format!("{prompt_prefix}{}", self.terminal.input_line)
        };

        let input_color = if self.terminal.input_line.is_empty() && !self.terminal_focused {
            Color::rgb(130, 135, 150)
        } else {
            text_color
        };

        self.terminal_input_buffer.set_text(
            &mut self.font_system,
            &input_display,
            base_attrs.color(input_color),
            Shaping::Advanced,
        );
        self.terminal_input_buffer
            .shape_until_scroll(&mut self.font_system);
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
        if !(0.0..=TOOLBAR_HEIGHT).contains(&y) {
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
        spans.push((" ▶ Ejecutar ", base_attrs.color(Color::rgb(70, 215, 120))));
        spans.push((" >_ Terminal ", base_attrs.color(normal_color)));

        self.toolbar_buffer.set_rich_text(&mut self.font_system, spans, Shaping::Advanced);
        self.toolbar_buffer.shape_until_scroll(&mut self.font_system);

        let badge_text = format!("{}  \u{25BE}", self.active_language().short_name());
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
            .map(|(s, c)| (s.as_str(), base_attrs.color(*c)))
            .collect();
        self.menu_buffer.set_rich_text(&mut self.font_system, rich, Shaping::Advanced);
        self.menu_buffer.shape_until_scroll(&mut self.font_system);
    }

    fn build_menu_rows(&self, menu: TopMenu) -> Vec<MenuRow> {
        match menu {
            TopMenu::Archivo => {
                let mut rows = vec![
                    MenuRow { label: " Nuevo".to_string(), shortcut: Some("Ctrl+N"), action: Some(MenuAction::New), is_checked: false },
                    MenuRow { label: " Abrir...".to_string(), shortcut: Some("Ctrl+O"), action: Some(MenuAction::Open), is_checked: false },
                    MenuRow { label: " Guardar".to_string(), shortcut: Some("Ctrl+S"), action: Some(MenuAction::Save), is_checked: false },
                    MenuRow { label: " Guardar como...".to_string(), shortcut: Some("Ctrl+Shift+S"), action: Some(MenuAction::SaveAs), is_checked: false },
                    MenuRow { label: " Guardar todo".to_string(), shortcut: None, action: Some(MenuAction::SaveAll), is_checked: false },
                    MenuRow { label: " Cerrar pestaña".to_string(), shortcut: Some("Ctrl+W"), action: Some(MenuAction::CloseTab), is_checked: false },
                ];
                if !self.settings.recent_files.is_empty() {
                    rows.push(MenuRow {
                        label: " ── Recientes ──".to_string(),
                        shortcut: None,
                        action: None,
                        is_checked: false,
                    });
                    for (i, path) in self.settings.recent_files.iter().take(5).enumerate() {
                        let name = path
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        let short = if name.chars().count() > 24 {
                            format!("{}…", name.chars().take(23).collect::<String>())
                        } else {
                            name
                        };
                        rows.push(MenuRow {
                            label: format!("   {short}"),
                            shortcut: None,
                            action: Some(MenuAction::OpenRecent(i)),
                            is_checked: false,
                        });
                    }
                }
                rows.push(MenuRow { label: " Salir".to_string(), shortcut: Some("Alt+F4"), action: Some(MenuAction::Exit), is_checked: false });
                rows
            }
            TopMenu::Editar => vec![
                MenuRow { label: " Deshacer".to_string(), shortcut: Some("Ctrl+Z"), action: Some(MenuAction::Undo), is_checked: false },
                MenuRow { label: " Rehacer".to_string(), shortcut: Some("Ctrl+Y"), action: Some(MenuAction::Redo), is_checked: false },
                MenuRow { label: " Cortar".to_string(), shortcut: Some("Ctrl+X"), action: Some(MenuAction::Cut), is_checked: false },
                MenuRow { label: " Copiar".to_string(), shortcut: Some("Ctrl+C"), action: Some(MenuAction::Copy), is_checked: false },
                MenuRow { label: " Pegar".to_string(), shortcut: Some("Ctrl+V"), action: Some(MenuAction::Paste), is_checked: false },
                MenuRow { label: " Indentar".to_string(), shortcut: Some("Tab"), action: Some(MenuAction::Indent), is_checked: false },
                MenuRow { label: " Desindentar".to_string(), shortcut: Some("Shift+Tab"), action: Some(MenuAction::Unindent), is_checked: false },
                MenuRow { label: " Duplicar línea".to_string(), shortcut: Some("Ctrl+D"), action: Some(MenuAction::DuplicateLine), is_checked: false },
                MenuRow { label: " Comentar línea".to_string(), shortcut: Some("Ctrl+/"), action: Some(MenuAction::ToggleComment), is_checked: false },
                MenuRow { label: " Mover línea arriba".to_string(), shortcut: Some("Alt+↑"), action: Some(MenuAction::MoveLineUp), is_checked: false },
                MenuRow { label: " Mover línea abajo".to_string(), shortcut: Some("Alt+↓"), action: Some(MenuAction::MoveLineDown), is_checked: false },
                MenuRow { label: " Alternar marcador".to_string(), shortcut: Some("F2"), action: Some(MenuAction::ToggleBookmark), is_checked: false },
                MenuRow { label: " Seleccionar todo".to_string(), shortcut: Some("Ctrl+A"), action: Some(MenuAction::SelectAll), is_checked: false },
            ],
            TopMenu::Buscar => vec![
                MenuRow { label: " Buscar en archivo...".to_string(), shortcut: Some("Ctrl+F"), action: Some(MenuAction::Find), is_checked: false },
                MenuRow { label: " Buscar siguiente".to_string(), shortcut: Some("F3"), action: Some(MenuAction::FindNext), is_checked: false },
                MenuRow { label: " Reemplazar...".to_string(), shortcut: Some("Ctrl+H"), action: Some(MenuAction::Replace), is_checked: false },
                MenuRow { label: " Ir a línea...".to_string(), shortcut: Some("Ctrl+G"), action: Some(MenuAction::GoToLine), is_checked: false },
            ],
            TopMenu::Ver => vec![
                MenuRow { label: " Acercar zoom".to_string(), shortcut: Some("Ctrl++"), action: Some(MenuAction::ZoomIn), is_checked: false },
                MenuRow { label: " Alejar zoom".to_string(), shortcut: Some("Ctrl+-"), action: Some(MenuAction::ZoomOut), is_checked: false },
                MenuRow { label: " Restablecer zoom".to_string(), shortcut: Some("Ctrl+0"), action: Some(MenuAction::ZoomReset), is_checked: false },
                MenuRow { label: " Alternar terminal".to_string(), shortcut: Some("Ctrl+J"), action: Some(MenuAction::ToggleTerminal), is_checked: self.terminal.is_open },
            ],
            TopMenu::Lenguaje => {
                let current = self.active_language();
                Language::ALL_POPULAR
                    .iter()
                    .map(|&lang| {
                        let is_checked = lang == current;
                        let prefix = if is_checked { " \u{2713} " } else { "   " };
                        let label = format!("{prefix}{}", lang.display_name());
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
        self.prompt_field = 0;
        let is_search = matches!(purpose, PromptPurpose::Find | PromptPurpose::Replace);
        if !is_search {
            self.prompt_text.clear();
        } else {
            // Si hay texto seleccionado de una sola línea, usarlo directo como consulta inicial
            if let Some(sel) = self.selected_text() {
                if !sel.contains('\n') && !sel.is_empty() && sel.chars().count() <= 48 {
                    self.prompt_text = sel;
                }
            }
            if self.prompt_text.is_empty() {
                self.prompt_text = self.last_search_query.clone();
            }
        }
        self.prompt_label = label.to_string();
        self.prompt_purpose = Some(purpose);
        if !is_search {
            self.prompt_x = None;
            self.prompt_y = None;
        } else if self.prompt_x.is_none() {
            let w = if purpose == PromptPurpose::Replace { 360.0 } else { 340.0 };
            self.prompt_x = Some((self.viewport_width - w - 24.0).max(8.0));
            self.prompt_y = Some(TOP_OFFSET + 8.0);
        }
        self.rebuild_prompt_buffer();
    }

    pub fn close_prompt(&mut self) {
        self.prompt_open = false;
        self.prompt_purpose = None;
        self.prompt_field = 0;
    }

    pub fn prompt_switch_field(&mut self) {
        if self.prompt_purpose == Some(PromptPurpose::Replace) {
            self.prompt_field = 1 - self.prompt_field;
            self.rebuild_prompt_buffer();
        }
    }

    pub fn prompt_set_field(&mut self, field: usize) {
        if self.prompt_purpose == Some(PromptPurpose::Replace) {
            self.prompt_field = field.min(1);
            self.rebuild_prompt_buffer();
        }
    }

    fn active_prompt_buf_mut(&mut self) -> &mut String {
        if self.prompt_purpose == Some(PromptPurpose::Replace) && self.prompt_field == 1 {
            &mut self.prompt_replace_text
        } else {
            &mut self.prompt_text
        }
    }

    pub fn prompt_push_char(&mut self, c: char) {
        let buf = self.active_prompt_buf_mut();
        if buf.chars().count() < 64 {
            buf.push(c);
            self.rebuild_prompt_buffer();
        }
    }

    pub fn prompt_push_str(&mut self, s: &str) {
        let buf = self.active_prompt_buf_mut();
        for c in s.chars() {
            if !c.is_control() && buf.chars().count() < 64 {
                buf.push(c);
            }
        }
        self.rebuild_prompt_buffer();
    }

    pub fn prompt_backspace(&mut self) {
        self.active_prompt_buf_mut().pop();
        self.rebuild_prompt_buffer();
    }

    pub fn prompt_delete_word_left(&mut self) {
        let mut chars: Vec<char> = self.active_prompt_buf_mut().chars().collect();
        while let Some(&c) = chars.last() {
            if c.is_whitespace() {
                chars.pop();
            } else {
                break;
            }
        }
        if let Some(&last) = chars.last() {
            let is_word = last.is_alphanumeric() || last == '_';
            while let Some(&c) = chars.last() {
                let c_word = c.is_alphanumeric() || c == '_';
                if (is_word && c_word) || (!is_word && !c.is_whitespace() && !c_word) {
                    chars.pop();
                } else {
                    break;
                }
            }
        }
        *self.active_prompt_buf_mut() = chars.into_iter().collect();
        self.rebuild_prompt_buffer();
    }

    /// Confirma la cajita (Enter): devuelve para qué se estaba usando y
    /// el texto escrito, y la cierra (excepto en Find/Replace para seguir buscando).
    pub fn prompt_confirm(&mut self) -> Option<(PromptPurpose, String)> {
        let purpose = self.prompt_purpose?;
        let text = self.prompt_text.clone();
        if !matches!(purpose, PromptPurpose::Find | PromptPurpose::Replace) {
            self.close_prompt();
        }
        Some((purpose, text))
    }

    pub fn prompt_purpose(&self) -> Option<PromptPurpose> {
        self.prompt_purpose
    }

    pub fn get_prompt_text(&self) -> &str {
        &self.prompt_text
    }

    /// Si la selección actual coincide (insensible a mayúsculas) con `prompt_text`,
    /// la reemplaza por `prompt_replace_text` y salta a la siguiente coincidencia;
    /// si no, busca primero la siguiente coincidencia.
    pub fn replace_current_and_find_next(&mut self) -> bool {
        let query = self.prompt_text.clone();
        if query.is_empty() {
            return false;
        }
        let replacement = self.prompt_replace_text.clone();
        let matches_selected = self
            .selected_text()
            .map(|sel| sel.to_lowercase() == query.to_lowercase())
            .unwrap_or(false);

        if matches_selected {
            self.paste_text(&replacement);
            let found_next = self.find_next(&query);
            if !found_next {
                self.last_search_status = "Reemplazado (sin más coincidencias)".to_string();
                if self.prompt_open {
                    self.rebuild_prompt_buffer();
                }
            }
            true
        } else {
            self.find_next(&query)
        }
    }

    /// Reemplaza todas las coincidencias (case-insensitive, UTF-8 seguro) de `query`
    /// por `replacement` en un solo paso de deshacer.
    pub fn replace_all_with(&mut self, query: &str, replacement: &str) -> usize {
        if query.is_empty() {
            return 0;
        }
        self.last_search_query = query.to_string();
        let query_lower_chars: Vec<char> = query.to_lowercase().chars().collect();
        let qlen = query_lower_chars.len();
        if qlen == 0 {
            return 0;
        }

        let mut total_replaced = 0usize;
        let mut new_lines = Vec::with_capacity(self.documents[self.active].lines.len());
        for line in &self.documents[self.active].lines {
            let line_chars: Vec<char> = line.chars().collect();
            let mut out = String::with_capacity(line.len());
            let mut pos = 0usize;
            while pos < line_chars.len() {
                if let Some(found) = Self::find_in_line_ci(line, &query_lower_chars, pos) {
                    out.extend(line_chars[pos..found].iter());
                    out.push_str(replacement);
                    pos = found + qlen;
                    total_replaced += 1;
                } else {
                    out.extend(line_chars[pos..].iter());
                    break;
                }
            }
            new_lines.push(out);
        }

        if total_replaced > 0 {
            self.push_undo_if_needed(EditKind::Other);
            let doc = &mut self.documents[self.active];
            doc.lines = new_lines;
            doc.cursor_line = doc.cursor_line.min(doc.lines.len().saturating_sub(1));
            doc.cursor_col = doc.cursor_col.min(doc.lines[doc.cursor_line].chars().count());
            doc.selection_anchor = None;
            self.last_search_status = format!("Reemplazadas {total_replaced} coincidencias");
            self.mark_active_dirty();
            self.ensure_cursor_visible();
        } else {
            self.last_search_status = "Sin coincidencias".to_string();
        }
        if self.prompt_open {
            self.rebuild_prompt_buffer();
        }
        total_replaced
    }

    pub fn replace_all(&mut self) -> usize {
        let q = self.prompt_text.clone();
        let r = self.prompt_replace_text.clone();
        self.replace_all_with(&q, &r)
    }

    /// Búsqueda insensible a mayúsculas 100% segura con UTF-8 (sin mezclar
    /// índices de bytes entre cadenas con distinta longitud al convertir a minúsculas).
    fn find_in_line_ci(line: &str, query_lower_chars: &[char], start_char: usize) -> Option<usize> {
        if query_lower_chars.is_empty() {
            return None;
        }
        let line_chars: Vec<char> = line.chars().collect();
        let qlen = query_lower_chars.len();
        if start_char + qlen > line_chars.len() {
            return None;
        }
        // Comparamos carácter a carácter en minúscula (c.to_lowercase() para soportar acentos/ñ).
        'outer: for pos in start_char..=(line_chars.len() - qlen) {
            for k in 0..qlen {
                let mut lc = line_chars[pos + k].to_lowercase();
                if lc.next() != Some(query_lower_chars[k]) || lc.next().is_some() {
                    continue 'outer;
                }
            }
            return Some(pos);
        }
        None
    }

    fn count_matches_in_doc(&self, query_lower_chars: &[char]) -> (usize, usize) {
        let doc = &self.documents[self.active];
        let mut total = 0usize;
        let mut current = 0usize;
        let cur = (doc.cursor_line, doc.cursor_col);
        let qlen = query_lower_chars.len();
        for (li, line) in doc.lines.iter().enumerate() {
            let mut start = 0usize;
            while let Some(pos) = Self::find_in_line_ci(line, query_lower_chars, start) {
                total += 1;
                if (li, pos + qlen) == cur {
                    current = total;
                }
                start = pos + qlen.max(1);
            }
        }
        (current, total)
    }

    /// Busca la siguiente aparición del texto dado (case-insensitive, UTF-8 seguro).
    pub fn find_next(&mut self, query: &str) -> bool {
        if query.is_empty() {
            self.last_search_status.clear();
            return false;
        }
        self.last_search_query = query.to_string();
        let query_lower_chars: Vec<char> = query.to_lowercase().chars().collect();
        let match_chars_len = query_lower_chars.len();

        let (found_pos, total_lines) = {
            let doc = &self.documents[self.active];
            let total_lines = doc.lines.len();
            if total_lines == 0 {
                return false;
            }
            let cur_line = doc.cursor_line.min(total_lines - 1);
            let cur_col = doc.cursor_col;

            let mut found = None;
            for offset in 0..total_lines {
                let line_idx = (cur_line + offset) % total_lines;
                let start_col = if offset == 0 { cur_col } else { 0 };
                if let Some(col) = Self::find_in_line_ci(&doc.lines[line_idx], &query_lower_chars, start_col) {
                    found = Some((line_idx, col));
                    break;
                }
            }
            if found.is_none() {
                if let Some(col) = Self::find_in_line_ci(&doc.lines[cur_line], &query_lower_chars, 0) {
                    if col < cur_col {
                        found = Some((cur_line, col));
                    }
                }
            }
            (found, total_lines)
        };

        if total_lines == 0 {
            return false;
        }

        if let Some((line_idx, match_col)) = found_pos {
            let doc = &mut self.documents[self.active];
            doc.cursor_line = line_idx;
            doc.cursor_col = match_col + match_chars_len;
            doc.selection_anchor = Some((line_idx, match_col));
            self.search_result_marker = Some((line_idx, self.anim_time + 3.0));
            let (cur_idx, total) = self.count_matches_in_doc(&query_lower_chars);
            self.last_search_status = format!("Coincidencia {cur_idx} de {total}");
            self.center_cursor_in_view();
            self.dirty = true;
            if self.prompt_open
                && matches!(self.prompt_purpose, Some(PromptPurpose::Find | PromptPurpose::Replace))
            {
                self.rebuild_prompt_buffer();
            }
            return true;
        }

        self.last_search_status = "Sin coincidencias".to_string();
        if self.prompt_open
            && matches!(self.prompt_purpose, Some(PromptPurpose::Find | PromptPurpose::Replace))
        {
            self.rebuild_prompt_buffer();
        }
        false
    }

    pub fn goto_line(&mut self, line_1_based: usize) {
        let doc = &mut self.documents[self.active];
        if doc.lines.is_empty() {
            return;
        }
        let target = line_1_based.saturating_sub(1).min(doc.lines.len() - 1);
        doc.cursor_line = target;
        doc.cursor_col = 0;
        doc.selection_anchor = None;
        self.search_result_marker = Some((target, self.anim_time + 2.5));
        self.center_cursor_in_view();
        self.dirty = true;
    }

    pub fn prompt_rect(&self) -> (f32, f32, f32, f32) {
        let is_replace = self.prompt_purpose == Some(PromptPurpose::Replace);
        let is_search = matches!(self.prompt_purpose, Some(PromptPurpose::Find | PromptPurpose::Replace));
        let w = if is_replace { 360.0 } else { 340.0 };
        let h = if is_replace {
            210.0
        } else if self.prompt_purpose == Some(PromptPurpose::Find) {
            150.0
        } else {
            120.0
        };
        let x = self.prompt_x.unwrap_or_else(|| {
            if is_search {
                (self.viewport_width - w - 24.0).max(8.0)
            } else {
                ((self.viewport_width - w) / 2.0).max(8.0)
            }
        });
        let y = self.prompt_y.unwrap_or(if is_search {
            TOP_OFFSET + 8.0
        } else {
            TOP_OFFSET + 60.0
        });
        (
            x.clamp(4.0, (self.viewport_width - w - 4.0).max(4.0)),
            y.clamp(TOP_OFFSET, (self.viewport_height - h - 4.0).max(TOP_OFFSET)),
            w,
            h,
        )
    }

    fn rebuild_prompt_buffer(&mut self) {
        let (text, w, h) = match self.prompt_purpose {
            Some(PromptPurpose::Find) => {
                let status = if self.last_search_status.is_empty() {
                    "Enter o botón para buscar".to_string()
                } else {
                    self.last_search_status.clone()
                };
                (
                    format!(
                        "{}\n{}_\n{}\n[ Buscar siguiente ]",
                        self.prompt_label, self.prompt_text, status
                    ),
                    320.0,
                    140.0,
                )
            }
            Some(PromptPurpose::Replace) => {
                let status = if self.last_search_status.is_empty() {
                    "Tab cambia campo · Enter reemplaza".to_string()
                } else {
                    self.last_search_status.clone()
                };
                let (m0, c0, m1, c1) = if self.prompt_field == 0 {
                    ("> ", "_", "  ", "")
                } else {
                    ("  ", "", "> ", "_")
                };
                (
                    format!(
                        "Buscar:\n{m0}{}{c0}\nReemplazar por:\n{m1}{}{c1}\n{}\n[ Reemplazar ]  [ Todo ]",
                        self.prompt_text, self.prompt_replace_text, status
                    ),
                    340.0,
                    196.0,
                )
            }
            Some(PromptPurpose::GoToLine) => (
                format!("{}\n{}_\n\n[ Ir a línea ]", self.prompt_label, self.prompt_text),
                320.0,
                110.0,
            ),
            _ => (
                format!("{}\n{}_\n\n[ Guardar preset ]", self.prompt_label, self.prompt_text),
                320.0,
                110.0,
            ),
        };
        self.prompt_buffer.set_size(&mut self.font_system, w, h);
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
            if my > py + ph - 38.0 {
                if self.prompt_purpose == Some(PromptPurpose::Replace) && mx > px + pw * 0.52 {
                    return Some(PromptHit::SecondaryButton);
                }
                return Some(PromptHit::Button);
            }
            if self.prompt_purpose == Some(PromptPurpose::Replace) {
                let rel_y = my - py;
                if (34.0..66.0).contains(&rel_y) {
                    return Some(PromptHit::Field(0));
                }
                if (84.0..118.0).contains(&rel_y) {
                    return Some(PromptHit::Field(1));
                }
            }
            return Some(PromptHit::Drag);
        }
        None
    }

    pub fn load_preset(&mut self, index: usize) {
        self.settings.apply_preset(index);
        self.settings.save();
        self.dirty = true;
        self.rebuild_panel_buffer();
        self.rebuild_toolbar_buffer();
        self.rebuild_tab_buffer();
        self.rebuild_status_buffer();
        if self.terminal.is_open {
            self.rebuild_terminal_buffer();
        }
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
        let pairs_str = if self.settings.auto_close_pairs { "ON" } else { "OFF" };
        rows.push(item(&format!("Autocierre de pares: [{pairs_str}]"), PanelAction::ToggleAutoClosePairs));

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

        rows.push(header("MENÚS DESPLEGABLES"));
        let menu_color = self.settings.color_for(Some("menu_bg"));
        rows.push(PanelRow {
            label: "   Color fondo menús".to_string(),
            action: Some(PanelAction::EditColor("menu_bg")),
            swatch: Some(menu_color),
        });
        let menu_op_pct = (self.settings.menu_opacity * 100.0).round() as u32;
        let menu_op_str = if menu_op_pct >= 99 { "100% (Sólido)".to_string() } else if menu_op_pct == 0 { "0% (Invisible)".to_string() } else { format!("{menu_op_pct}%") };
        rows.push(item(&format!("Opacidad menús: {menu_op_str}"), PanelAction::CycleMenuOpacity));

        rows.push(header("SCROLLBAR (DERECHA)"));
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

        rows.push(header("TERMINAL"));
        let term_op_pct = (self.settings.terminal_opacity * 100.0).round() as u32;
        let term_op_str = if term_op_pct >= 99 { "100% (Sólido)".to_string() } else if term_op_pct == 0 { "0% (Invisible)".to_string() } else { format!("{term_op_pct}%") };
        rows.push(item(&format!("Opacidad terminal: {term_op_str}"), PanelAction::CycleTerminalOpacity));
        rows.push(PanelRow {
            label: "   Fondo terminal".to_string(),
            action: Some(PanelAction::EditColor("terminal_bg")),
            swatch: Some(self.settings.color_for(Some("terminal_bg"))),
        });
        rows.push(PanelRow {
            label: "   Texto terminal".to_string(),
            action: Some(PanelAction::EditColor("terminal_text")),
            swatch: Some(self.settings.color_for(Some("terminal_text"))),
        });

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
        let max_y = self.editor_bottom();
        let line_h = self.eff_line_height();
        let mut res = Vec::new();
        for line in start..end {
            if doc.bookmarks.contains(&line) {
                let visible_idx = line - start;
                let y = TOP_OFFSET + PADDING + visible_idx as f32 * line_h;
                if y + line_h <= max_y {
                    res.push((line, y));
                }
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
        let max_y = self.editor_bottom();
        let line_h = self.eff_line_height();
        if line >= start && line < end {
            let visible_idx = line - start;
            let y = TOP_OFFSET + PADDING + visible_idx as f32 * line_h;
            if y + line_h > max_y {
                return None;
            }
            let remaining = expiration - self.anim_time;
            let alpha = if remaining < 1.0 { remaining.max(0.0) } else { 1.0 };
            Some((y, alpha))
        } else {
            None
        }
    }

    pub fn active_line_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let doc = &self.documents[self.active];
        if doc.cursor_line < doc.scroll_offset {
            return None;
        }
        let visible_idx = doc.cursor_line - doc.scroll_offset;
        let line_h = self.eff_line_height();
        let y = TOP_OFFSET + PADDING + visible_idx as f32 * line_h;
        if y + line_h > self.editor_bottom() {
            return None;
        }
        let x = PADDING + GUTTER_WIDTH;
        let w = (self.viewport_width - x - SCROLLBAR_WIDTH).max(0.0);
        Some((x, y, w, line_h))
    }

    pub fn scrollbar_rect(&self) -> (f32, f32, f32, f32) {
        (
            self.viewport_width - SCROLLBAR_WIDTH,
            TOP_OFFSET,
            SCROLLBAR_WIDTH,
            (self.editor_bottom() - TOP_OFFSET).max(0.0),
        )
    }

    pub fn scrollbar_thumb_rect(&self) -> (f32, f32, f32, f32) {
        let doc = &self.documents[self.active];
        let total_lines = doc.lines.len();
        let visible = self.visible_lines();
        let track_h = (self.editor_bottom() - TOP_OFFSET).max(10.0);
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
        x >= self.viewport_width - SCROLLBAR_WIDTH
            && x <= self.viewport_width
            && y >= TOP_OFFSET
            && y <= self.editor_bottom()
    }

    pub fn scrollbar_scroll_to_y(&mut self, y: f32) {
        let track_h = (self.editor_bottom() - TOP_OFFSET).max(10.0);
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
        if !(0.0..=(PADDING + GUTTER_WIDTH)).contains(&x)
            || y < TOP_OFFSET + PADDING
            || y >= self.editor_bottom()
        {
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
            .map(|(s, c)| (s.as_str(), base_attrs.color(*c)))
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

    /// Texto listo para guardar en disco, preservando saltos CRLF si el archivo
    /// original de Windows los utilizaba.
    pub fn full_text_for_save(&self) -> String {
        let doc = &self.documents[self.active];
        if doc.uses_crlf {
            doc.lines.join("\r\n")
        } else {
            doc.lines.join("\n")
        }
    }

    pub fn completion_visible(&self) -> bool {
        self.completion_visible
    }

    pub fn completion_hit_test(&self, mx: f32, my: f32) -> Option<usize> {
        let (rx, ry, rw, rh) = self.completion_rect()?;
        if mx < rx || mx > rx + rw || my < ry + 4.0 || my > ry + rh - 4.0 {
            return None;
        }
        let row_h = LINE_HEIGHT * 0.85;
        let idx = ((my - (ry + 4.0)) / row_h) as usize;
        if idx < self.completion_items.len().min(12) {
            Some(idx)
        } else {
            None
        }
    }

    pub fn completion_select_and_accept(&mut self, idx: usize) {
        if idx < self.completion_items.len() {
            self.completion_selected = idx;
            self.completion_accept();
        }
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
            self.push_undo_if_needed(EditKind::Other);
            let doc = &mut self.documents[self.active];
            let chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
            let cur = doc.cursor_col.min(chars.len());
            // Reemplazar el prefijo parcial de identificador ya escrito (ej. "pri" -> "printf")
            let mut prefix_start = cur;
            while prefix_start > 0 && (chars[prefix_start - 1].is_alphanumeric() || chars[prefix_start - 1] == '_') {
                prefix_start -= 1;
            }
            let mut new_line = String::new();
            new_line.extend(chars[..prefix_start].iter());
            new_line.push_str(&item.insert_text);
            new_line.extend(chars[cur..].iter());
            doc.lines[doc.cursor_line] = new_line;
            doc.cursor_col = prefix_start + item.insert_text.chars().count();
            doc.selection_anchor = None;
            self.mark_active_dirty();
            self.ensure_cursor_visible();
        }
        self.hide_completions();
    }

    fn rebuild_completion_buffer(&mut self) {
        let base_attrs = Attrs::new().family(Family::Monospace);
        let lines: Vec<String> = self
            .completion_items
            .iter()
            .take(12)
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
            spans.push((&text[offset..end], base_attrs.color(color)));
            offset = end;
            if i + 1 < lines.len() {
                spans.push(("\n", base_attrs));
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

    pub fn x_for_line_col(&self, line: usize, col: usize) -> f32 {
        let doc = &self.documents[self.active];
        let char_width = self.char_width();
        let base_x = PADDING + GUTTER_WIDTH - doc.scroll_x;
        if line >= doc.scroll_offset {
            let line_in_view = line - doc.scroll_offset;
            if let Some(line_str) = doc.lines.get(line) {
                for run in doc.buffer.layout_runs() {
                    if run.line_i == line_in_view {
                        let target_byte = line_str
                            .char_indices()
                            .nth(col)
                            .map(|(i, _)| i)
                            .unwrap_or(line_str.len());

                        if target_byte == 0 {
                            return base_x;
                        } else if target_byte >= line_str.len() {
                            if let Some(last) = run.glyphs.last() {
                                return base_x + last.x + last.w;
                            } else if line_str.is_empty() {
                                return base_x;
                            }
                        } else {
                            for g in run.glyphs {
                                if (g.start <= target_byte && target_byte < g.end)
                                    || g.start >= target_byte
                                {
                                    return base_x + g.x;
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
        base_x + col as f32 * char_width
    }

    fn cursor_pixel_pos(&self) -> (f32, f32) {
        let doc = &self.documents[self.active];
        let visible_line = doc.cursor_line.saturating_sub(doc.scroll_offset);
        let y = TOP_OFFSET + PADDING + visible_line as f32 * self.eff_line_height();
        let x = self.x_for_line_col(doc.cursor_line, doc.cursor_col);
        (x, y)
    }

    /// Rectángulo (x, y, ancho, alto) del cursor visual: una barrita
    /// delgada tipo Word/Notepad, no un bloque grueso tipo terminal.
    pub fn cursor_rect(&self) -> (f32, f32, f32, f32) {
        let (x, y) = self.cursor_pixel_pos();
        (x, y, 2.0, self.eff_line_height() * 0.85)
    }

    pub fn cursor_visible(&self) -> bool {
        if !self.cursor_blink_on || self.terminal_focused || self.prompt_open {
            return false;
        }
        let doc = &self.documents[self.active];
        if doc.cursor_line < doc.scroll_offset {
            return false;
        }
        let (x, y, _w, h) = self.cursor_rect();
        x >= PADDING + GUTTER_WIDTH - 2.0
            && x <= self.viewport_width - SCROLLBAR_WIDTH
            && y >= TOP_OFFSET
            && y + h <= self.editor_bottom()
    }

    /// Devuelve los rectángulos de los delimitadores coincidentes `()`, `[]`, `{}`
    /// cuando el cursor está adyacente a uno de ellos.
    pub fn matching_bracket_rects(&self) -> Vec<(f32, f32, f32, f32)> {
        let doc = &self.documents[self.active];
        let Some(cur_line_str) = doc.lines.get(doc.cursor_line) else {
            return Vec::new();
        };
        let cur_chars: Vec<char> = cur_line_str.chars().collect();
        let candidate = if doc.cursor_col > 0 {
            cur_chars
                .get(doc.cursor_col - 1)
                .copied()
                .filter(|c| matches!(c, '(' | ')' | '[' | ']' | '{' | '}'))
                .map(|c| (doc.cursor_line, doc.cursor_col - 1, c))
        } else {
            None
        }
        .or_else(|| {
            cur_chars
                .get(doc.cursor_col)
                .copied()
                .filter(|c| matches!(c, '(' | ')' | '[' | ']' | '{' | '}'))
                .map(|c| (doc.cursor_line, doc.cursor_col, c))
        });

        let Some((src_line, src_col, ch)) = candidate else {
            return Vec::new();
        };

        let (target_ch, forward) = match ch {
            '(' => (')', true),
            '[' => (']', true),
            '{' => ('}', true),
            ')' => ('(', false),
            ']' => ('[', false),
            '}' => ('{', false),
            _ => return Vec::new(),
        };

        let mut depth = 0i32;
        let mut matched: Option<(usize, usize)> = None;
        if forward {
            let max_line = (src_line + 400).min(doc.lines.len());
            'fwd: for li in src_line..max_line {
                let start_c = if li == src_line { src_col } else { 0 };
                for (ci, c) in doc.lines[li].chars().enumerate().skip(start_c) {
                    if c == ch {
                        depth += 1;
                    } else if c == target_ch {
                        depth -= 1;
                        if depth == 0 {
                            matched = Some((li, ci));
                            break 'fwd;
                        }
                    }
                }
            }
        } else {
            let min_line = src_line.saturating_sub(400);
            'bwd: for li in (min_line..=src_line).rev() {
                let chars: Vec<char> = doc.lines[li].chars().collect();
                let end_c = if li == src_line {
                    src_col.min(chars.len().saturating_sub(1))
                } else {
                    chars.len().saturating_sub(1)
                };
                if chars.is_empty() {
                    continue;
                }
                for ci in (0..=end_c).rev() {
                    let c = chars[ci];
                    if c == ch {
                        depth += 1;
                    } else if c == target_ch {
                        depth -= 1;
                        if depth == 0 {
                            matched = Some((li, ci));
                            break 'bwd;
                        }
                    }
                }
            }
        }

        let Some((m_line, m_col)) = matched else {
            return Vec::new();
        };

        let line_h = self.eff_line_height();
        let char_w = self.char_width();
        let max_y = self.editor_bottom();
        let min_x = PADDING + GUTTER_WIDTH;
        let max_x = self.viewport_width - SCROLLBAR_WIDTH;
        let mut out = Vec::with_capacity(2);
        for (li, ci) in [(src_line, src_col), (m_line, m_col)] {
            if li < doc.scroll_offset {
                continue;
            }
            let y = TOP_OFFSET + PADDING + (li - doc.scroll_offset) as f32 * line_h;
            if y + line_h > max_y {
                continue;
            }
            let x = self.x_for_line_col(li, ci);
            if x >= min_x - 1.0 && x + char_w <= max_x + 1.0 {
                out.push((x, y, char_w, line_h));
            }
        }
        out
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
        let (cx, cy) = self.cursor_pixel_pos();
        let row_h = LINE_HEIGHT * 0.85;
        let pop_w = 320.0;
        let pop_h = (self.completion_items.len().min(12) as f32 * row_h + 12.0).max(32.0);
        let x = cx.min((self.viewport_width - pop_w - 16.0).max(8.0));
        let below_y = cy + self.eff_line_height();
        let y = if below_y + pop_h > self.editor_bottom() {
            (cy - pop_h).max(TOP_OFFSET + 4.0)
        } else {
            below_y
        };
        (x, y)
    }

    pub fn completion_rect(&self) -> Option<(f32, f32, f32, f32)> {
        if !self.completion_visible || self.completion_items.is_empty() {
            return None;
        }
        let (x, y) = self.completion_anchor_px();
        let row_h = LINE_HEIGHT * 0.85;
        let h = (self.completion_items.len().min(12) as f32 * row_h + 12.0).max(32.0);
        Some((x - 6.0, y - 4.0, 332.0, h))
    }

    pub fn completion_selected_row_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let (rx, ry, rw, _rh) = self.completion_rect()?;
        let row_h = LINE_HEIGHT * 0.85;
        let row_y = ry + 4.0 + self.completion_selected as f32 * row_h;
        Some((rx + 3.0, row_y, rw - 6.0, row_h))
    }

    pub fn terminal_input_cursor_x(&self) -> f32 {
        let prefix_len = match self.terminal.active_tab {
            crate::terminal::TerminalTab::Output => {
                if self.terminal.is_running { 8 } else { 2 }
            }
            crate::terminal::TerminalTab::Shell => 6,
        };
        let char_w = self.terminal.eff_font_size() * 0.6;
        PADDING + (prefix_len + self.terminal.input_line.chars().count()) as f32 * char_w
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
        doc.cursor_col = prev.cursor_col.min(doc.lines[doc.cursor_line].chars().count());
        doc.selection_anchor = None;
        doc.last_edit_kind = EditKind::None;
        self.mark_active_dirty();
        self.ensure_cursor_visible();
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
        doc.cursor_col = next.cursor_col.min(doc.lines[doc.cursor_line].chars().count());
        doc.selection_anchor = None;
        doc.last_edit_kind = EditKind::None;
        self.mark_active_dirty();
        self.ensure_cursor_visible();
    }

    fn closing_pair(open: char) -> Option<char> {
        match open {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            '`' => Some('`'),
            _ => None,
        }
    }

    fn pairs_enabled(&self) -> bool {
        self.settings.auto_close_pairs && self.documents[self.active].language != Language::PlainText
    }

    pub fn insert_char(&mut self, c: char) {
        let pairs_on = self.pairs_enabled();

        // Si hay selección activa y se escribe un delimitador de apertura, envolver la selección
        if pairs_on {
            if let (Some(close), Some((start, end))) = (Self::closing_pair(c), self.selection_range()) {
                if let Some(sel_text) = self.selected_text() {
                    self.push_undo_if_needed(EditKind::Other);
                    self.paste_in_progress = true;
                    self.delete_selection();
                    self.paste_in_progress = false;
                    let wrapped = format!("{c}{sel_text}{close}");
                    let line_start = start.0;
                    let col_start = start.1;
                    self.paste_in_progress = true;
                    let doc = &mut self.documents[self.active];
                    let normalized = wrapped.replace("\r\n", "\n");
                    let parts: Vec<&str> = normalized.split('\n').collect();
                    let chars: Vec<char> = doc.lines[line_start].chars().collect();
                    let at = col_start.min(chars.len());
                    let before: String = chars[..at].iter().collect();
                    let after: String = chars[at..].iter().collect();
                    if parts.len() == 1 {
                        doc.lines[line_start] = format!("{before}{}{after}", parts[0]);
                        doc.selection_anchor = Some((line_start, at + 1));
                        doc.cursor_line = line_start;
                        doc.cursor_col = at + 1 + sel_text.chars().count();
                    } else {
                        let mut repl = Vec::with_capacity(parts.len());
                        repl.push(format!("{before}{}", parts[0]));
                        for mid in &parts[1..parts.len() - 1] {
                            repl.push((*mid).to_string());
                        }
                        let last = parts[parts.len() - 1];
                        repl.push(format!("{last}{after}"));
                        doc.lines.splice(line_start..=line_start, repl);
                        doc.selection_anchor = Some((line_start, at + 1));
                        doc.cursor_line = end.0;
                        doc.cursor_col = last.chars().count().saturating_sub(1);
                    }
                    self.paste_in_progress = false;
                    self.mark_active_dirty();
                    self.ensure_cursor_visible();
                    return;
                }
            }
        }

        if self.selection_range().is_some() {
            self.delete_selection();
        } else {
            self.documents[self.active].selection_anchor = None;
        }

        if pairs_on {
            let doc = &mut self.documents[self.active];
            let chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
            let at = doc.cursor_col.min(chars.len());
            let next_ch = chars.get(at).copied();
            let prev_ch = if at > 0 { chars.get(at - 1).copied() } else { None };

            // Saltar sobre un delimitador de cierre que ya está justo delante del cursor
            if matches!(c, ')' | ']' | '}' | '"' | '\'' | '`') && next_ch == Some(c) {
                doc.cursor_col = at + 1;
                doc.last_edit_kind = EditKind::None;
                self.dirty = true;
                self.ensure_cursor_visible();
                return;
            }

            // Autocierre inteligente (evitar cerrar comilla simple en lifetimes Rust o contracciones)
            if let Some(close) = Self::closing_pair(c) {
                let is_quote = matches!(c, '"' | '\'' | '`');
                let next_is_word = next_ch.map(|ch| ch.is_alphanumeric() || ch == '_').unwrap_or(false);
                let prev_is_word = prev_ch.map(|ch| ch.is_alphanumeric() || ch == '_').unwrap_or(false);
                let rust_lifetime_ctx = c == '\''
                    && doc.language == Language::Rust
                    && matches!(prev_ch, Some('&' | '<' | ',' | ' '));

                let should_close = !(next_is_word || rust_lifetime_ctx || (is_quote && prev_is_word));
                if should_close {
                    self.push_undo_if_needed(EditKind::Insert);
                    let doc = &mut self.documents[self.active];
                    let mut chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
                    let at = doc.cursor_col.min(chars.len());
                    chars.insert(at, c);
                    chars.insert(at + 1, close);
                    doc.lines[doc.cursor_line] = chars.into_iter().collect();
                    doc.cursor_col = at + 1;
                    self.mark_active_dirty();
                    self.ensure_cursor_visible();
                    return;
                }
            }
        }

        self.push_undo_if_needed(EditKind::Insert);
        let doc = &mut self.documents[self.active];
        let line = &mut doc.lines[doc.cursor_line];
        let mut chars: Vec<char> = line.chars().collect();
        let at = doc.cursor_col.min(chars.len());
        chars.insert(at, c);
        *line = chars.into_iter().collect();
        doc.cursor_col = at + 1;
        self.mark_active_dirty();
        self.ensure_cursor_visible();
    }

    pub fn insert_newline(&mut self) {
        if self.selection_range().is_some() {
            self.delete_selection();
        } else {
            self.documents[self.active].selection_anchor = None;
        }
        let pairs_on = self.pairs_enabled();
        self.push_undo_if_needed(EditKind::Other);
        let doc = &mut self.documents[self.active];
        let chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
        let at = doc.cursor_col.min(chars.len());
        let before: String = chars[..at].iter().collect();
        let after: String = chars[at..].iter().collect();

        // Extraer la sangría previa (espacios y tabulaciones)
        let mut base_indent = String::new();
        for c in before.chars() {
            if c == ' ' || c == '\t' {
                base_indent.push(c);
            } else {
                break;
            }
        }

        // Si el cursor está entre un par `{|}`, `[|]` o `(|)`, expandir en bloque de 3 líneas
        let trimmed_before = before.trim_end();
        let trimmed_after = after.trim_start();
        let between_brackets = pairs_on
            && ((trimmed_before.ends_with('{') && trimmed_after.starts_with('}'))
                || (trimmed_before.ends_with('[') && trimmed_after.starts_with(']'))
                || (trimmed_before.ends_with('(') && trimmed_after.starts_with(')')));

        if between_brackets {
            let inner_indent = format!("{base_indent}    ");
            let closing_line = format!("{base_indent}{trimmed_after}");
            let new_col = inner_indent.chars().count();
            doc.lines[doc.cursor_line] = before;
            doc.lines.insert(doc.cursor_line + 1, inner_indent);
            doc.lines.insert(doc.cursor_line + 2, closing_line);
            doc.cursor_line += 1;
            doc.cursor_col = new_col;
            self.mark_active_dirty();
            self.ensure_cursor_visible();
            return;
        }

        let mut indent = base_indent;
        // Si la línea antes del cursor termina con un delimitador de apertura, aumentar un nivel (4 espacios)
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
        self.mark_active_dirty();
        self.ensure_cursor_visible();
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
            self.mark_active_dirty();
        } else {
            let line = &mut doc.lines[doc.cursor_line];
            let mut chars: Vec<char> = line.chars().collect();
            let at = doc.cursor_col.min(chars.len());
            for _ in 0..4 {
                chars.insert(at, ' ');
            }
            *line = chars.into_iter().collect();
            doc.cursor_col = at + 4;
            self.mark_active_dirty();
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
            let mut changed = false;
            for line_idx in start_line..=end_line {
                if line_idx < doc.lines.len() {
                    let line = &doc.lines[line_idx];
                    let to_remove = if line.starts_with('\t') {
                        1
                    } else {
                        line.chars().take(4).take_while(|&c| c == ' ').count()
                    };
                    if to_remove > 0 {
                        changed = true;
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
            if changed {
                self.mark_active_dirty();
            }
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
                self.mark_active_dirty();
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.selection_range().is_some() {
            self.delete_selection();
            return;
        }
        self.documents[self.active].selection_anchor = None;
        let pairs_on = self.pairs_enabled();
        self.push_undo_if_needed(EditKind::Delete);
        let doc = &mut self.documents[self.active];
        if doc.cursor_col > 0 {
            let line = &mut doc.lines[doc.cursor_line];
            let mut chars: Vec<char> = line.chars().collect();
            let col = doc.cursor_col.min(chars.len());
            if col > 0 {
                let delete_pair = pairs_on
                    && col < chars.len()
                    && Self::closing_pair(chars[col - 1]) == Some(chars[col]);
                if delete_pair {
                    chars.remove(col);
                }
                chars.remove(col - 1);
                *line = chars.into_iter().collect();
                doc.cursor_col = col - 1;
            }
        } else if doc.cursor_line > 0 {
            let current_line = doc.lines.remove(doc.cursor_line);
            doc.cursor_line -= 1;
            doc.cursor_col = doc.lines[doc.cursor_line].chars().count();
            doc.lines[doc.cursor_line].push_str(&current_line);
        }
        self.mark_active_dirty();
        self.ensure_cursor_visible();
    }

    /// Como `backspace`, pero borra el carácter DESPUÉS del cursor (la
    /// tecla "Supr"/"Delete", no "Backspace").
    pub fn delete_forward(&mut self) {
        if self.selection_range().is_some() {
            self.delete_selection();
            return;
        }
        self.documents[self.active].selection_anchor = None;
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
        self.mark_active_dirty();
    }

    /// Duplica la línea actual (Ctrl+D) o el bloque de líneas seleccionadas.
    pub fn duplicate_line(&mut self) {
        self.push_undo_if_needed(EditKind::Other);
        let sel = self.selection_range();
        let doc = &mut self.documents[self.active];
        let (start_line, end_line) = if let Some((s, e)) = sel {
            let el = if e.1 == 0 && e.0 > s.0 { e.0 - 1 } else { e.0 };
            (s.0, el)
        } else {
            (doc.cursor_line, doc.cursor_line)
        };
        let cloned: Vec<String> = doc.lines[start_line..=end_line].to_vec();
        let insert_at = end_line + 1;
        let count = cloned.len();
        doc.lines.splice(insert_at..insert_at, cloned);
        doc.cursor_line += count;
        doc.selection_anchor = None;
        self.mark_active_dirty();
        self.ensure_cursor_visible();
    }

    /// Alterna comentarios de línea (`// `, `# `, `-- `) en la línea actual o selección (Ctrl+/).
    pub fn toggle_line_comment(&mut self) {
        let Some(prefix) = self.documents[self.active].language.line_comment_prefix() else {
            return;
        };
        let prefix_trimmed = prefix.trim_end();
        self.push_undo_if_needed(EditKind::Other);
        let sel = self.selection_range();
        let doc = &mut self.documents[self.active];
        let (start_line, end_line) = if let Some((s, e)) = sel {
            let el = if e.1 == 0 && e.0 > s.0 { e.0 - 1 } else { e.0 };
            (s.0, el)
        } else {
            (doc.cursor_line, doc.cursor_line)
        };

        let all_commented = (start_line..=end_line)
            .filter(|&i| !doc.lines[i].trim().is_empty())
            .all(|i| doc.lines[i].trim_start().starts_with(prefix_trimmed));

        for i in start_line..=end_line {
            let line = &doc.lines[i];
            if line.trim().is_empty() {
                continue;
            }
            let ws_bytes: usize = line
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| c.len_utf8())
                .sum();
            let rest = &line[ws_bytes..];
            if all_commented {
                if let Some(stripped) = rest.strip_prefix(prefix).or_else(|| rest.strip_prefix(prefix_trimmed)) {
                    let removed_chars = rest.chars().count() - stripped.chars().count();
                    let new_line = format!("{}{}", &line[..ws_bytes], stripped);
                    doc.lines[i] = new_line;
                    if i == doc.cursor_line {
                        doc.cursor_col = doc.cursor_col.saturating_sub(removed_chars);
                    }
                }
            } else {
                let added_chars = prefix.chars().count();
                let new_line = format!("{}{}{}", &line[..ws_bytes], prefix, rest);
                doc.lines[i] = new_line;
                if i == doc.cursor_line {
                    doc.cursor_col += added_chars;
                }
            }
        }
        self.mark_active_dirty();
    }

    /// Mueve la línea actual hacia arriba (`dir < 0`) o hacia abajo (`dir > 0`) (Alt+↑/↓).
    pub fn move_line(&mut self, dir: i32) {
        let cur = self.documents[self.active].cursor_line;
        let len = self.documents[self.active].lines.len();
        if dir < 0 && cur > 0 {
            self.push_undo_if_needed(EditKind::Other);
            let doc = &mut self.documents[self.active];
            doc.lines.swap(cur - 1, cur);
            doc.cursor_line -= 1;
            doc.selection_anchor = None;
            self.mark_active_dirty();
            self.ensure_cursor_visible();
        } else if dir > 0 && cur + 1 < len {
            self.push_undo_if_needed(EditKind::Other);
            let doc = &mut self.documents[self.active];
            doc.lines.swap(cur, cur + 1);
            doc.cursor_line += 1;
            doc.selection_anchor = None;
            self.mark_active_dirty();
            self.ensure_cursor_visible();
        }
    }

    pub fn move_cursor_page(&mut self, dir: i32, extend: bool) {
        let step = self.visible_lines().saturating_sub(1).max(1);
        let doc = &mut self.documents[self.active];
        doc.last_edit_kind = EditKind::None;
        if extend {
            if doc.selection_anchor.is_none() {
                doc.selection_anchor = Some((doc.cursor_line, doc.cursor_col));
            }
        } else {
            doc.selection_anchor = None;
        }
        let max_line = doc.lines.len().saturating_sub(1);
        if dir < 0 {
            doc.cursor_line = doc.cursor_line.saturating_sub(step);
        } else {
            doc.cursor_line = (doc.cursor_line + step).min(max_line);
        }
        doc.cursor_col = doc.cursor_col.min(doc.lines[doc.cursor_line].chars().count());
        self.ensure_cursor_visible();
        self.dirty = true;
    }

    // ---------------------------------------------------------------
    // Selección, clic del mouse y portapapeles
    // ---------------------------------------------------------------

    /// ¿El punto (x, y) cae dentro del área de texto (no en la barra de
    /// herramientas, pestañas, panel abierto o selector abierto)?
    pub fn is_in_text_area(&self, x: f32, y: f32) -> bool {
        let max_y = self.editor_bottom();
        y >= TOP_OFFSET
            && y < max_y
            && !self.panel_contains(x, y)
            && !self.picker_contains(x, y)
            && !self.menu_contains(x, y)
    }

    /// Convierte una posición en píxeles a (línea, columna) del
    /// documento activo, usando el mismo supuesto de ancho monoespaciado
    /// que el resto del editor.
    fn cursor_from_click(&self, x: f32, y: f32) -> (usize, usize) {
        let doc = &self.documents[self.active];
        let char_width = self.char_width();

        let rel_y = (y - TOP_OFFSET - PADDING).max(0.0);
        let line_in_view = (rel_y / self.eff_line_height()) as usize;
        let line = (doc.scroll_offset + line_in_view).min(doc.lines.len().saturating_sub(1));

        let rel_x = (x - PADDING - GUTTER_WIDTH + doc.scroll_x).max(0.0);
        let line_len = doc.lines.get(line).map(|l| l.chars().count()).unwrap_or(0);

        let mut clicked_col: Option<usize> = None;
        if let Some(line_str) = doc.lines.get(line) {
            for run in doc.buffer.layout_runs() {
                if run.line_i == line_in_view {
                    if run.glyphs.is_empty() || rel_x <= 0.0 {
                        clicked_col = Some(0);
                    } else if let Some(last) = run.glyphs.last() {
                        if rel_x >= last.x + last.w * 0.5 {
                            clicked_col = Some(line_len);
                        } else {
                            for g in run.glyphs {
                                let glyph_mid = g.x + g.w * 0.5;
                                if rel_x < glyph_mid {
                                    let safe_end = g.start.min(line_str.len());
                                    if line_str.is_char_boundary(safe_end) {
                                        let col = line_str[..safe_end].chars().count();
                                        clicked_col = Some(col);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }

        let col = clicked_col.unwrap_or_else(|| {
            ((rel_x / char_width).round() as usize).min(line_len)
        });

        (line, col)
    }

    /// Clic simple: coloca el cursor ahí y arranca una selección nueva.
    pub fn click_place_cursor(&mut self, x: f32, y: f32) {
        let (line, col) = self.cursor_from_click(x, y);
        let doc = &mut self.documents[self.active];
        doc.cursor_line = line;
        doc.cursor_col = col;
        doc.selection_anchor = Some((line, col));
        doc.last_edit_kind = EditKind::None;
        self.dirty = true;
    }

    /// Shift+Clic: extiende la selección desde el ancla (o cursor actual) hasta el punto clickeado.
    pub fn shift_click_extend_selection(&mut self, x: f32, y: f32) {
        let (line, col) = self.cursor_from_click(x, y);
        let doc = &mut self.documents[self.active];
        if doc.selection_anchor.is_none() {
            doc.selection_anchor = Some((doc.cursor_line, doc.cursor_col));
        }
        doc.cursor_line = line;
        doc.cursor_col = col;
        doc.last_edit_kind = EditKind::None;
        self.dirty = true;
    }

    /// Doble clic: selecciona la palabra bajo el cursor.
    pub fn select_word_at(&mut self, x: f32, y: f32) {
        let (line, col) = self.cursor_from_click(x, y);
        let doc = &mut self.documents[self.active];
        let chars: Vec<char> = doc.lines[line].chars().collect();
        if chars.is_empty() {
            doc.cursor_line = line;
            doc.cursor_col = 0;
            doc.selection_anchor = None;
            self.dirty = true;
            return;
        }
        let idx = col.min(chars.len().saturating_sub(1));
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let (mut start, mut end) = (idx, idx + 1);
        if is_word(chars[idx]) {
            while start > 0 && is_word(chars[start - 1]) {
                start -= 1;
            }
            while end < chars.len() && is_word(chars[end]) {
                end += 1;
            }
        } else if chars[idx].is_whitespace() {
            while start > 0 && chars[start - 1].is_whitespace() {
                start -= 1;
            }
            while end < chars.len() && chars[end].is_whitespace() {
                end += 1;
            }
        }
        doc.cursor_line = line;
        doc.selection_anchor = Some((line, start));
        doc.cursor_col = end;
        doc.last_edit_kind = EditKind::None;
        self.dirty = true;
    }

    /// Triple clic: selecciona la línea completa.
    pub fn select_line_at(&mut self, x: f32, y: f32) {
        let (line, _) = self.cursor_from_click(x, y);
        let doc = &mut self.documents[self.active];
        let line_len = doc.lines[line].chars().count();
        doc.cursor_line = line;
        doc.selection_anchor = Some((line, 0));
        doc.cursor_col = line_len;
        doc.last_edit_kind = EditKind::None;
        self.dirty = true;
    }

    pub fn clear_selection(&mut self) {
        let doc = &mut self.documents[self.active];
        if doc.selection_anchor.is_some() {
            doc.selection_anchor = None;
            self.dirty = true;
        }
    }

    /// Al soltar el botón izquierdo del ratón, limpia el ancla vacía si el
    /// usuario solo hizo clic (sin arrastrar), evitando selecciones fantasma.
    pub fn finalize_mouse_selection(&mut self) {
        let doc = &mut self.documents[self.active];
        if doc.selection_anchor == Some((doc.cursor_line, doc.cursor_col)) {
            doc.selection_anchor = None;
        }
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
            let clamped_x = mouse_x.clamp(8.0, (self.viewport_width - 410.0).max(8.0));
            let clamped_y = (mouse_y - 34.0).clamp(TOP_OFFSET + 4.0, (self.editor_bottom() - 56.0).max(TOP_OFFSET + 4.0));
            self.tooltip_pos = (clamped_x, clamped_y);
            
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
        let char_width = self.char_width();
        let line_height = self.eff_line_height();

        let mut rects = Vec::new();
        let max_y = self.editor_bottom();
        for line in start.0..=end.0 {
            if line < doc.scroll_offset {
                continue;
            }
            let visible_line = line - doc.scroll_offset;
            let y = TOP_OFFSET + PADDING + visible_line as f32 * line_height;
            if y + line_height > max_y {
                break;
            }
            let line_len = doc.lines.get(line).map(|l| l.chars().count()).unwrap_or(0);
            let col_start = if line == start.0 { start.1 } else { 0 };
            let col_end = if line == end.0 { end.1 } else { line_len };
            if line == end.0 && start.0 < end.0 && col_end == 0 {
                continue;
            }
            let x_start = self.x_for_line_col(line, col_start);
            let x_end = self.x_for_line_col(line, col_end);
            let mut raw_w = (x_end - x_start).max(0.0);
            if col_start == col_end || raw_w == 0.0 {
                raw_w = char_width * 0.4;
            }
            let min_x = PADDING + GUTTER_WIDTH;
            let max_x = (self.viewport_width - SCROLLBAR_WIDTH).max(min_x);
            let clipped_start = x_start.clamp(min_x, max_x);
            let clipped_end = (x_start + raw_w).clamp(min_x, max_x);
            let w = clipped_end - clipped_start;
            if w > 0.0 {
                rects.push((clipped_start, y, w, line_height));
            }
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
        self.mark_active_dirty();
        self.ensure_cursor_visible();
    }

    /// Inserta texto (potencialmente multilínea) en la posición del cursor en O(N),
    /// preservando fielmente su sangría original (sin auto-indentar las líneas pegadas)
    /// y contándolo como UN SOLO paso de deshacer.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.selection_range().is_some() {
            self.delete_selection();
        } else {
            self.documents[self.active].selection_anchor = None;
        }
        self.push_undo_if_needed(EditKind::Other);
        self.paste_in_progress = true;

        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let pasted_lines: Vec<&str> = normalized.split('\n').collect();

        let doc = &mut self.documents[self.active];
        let chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
        let at = doc.cursor_col.min(chars.len());
        let before: String = chars[..at].iter().collect();
        let after: String = chars[at..].iter().collect();

        if pasted_lines.len() == 1 {
            doc.lines[doc.cursor_line] = format!("{before}{}{after}", pasted_lines[0]);
            doc.cursor_col = at + pasted_lines[0].chars().count();
        } else {
            let mut replacement = Vec::with_capacity(pasted_lines.len());
            replacement.push(format!("{before}{}", pasted_lines[0]));
            for mid in &pasted_lines[1..pasted_lines.len() - 1] {
                replacement.push((*mid).to_string());
            }
            let last_part = pasted_lines[pasted_lines.len() - 1];
            replacement.push(format!("{last_part}{after}"));

            let line_idx = doc.cursor_line;
            doc.lines.splice(line_idx..=line_idx, replacement);
            doc.cursor_line = line_idx + pasted_lines.len() - 1;
            doc.cursor_col = last_part.chars().count();
        }

        self.paste_in_progress = false;
        self.mark_active_dirty();
        self.ensure_cursor_visible();
    }

    pub fn move_cursor(&mut self, dx: i32, dy: i32, extend: bool) {
        let doc = &mut self.documents[self.active];
        doc.last_edit_kind = EditKind::None;

        if extend {
            if doc.selection_anchor.is_none() {
                doc.selection_anchor = Some((doc.cursor_line, doc.cursor_col));
            }
        } else if let Some(anchor) = doc.selection_anchor.take() {
            let cursor = (doc.cursor_line, doc.cursor_col);
            if anchor != cursor {
                let (start, end) = if anchor <= cursor { (anchor, cursor) } else { (cursor, anchor) };
                if dx < 0 {
                    doc.cursor_line = start.0;
                    doc.cursor_col = start.1;
                    self.ensure_cursor_visible();
                    self.dirty = true;
                    return;
                } else if dx > 0 {
                    doc.cursor_line = end.0;
                    doc.cursor_col = end.1;
                    self.ensure_cursor_visible();
                    self.dirty = true;
                    return;
                } else if dy < 0 {
                    doc.cursor_line = start.0;
                    doc.cursor_col = start.1;
                } else if dy > 0 {
                    doc.cursor_line = end.0;
                    doc.cursor_col = end.1;
                }
            }
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

    pub fn move_cursor_by_word(&mut self, dir: i32, extend: bool) {
        let doc = &mut self.documents[self.active];
        doc.last_edit_kind = EditKind::None;

        if extend {
            if doc.selection_anchor.is_none() {
                doc.selection_anchor = Some((doc.cursor_line, doc.cursor_col));
            }
        } else {
            doc.selection_anchor = None;
        }

        let is_word_char = |c: char| -> bool {
            c.is_alphanumeric() || c == '_'
        };

        if dir < 0 {
            // Mover hacia la izquierda una palabra
            if doc.cursor_col == 0 {
                if doc.cursor_line > 0 {
                    doc.cursor_line -= 1;
                    doc.cursor_col = doc.lines[doc.cursor_line].chars().count();
                }
            } else {
                let chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
                let mut idx = doc.cursor_col.min(chars.len());
                // 1. Saltar espacios hacia atrás
                while idx > 0 && chars[idx - 1].is_whitespace() {
                    idx -= 1;
                }
                // 2. Saltar grupo de palabras o puntuación hacia atrás
                if idx > 0 {
                    let is_word = is_word_char(chars[idx - 1]);
                    if is_word {
                        while idx > 0 && is_word_char(chars[idx - 1]) {
                            idx -= 1;
                        }
                    } else {
                        while idx > 0 && !chars[idx - 1].is_whitespace() && !is_word_char(chars[idx - 1]) {
                            idx -= 1;
                        }
                    }
                }
                doc.cursor_col = idx;
            }
        } else if dir > 0 {
            // Mover hacia la derecha una palabra
            let chars: Vec<char> = doc.lines[doc.cursor_line].chars().collect();
            let mut idx = doc.cursor_col;
            if idx >= chars.len() {
                if doc.cursor_line + 1 < doc.lines.len() {
                    doc.cursor_line += 1;
                    doc.cursor_col = 0;
                }
            } else {
                if chars[idx].is_whitespace() {
                    while idx < chars.len() && chars[idx].is_whitespace() {
                        idx += 1;
                    }
                } else {
                    let is_word = is_word_char(chars[idx]);
                    if is_word {
                        while idx < chars.len() && is_word_char(chars[idx]) {
                            idx += 1;
                        }
                    } else {
                        while idx < chars.len() && !chars[idx].is_whitespace() && !is_word_char(chars[idx]) {
                            idx += 1;
                        }
                    }
                    // Saltar espacios subsiguientes hasta la siguiente palabra (estilo Word)
                    while idx < chars.len() && (chars[idx] == ' ' || chars[idx] == '\t') {
                        idx += 1;
                    }
                }
                doc.cursor_col = idx;
            }
        }

        self.ensure_cursor_visible();
        self.dirty = true;
    }

    pub fn move_cursor_paragraph(&mut self, dir: i32, extend: bool) {
        let doc = &mut self.documents[self.active];
        doc.last_edit_kind = EditKind::None;

        if extend {
            if doc.selection_anchor.is_none() {
                doc.selection_anchor = Some((doc.cursor_line, doc.cursor_col));
            }
        } else {
            doc.selection_anchor = None;
        }

        let total_lines = doc.lines.len();
        if dir < 0 {
            if doc.cursor_line == 0 {
                doc.cursor_col = 0;
            } else {
                let mut line = doc.cursor_line;
                while line > 0 && doc.lines[line].trim().is_empty() {
                    line -= 1;
                }
                while line > 0 && !doc.lines[line - 1].trim().is_empty() {
                    line -= 1;
                }
                doc.cursor_line = line;
                doc.cursor_col = 0;
            }
        } else if dir > 0 {
            if doc.cursor_line + 1 >= total_lines {
                doc.cursor_line = total_lines.saturating_sub(1);
                doc.cursor_col = doc.lines[doc.cursor_line].chars().count();
            } else {
                let mut line = doc.cursor_line;
                while line + 1 < total_lines && doc.lines[line].trim().is_empty() {
                    line += 1;
                }
                while line + 1 < total_lines && !doc.lines[line].trim().is_empty() {
                    line += 1;
                }
                doc.cursor_line = line;
                doc.cursor_col = 0;
            }
        }

        self.ensure_cursor_visible();
        self.dirty = true;
    }

    pub fn move_cursor_home(&mut self, ctrl: bool, extend: bool) {
        let doc = &mut self.documents[self.active];
        doc.last_edit_kind = EditKind::None;

        if extend {
            if doc.selection_anchor.is_none() {
                doc.selection_anchor = Some((doc.cursor_line, doc.cursor_col));
            }
        } else {
            doc.selection_anchor = None;
        }

        if ctrl {
            doc.cursor_line = 0;
            doc.cursor_col = 0;
        } else {
            let line = &doc.lines[doc.cursor_line];
            let first_non_ws = line.chars().take_while(|c| c.is_whitespace()).count();
            if doc.cursor_col == first_non_ws {
                doc.cursor_col = 0;
            } else {
                doc.cursor_col = first_non_ws;
            }
        }

        self.ensure_cursor_visible();
        self.dirty = true;
    }

    pub fn move_cursor_end(&mut self, ctrl: bool, extend: bool) {
        let doc = &mut self.documents[self.active];
        doc.last_edit_kind = EditKind::None;

        if extend {
            if doc.selection_anchor.is_none() {
                doc.selection_anchor = Some((doc.cursor_line, doc.cursor_col));
            }
        } else {
            doc.selection_anchor = None;
        }

        if ctrl {
            let last_line = doc.lines.len().saturating_sub(1);
            doc.cursor_line = last_line;
            doc.cursor_col = doc.lines[last_line].chars().count();
        } else {
            doc.cursor_col = doc.lines[doc.cursor_line].chars().count();
        }

        self.ensure_cursor_visible();
        self.dirty = true;
    }

    pub fn delete_word_left(&mut self) {
        if self.selection_range().is_some() {
            self.delete_selection();
            return;
        }
        let cur = (self.documents[self.active].cursor_line, self.documents[self.active].cursor_col);
        self.documents[self.active].selection_anchor = Some(cur);
        self.move_cursor_by_word(-1, true);
        if self.selection_range().is_some() {
            self.delete_selection();
        } else {
            self.documents[self.active].selection_anchor = None;
        }
    }

    pub fn delete_word_right(&mut self) {
        if self.selection_range().is_some() {
            self.delete_selection();
            return;
        }
        let cur = (self.documents[self.active].cursor_line, self.documents[self.active].cursor_col);
        self.documents[self.active].selection_anchor = Some(cur);
        self.move_cursor_by_word(1, true);
        if self.selection_range().is_some() {
            self.delete_selection();
        } else {
            self.documents[self.active].selection_anchor = None;
        }
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
            let attrs = base_attrs.color(Color::rgb(r, g, b));
            rich_spans.push((piece, attrs));
        }

        let font_system = &mut self.font_system;
        let doc = &mut self.documents[active];
        doc.buffer.set_rich_text(font_system, rich_spans, Shaping::Advanced);
        doc.buffer.shape_until_scroll(font_system);

        self.rebuild_line_numbers();
        self.rebuild_status_buffer();
    }

    /// Vuelve a escribir la columna de números de línea (uno por cada
    /// línea de código visible), sin envolver jamás (`Wrap::None`), alineados
    /// a la derecha (`Align::Right`) y con exactamente el mismo alto de línea
    /// que el código para coincidir 1:1 en cualquier nivel de zoom.
    fn rebuild_line_numbers(&mut self) {
        let ln_metrics = self.line_number_metrics();
        let ln_width = self.gutter_text_width();
        let editor_h = (self.editor_bottom() - TOP_OFFSET).max(40.0);
        self.line_numbers_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        self.line_numbers_buffer
            .set_metrics(&mut self.font_system, ln_metrics);
        self.line_numbers_buffer
            .set_size(&mut self.font_system, ln_width, editor_h);

        let doc = &self.documents[self.active];
        let visible = self.visible_lines();
        let start = doc.scroll_offset.min(doc.lines.len().saturating_sub(1));
        let end = (start + visible).min(doc.lines.len());

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

        let mut lines_data: Vec<(String, Attrs)> = Vec::with_capacity(end.saturating_sub(start));
        for i in start..end {
            let num_str = if i + 1 < end {
                format!("{}\n", i + 1)
            } else {
                format!("{}", i + 1)
            };
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
            lines_data.push((num_str, base_attrs.color(color)));
        }

        let rich_spans: Vec<(&str, Attrs)> = lines_data
            .iter()
            .map(|(s, a)| (s.as_str(), *a))
            .collect();

        self.line_numbers_buffer
            .set_rich_text(&mut self.font_system, rich_spans, Shaping::Advanced);
        for line in self.line_numbers_buffer.lines.iter_mut() {
            line.set_align(Some(glyphon::cosmic_text::Align::Right));
        }
        self.line_numbers_buffer.shape_until_scroll(&mut self.font_system);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.viewport_width = width as f32;
        self.viewport_height = height as f32;
        self.update_viewport_sizes();
        self.toolbar_buffer
            .set_size(&mut self.font_system, width as f32, TOOLBAR_HEIGHT);
        self.rebuild_tab_buffer();
        self.rebuild_terminal_buffer();
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
        if self.dirty
            || self.settings.is_rgb_active(crate::settings::RgbElement::Syntax)
            || self.settings.is_rgb_active(crate::settings::RgbElement::LineNumbers)
        {
            self.rebuild();
            self.dirty = false;
        }
        if self.terminal.is_open && self.settings.is_rgb_active(crate::settings::RgbElement::Terminal) {
            self.rebuild_terminal_buffer();
        }

        let [dr, dg, db] = self.settings.theme.default_text_color();
        let editor_bottom_i32 = self.editor_bottom() as i32;
        let code_right_i32 = (self.viewport_width - SCROLLBAR_WIDTH - 2.0)
            .max(PADDING + GUTTER_WIDTH) as i32;
        let scroll_x = self.documents[self.active].scroll_x;

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
                    bottom: editor_bottom_i32,
                },
                default_color: Color::rgb(110, 114, 128),
            },
            TextArea {
                buffer: &self.documents[self.active].buffer,
                left: PADDING + GUTTER_WIDTH - scroll_x,
                top: TOP_OFFSET + PADDING,
                scale: 1.0,
                bounds: TextBounds {
                    left: (PADDING + GUTTER_WIDTH - 2.0) as i32,
                    top: TOP_OFFSET as i32,
                    right: code_right_i32,
                    bottom: editor_bottom_i32,
                },
                default_color: Color::rgb(dr, dg, db),
            },
        ];

        // Autocompletado en overlay_areas para que se dibuje ENCIMA de su tarjeta oscura.
        if self.completion_visible {
            let (x, y) = self.completion_anchor_px();
            overlay_areas.push(TextArea {
                buffer: &self.completion_buffer,
                left: x,
                top: y,
                scale: 1.0,
                bounds: TextBounds {
                    left: x as i32,
                    top: y as i32,
                    right: (x + 320.0).min(self.viewport_width) as i32,
                    bottom: (y + 300.0).min(self.viewport_height) as i32,
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
                right: (self.viewport_width - 48.0).max(0.0) as i32,
                bottom: TOOLBAR_HEIGHT as i32,
            },
            default_color: Color::rgb(220, 220, 225),
        });

        base_areas.push(TextArea {
            buffer: &self.tab_buffer,
            left: 8.0 - self.tab_scroll_x,
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

        let (bx, _by, _bw, _bh, _, _, _, _) = self.lang_badge_geometry();
        let (tx, ty) = self.lang_badge_text_pos();
        let [tr, tg, tb] = self.settings.color_for(Some("topbar_text"));

        // Indicador de estado compacto (Ln, Col · Zoom) entre los botones y el badge de lenguaje.
        let status_min_x = 710.0;
        let status_max_x = bx - 14.0;
        if status_max_x - status_min_x >= 160.0 {
            let status_x = (status_max_x - 230.0).max(status_min_x);
            base_areas.push(TextArea {
                buffer: &self.status_buffer,
                left: status_x,
                top: 0.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: status_min_x as i32,
                    top: 0,
                    right: status_max_x as i32,
                    bottom: TOOLBAR_HEIGHT as i32,
                },
                default_color: Color::rgb(tr, tg, tb),
            });
        }

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

        if self.terminal.is_open {
            let (_tx, ty, _tw, _th) = self.terminal_rect();
            let is_rgb = self.settings.is_rgb_active(crate::settings::RgbElement::Terminal);
            let tc = if is_rgb {
                let [r, g, b] = crate::color::chroma_rgb_f32(self.anim_time, 0.0);
                glyphon::Color::rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
            } else {
                let tc_u8 = self.settings.color_for(Some("terminal_text"));
                glyphon::Color::rgb(tc_u8[0], tc_u8[1], tc_u8[2])
            };

            // 1. Pestañas del header
            base_areas.push(TextArea {
                buffer: &self.terminal_header_buffer,
                left: 6.0,
                top: ty + 4.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: ty as i32,
                    right: (width as f32 - 415.0).max(120.0) as i32,
                    bottom: (ty + 28.0) as i32,
                },
                default_color: tc,
            });

            // 2. Botones del header a la derecha
            let run_rect = self.terminal_btn_run_rect();
            let stop_rect = self.terminal_btn_stop_rect();
            let clear_rect = self.terminal_btn_clear_rect();
            let z_out_rect = self.terminal_btn_zoom_out_rect();
            let z_lbl_rect = self.terminal_btn_zoom_label_rect();
            let z_in_rect = self.terminal_btn_zoom_in_rect();
            let close_rect = self.terminal_btn_close_rect();

            // Run
            base_areas.push(TextArea {
                buffer: &self.terminal_btn_run_buffer,
                left: run_rect.0 + 12.0,
                top: ty + 6.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: run_rect.0 as i32,
                    top: ty as i32,
                    right: (run_rect.0 + run_rect.2) as i32,
                    bottom: (ty + 28.0) as i32,
                },
                default_color: tc,
            });

            // Stop
            base_areas.push(TextArea {
                buffer: &self.terminal_btn_stop_buffer,
                left: stop_rect.0 + 12.0,
                top: ty + 6.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: stop_rect.0 as i32,
                    top: ty as i32,
                    right: (stop_rect.0 + stop_rect.2) as i32,
                    bottom: (ty + 28.0) as i32,
                },
                default_color: tc,
            });

            // Clear
            base_areas.push(TextArea {
                buffer: &self.terminal_btn_clear_buffer,
                left: clear_rect.0 + 11.0,
                top: ty + 6.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: clear_rect.0 as i32,
                    top: ty as i32,
                    right: (clear_rect.0 + clear_rect.2) as i32,
                    bottom: (ty + 28.0) as i32,
                },
                default_color: tc,
            });

            // Zoom Out (-)
            base_areas.push(TextArea {
                buffer: &self.terminal_btn_zoom_out_buffer,
                left: z_out_rect.0 + 9.0,
                top: ty + 6.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: z_out_rect.0 as i32,
                    top: ty as i32,
                    right: (z_out_rect.0 + z_out_rect.2) as i32,
                    bottom: (ty + 28.0) as i32,
                },
                default_color: tc,
            });

            // Zoom Label (100%)
            let zoom_pct = (self.terminal.font_scale * 100.0).round() as u32;
            let zoom_chars = if zoom_pct >= 100 { 4 } else { 3 };
            let pad_lbl = ((z_lbl_rect.2 - zoom_chars as f32 * 7.2) / 2.0).max(2.0);
            base_areas.push(TextArea {
                buffer: &self.terminal_btn_zoom_lbl_buffer,
                left: z_lbl_rect.0 + pad_lbl,
                top: ty + 6.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: z_lbl_rect.0 as i32,
                    top: ty as i32,
                    right: (z_lbl_rect.0 + z_lbl_rect.2) as i32,
                    bottom: (ty + 28.0) as i32,
                },
                default_color: tc,
            });

            // Zoom In (+)
            base_areas.push(TextArea {
                buffer: &self.terminal_btn_zoom_in_buffer,
                left: z_in_rect.0 + 9.0,
                top: ty + 6.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: z_in_rect.0 as i32,
                    top: ty as i32,
                    right: (z_in_rect.0 + z_in_rect.2) as i32,
                    bottom: (ty + 28.0) as i32,
                },
                default_color: tc,
            });

            // Close (✕)
            base_areas.push(TextArea {
                buffer: &self.terminal_btn_close_buffer,
                left: close_rect.0 + 10.0,
                top: ty + 6.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: close_rect.0 as i32,
                    top: ty as i32,
                    right: (close_rect.0 + close_rect.2) as i32,
                    bottom: (ty + 28.0) as i32,
                },
                default_color: tc,
            });

            // 3. Contenido de salida con recorte estricto
            let (_cx, cy, _cw, ch) = self.terminal_content_rect();
            base_areas.push(TextArea {
                buffer: &self.terminal_buffer,
                left: PADDING,
                top: cy + 4.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: cy as i32,
                    right: (width as f32 - 16.0) as i32,
                    bottom: (cy + ch) as i32,
                },
                default_color: tc,
            });

            // 4. Barra de entrada inferior
            let (_ix, iy, _iw, _ih) = self.terminal_input_rect();
            base_areas.push(TextArea {
                buffer: &self.terminal_input_buffer,
                left: PADDING,
                top: iy + 5.0,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: iy as i32,
                    right: width as i32,
                    bottom: (iy + 28.0) as i32,
                },
                default_color: tc,
            });
        }
        
        self.text_renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            Resolution { width, height },
            base_areas,
            &mut self.swash_cache,
        )?;

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
    fn test_cursor_pixel_alignment() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.documents[editor.active].lines = vec!["0123456789".into()];
        editor.rebuild();
        let char_w = editor.char_width();
        assert!(char_w > 10.0 && char_w < 15.0);

        editor.documents[editor.active].cursor_col = 0;
        let (x0, _, _, _) = editor.cursor_rect();
        assert_eq!(x0, PADDING + GUTTER_WIDTH);

        editor.documents[editor.active].cursor_col = 1;
        let (x1, _, _, _) = editor.cursor_rect();
        assert!((x1 - (PADDING + GUTTER_WIDTH + char_w)).abs() < 0.05);

        editor.documents[editor.active].cursor_col = 10;
        let (x10, _, _, _) = editor.cursor_rect();
        assert!((x10 - (PADDING + GUTTER_WIDTH + 10.0 * char_w)).abs() < 0.05);
    }

    #[test]
    fn test_word_navigation_and_selection() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.documents[editor.active].lines = vec!["hello world = 42;".into()];
        editor.documents[editor.active].cursor_col = 0;

        // Move right by word (Ctrl+Right without shift)
        editor.move_cursor_by_word(1, false);
        assert_eq!(editor.documents[editor.active].cursor_col, 6); // before "world"
        assert_eq!(editor.selection_range(), None);

        // Move right with shift (Ctrl+Shift+Right) -> select "world "
        editor.move_cursor_by_word(1, true);
        assert_eq!(editor.documents[editor.active].cursor_col, 12); // before "="
        assert_eq!(editor.selection_range(), Some(((0, 6), (0, 12))));
        assert_eq!(editor.selected_text(), Some("world ".to_string()));

        // Collapse selection with plain Left arrow
        editor.move_cursor(-1, 0, false);
        assert_eq!(editor.documents[editor.active].cursor_col, 6);
        assert_eq!(editor.selection_range(), None);

        // Move to end and word left
        editor.documents[editor.active].cursor_col = 17; // after ";"
        editor.move_cursor_by_word(-1, false);
        assert_eq!(editor.documents[editor.active].cursor_col, 16); // before ";"
        editor.move_cursor_by_word(-1, false);
        assert_eq!(editor.documents[editor.active].cursor_col, 14); // before "42"
    }

    #[test]
    fn test_paragraph_and_home_end_navigation() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.documents[editor.active].lines = vec![
            "   line one".into(),
            "line two".into(),
            "".into(),
            "line four".into(),
        ];
        // Test smart Home
        editor.documents[editor.active].cursor_line = 0;
        editor.documents[editor.active].cursor_col = 8;
        editor.move_cursor_home(false, false);
        assert_eq!(editor.documents[editor.active].cursor_col, 3); // first non-whitespace
        editor.move_cursor_home(false, false);
        assert_eq!(editor.documents[editor.active].cursor_col, 0); // column 0

        // Test End
        editor.move_cursor_end(false, false);
        assert_eq!(editor.documents[editor.active].cursor_col, 11);

        // Test Ctrl+End
        editor.move_cursor_end(true, false);
        assert_eq!(editor.documents[editor.active].cursor_line, 3);
        assert_eq!(editor.documents[editor.active].cursor_col, 9);

        // Test Ctrl+Home
        editor.move_cursor_home(true, false);
        assert_eq!(editor.documents[editor.active].cursor_line, 0);
        assert_eq!(editor.documents[editor.active].cursor_col, 0);

        // Test paragraph movement
        editor.move_cursor_paragraph(1, false);
        assert_eq!(editor.documents[editor.active].cursor_line, 2); // empty line
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
        assert!(py_row.label.contains('\u{2713}'));

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

    #[test]
    fn test_terminal_zoom_and_layout_hits() {
        let Some(mut editor) = create_test_editor() else { return; };
        assert_eq!(editor.terminal.font_scale, 1.0);
        assert_eq!(editor.terminal.eff_font_size(), 13.0);
        assert_eq!(editor.terminal.eff_line_height(), 18.0);

        // Zoom in
        editor.terminal_zoom(0.2);
        assert!((editor.terminal.font_scale - 1.2).abs() < 1e-4);
        assert!((editor.terminal.eff_font_size() - 15.6).abs() < 1e-4);
        assert!((editor.terminal.eff_line_height() - 21.6).abs() < 1e-4);

        // Zoom out
        editor.terminal_zoom(-0.5);
        assert!((editor.terminal.font_scale - 0.7).abs() < 1e-4);

        // Zoom reset
        editor.terminal_zoom_reset();
        assert_eq!(editor.terminal.font_scale, 1.0);

        // Hit testing en todos los botones del header del dock
        editor.toggle_terminal();
        assert!(editor.terminal.is_open);
        let run_rect = editor.terminal_btn_run_rect();
        assert_eq!(editor.terminal_hit_test(run_rect.0 + 2.0, run_rect.1 + 2.0), Some(TerminalHit::BtnRun));
        let stop_rect = editor.terminal_btn_stop_rect();
        assert_eq!(editor.terminal_hit_test(stop_rect.0 + 2.0, stop_rect.1 + 2.0), Some(TerminalHit::BtnStop));
        let clear_rect = editor.terminal_btn_clear_rect();
        assert_eq!(editor.terminal_hit_test(clear_rect.0 + 2.0, clear_rect.1 + 2.0), Some(TerminalHit::BtnClear));
        let z_in_rect = editor.terminal_btn_zoom_in_rect();
        assert_eq!(editor.terminal_hit_test(z_in_rect.0 + 2.0, z_in_rect.1 + 2.0), Some(TerminalHit::BtnZoomIn));
        let z_out_rect = editor.terminal_btn_zoom_out_rect();
        assert_eq!(editor.terminal_hit_test(z_out_rect.0 + 2.0, z_out_rect.1 + 2.0), Some(TerminalHit::BtnZoomOut));
        let z_lbl_rect = editor.terminal_btn_zoom_label_rect();
        assert_eq!(editor.terminal_hit_test(z_lbl_rect.0 + 2.0, z_lbl_rect.1 + 2.0), Some(TerminalHit::BtnZoomReset));
        let close_rect = editor.terminal_btn_close_rect();
        assert_eq!(editor.terminal_hit_test(close_rect.0 + 2.0, close_rect.1 + 2.0), Some(TerminalHit::BtnClose));
    }

    #[test]
    fn test_execute_c_and_python_runner() {
        let temp_dir = std::env::temp_dir();
        let py_path = temp_dir.join("amoxli_test_runner_selfcontained.py");
        let py_code = "nums = [10, 20, 30, 40, 50]\nprint(f'Elementos procesados: {len(nums)}')\nprint(f'El promedio final es: {sum(nums)/len(nums):.2f}')\n";
        std::fs::write(&py_path, py_code).expect("escribir py temporal");

        let mut term = crate::terminal::TerminalState::new();
        term.start_run(&py_path, Language::Python);
        for _ in 0..80 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            term.poll();
            if !term.is_running {
                break;
            }
        }
        let py_out = term.output_lines.join("\n");
        let _ = std::fs::remove_file(&py_path);
        if !py_out.contains("No se encontró Python") {
            assert!(py_out.contains("Elementos procesados: 5"), "Python Output: {py_out}");
            assert!(py_out.contains("El promedio final es: 30.00"), "Python Output: {py_out}");
            assert!(
                term.output_lines.iter().any(|l| l.trim() == "Elementos procesados: 5"),
                "Debe ser una línea separada sin mezclarse con ====: {py_out}"
            );
        }

        let c_path = temp_dir.join("amoxli_test_runner_selfcontained.c");
        let c_code = "#include <stdio.h>\nint main(void) {\n    printf(\"Elementos procesados: 5\\n\");\n    printf(\"El promedio final es: 30.00\\n\");\n    return 0;\n}\n";
        std::fs::write(&c_path, c_code).expect("escribir c temporal");

        term.start_run(&c_path, Language::C);
        for _ in 0..120 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            term.poll();
            if !term.is_running {
                break;
            }
        }
        let c_out = term.output_lines.join("\n");
        let _ = std::fs::remove_file(&c_path);
        let exe_path = c_path.with_extension(if cfg!(windows) { "exe" } else { "out" });
        let _ = std::fs::remove_file(&exe_path);
        if !c_out.contains("No se encontró compilador") {
            assert!(c_out.contains("Elementos procesados: 5"), "C Output: {c_out}");
            assert!(c_out.contains("El promedio final es: 30.00"), "C Output: {c_out}");
            assert!(
                term.output_lines.iter().any(|l| l.trim() == "Elementos procesados: 5"),
                "Debe ser una línea separada sin mezclarse con ====: {c_out}"
            );
        }
    }

    #[test]
    fn test_open_file_as_tab_replaces_virgin_and_detects_language() {
        let Some(mut editor) = create_test_editor() else { return; };
        assert_eq!(editor.tab_count(), 1);
        assert_eq!(editor.active_title(), "Sin título 1.txt");

        // Abrir un archivo HTML como primer archivo debe reemplazar la pestaña virgen inicial
        let html_path = PathBuf::from("C:/test/index.html");
        editor.open_file_as_tab(html_path.clone(), "<h1>Hola Mundo</h1>");
        assert_eq!(editor.tab_count(), 1);
        assert_eq!(editor.active_title(), "index.html");
        assert_eq!(editor.active_language(), Language::Html);
        assert_eq!(editor.active_file_path(), Some(html_path.clone()));
        assert_eq!(editor.full_text(), "<h1>Hola Mundo</h1>");

        // Abrir un segundo archivo CSS debe agregarse como segunda pestaña (no reemplazar la primera)
        let css_path = PathBuf::from("C:/test/style.css");
        editor.open_file_as_tab(css_path.clone(), "body { color: red; }");
        assert_eq!(editor.tab_count(), 2);
        assert_eq!(editor.active_title(), "style.css");
        assert_eq!(editor.active_language(), Language::Css);
        assert_eq!(editor.active_file_path(), Some(css_path));

        // Volver a abrir index.html no debe duplicar la pestaña: solo cambia el foco a la existente
        editor.open_file_as_tab(html_path, "<h1>Hola Mundo</h1>");
        assert_eq!(editor.tab_count(), 2);
        assert_eq!(editor.active_tab(), 0);
        assert_eq!(editor.active_title(), "index.html");
    }

    #[test]
    fn test_paste_preserves_indentation_and_single_undo() {
        let Some(mut editor) = create_test_editor() else { return; };
        let snippet = "def saludar():\n    if True:\n        print(\"hola\")\n";
        editor.paste_text(snippet);
        assert_eq!(editor.full_text(), snippet);
        assert!(editor.active_is_dirty());
        // Un solo Undo debe revertir todo el bloque pegado
        editor.undo();
        assert_eq!(editor.full_text(), "");
    }

    #[test]
    fn test_completion_replaces_prefix_and_new_edit_actions() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.set_active_language(Language::Rust);
        editor.documents[editor.active].lines = vec!["let x = pri".into()];
        editor.documents[editor.active].cursor_line = 0;
        editor.documents[editor.active].cursor_col = 11;

        editor.show_completions(
            &[crate::lsp::CompletionItem {
                label: "println!".into(),
                insert_text: "println!".into(),
            }],
            0,
        );
        editor.completion_accept();
        assert_eq!(editor.full_text(), "let x = println!");

        // Duplicar línea
        editor.duplicate_line();
        assert_eq!(editor.documents[editor.active].lines.len(), 2);
        assert_eq!(editor.documents[editor.active].lines[1], "let x = println!");

        // Comentar y descomentar línea
        editor.toggle_line_comment();
        assert_eq!(editor.documents[editor.active].lines[1], "// let x = println!");
        editor.toggle_line_comment();
        assert_eq!(editor.documents[editor.active].lines[1], "let x = println!");

        // Mover línea arriba/abajo
        editor.documents[editor.active].lines = vec!["uno".into(), "dos".into(), "tres".into()];
        editor.documents[editor.active].cursor_line = 1;
        editor.move_line(-1);
        assert_eq!(editor.documents[editor.active].lines, vec!["dos", "uno", "tres"]);
        assert_eq!(editor.documents[editor.active].cursor_line, 0);
        editor.move_line(1);
        assert_eq!(editor.documents[editor.active].lines, vec!["uno", "dos", "tres"]);

        // Ir a línea
        editor.goto_line(3);
        assert_eq!(editor.cursor_position(), (2, 0));
    }

    #[test]
    fn test_close_tab_preserves_active_tab_and_utf8_find_next() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.open_file_as_tab(PathBuf::from("C:/a.rs"), "fn acción() { let ñ = 1; // ACCIÓN }");
        editor.open_file_as_tab(PathBuf::from("C:/b.rs"), "fn dos() {}");
        editor.open_file_as_tab(PathBuf::from("C:/c.rs"), "fn tres() {}");
        assert_eq!(editor.active_tab(), 2);
        assert_eq!(editor.active_title(), "c.rs");

        // Cerrar una pestaña anterior a la activa debe mantener "c.rs" activa
        let closed = editor.close_tab(0);
        assert!(closed.is_some());
        assert_eq!(editor.active_title(), "c.rs");
        assert_eq!(editor.active_tab(), 1);

        // Búsqueda UTF-8 con acentos y mayúsculas/minúsculas sin pánico
        editor.documents[editor.active].lines = vec!["hola acción y ACCIÓN ñandú".into()];
        editor.documents[editor.active].cursor_line = 0;
        editor.documents[editor.active].cursor_col = 0;
        assert!(editor.find_next("acción"));
        assert_eq!(editor.selected_text(), Some("acción".to_string()));
        assert!(editor.find_next("acción"));
        assert_eq!(editor.selected_text(), Some("ACCIÓN".to_string()));
    }

    #[test]
    fn test_editor_bottom_clips_when_terminal_open() {
        let Some(mut editor) = create_test_editor() else { return; };
        let full_h = editor.editor_bottom();
        assert_eq!(full_h, editor.viewport_height);

        editor.toggle_terminal();
        let clipped_h = editor.editor_bottom();
        assert!(clipped_h < full_h);
        let (_, sb_y, _, sb_h) = editor.scrollbar_rect();
        assert!((sb_y + sb_h - clipped_h).abs() < 0.01);
    }

    #[test]
    fn test_line_numbers_exact_vertical_alignment_across_zooms() {
        let Some(mut editor) = create_test_editor() else { return; };
        let mut lines: Vec<String> = (1..=120)
            .map(|i| {
                if i % 5 == 0 {
                    String::new()
                } else {
                    format!("let variable_larga_{i} = \"texto suficientemente largo para probar que nunca se envuelva en el viewport\";")
                }
            })
            .collect();
        lines[0] = "primera linea".into();
        editor.documents[editor.active].lines = lines;

        for (scale, scroll) in [(1.0_f32, 0usize), (1.5_f32, 0usize), (2.0_f32, 95usize)] {
            editor.font_scale = scale;
            editor.apply_zoom_metrics();
            editor.documents[editor.active].scroll_offset = scroll;
            editor.rebuild();

            let code_runs: Vec<(usize, f32, f32, bool)> = editor.documents[editor.active]
                .buffer
                .layout_runs()
                .map(|r| (r.line_i, r.line_top, r.line_y, r.glyphs.is_empty()))
                .collect();
            let ln_runs: Vec<(usize, f32, f32, f32)> = editor
                .line_numbers_buffer
                .layout_runs()
                .map(|r| (r.line_i, r.line_top, r.line_y, r.line_w))
                .collect();

            assert!(!ln_runs.is_empty());
            assert_eq!(
                ln_runs.len(),
                code_runs.len(),
                "Cantidad de filas de números ({}) debe coincidir 1:1 con código ({}) en zoom {scale}",
                ln_runs.len(),
                code_runs.len()
            );

            for (idx, ((c_i, c_top, c_y, c_empty), (l_i, l_top, l_y, l_w))) in
                code_runs.iter().zip(ln_runs.iter()).enumerate()
            {
                assert_eq!(*c_i, idx, "Código no debe envolverse en zoom {scale}");
                assert_eq!(*l_i, idx, "Número de línea no debe envolverse en zoom {scale}");
                assert!(
                    (c_top - l_top).abs() < 0.01,
                    "Desalineación vertical en línea {idx} (zoom {scale}): code_top={c_top}, ln_top={l_top}"
                );
                if (scale - 1.0).abs() < 0.01 && !*c_empty {
                    assert!(
                        (c_y - l_y).abs() < 0.01,
                        "Desalineación de baseline en línea {idx}: code_y={c_y}, ln_y={l_y}"
                    );
                }
                assert!(
                    *l_w <= editor.gutter_text_width() + 0.5,
                    "Número demasiado ancho ({l_w}) para el gutter en zoom {scale}"
                );
            }
        }
    }

    #[test]
    fn test_ctrl_word_navigation_delete_and_shift_selection_after_click() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.documents[editor.active].lines = vec![
            "alpha beta gamma delta".into(),
            "segunda linea".into(),
        ];
        editor.rebuild();

        // 1. Simular clic al final de la línea 0 -> incluso SIN soltar/limpiar el ancla,
        //    Ctrl+Backspace debe borrar la palabra completa en la primera pulsación.
        let end_x = editor.x_for_line_col(0, 22);
        let line0_y = TOP_OFFSET + PADDING + 4.0;
        editor.click_place_cursor(end_x, line0_y);
        assert_eq!(editor.cursor_position(), (0, 22));

        editor.delete_word_left(); // Ctrl+Backspace -> borra "delta"
        assert_eq!(editor.documents[editor.active].lines[0], "alpha beta gamma ");

        // Clic de nuevo + Backspace normal debe borrar el espacio en la primera pulsación
        let x_after = editor.x_for_line_col(0, 17);
        editor.click_place_cursor(x_after, line0_y);
        editor.finalize_mouse_selection();
        editor.backspace();
        assert_eq!(editor.documents[editor.active].lines[0], "alpha beta gamma");

        // 2. Ir al inicio y probar Ctrl+Delete (borrar palabra a la derecha) tras clic
        editor.click_place_cursor(PADDING + GUTTER_WIDTH, line0_y);
        assert_eq!(editor.cursor_position(), (0, 0));
        editor.delete_word_right(); // Ctrl+Delete -> borra "alpha "
        assert_eq!(editor.documents[editor.active].lines[0], "beta gamma");

        // 3. Shift solo -> selección carácter por carácter
        editor.move_cursor(1, 0, true);
        editor.move_cursor(1, 0, true);
        assert_eq!(editor.selected_text(), Some("be".to_string()));

        // 4. Ctrl+Shift+Right -> extiende selección palabra completa
        editor.move_cursor_by_word(1, true);
        assert_eq!(editor.selected_text(), Some("beta ".to_string()));
        editor.move_cursor_by_word(1, true);
        assert_eq!(editor.selected_text(), Some("beta gamma".to_string()));

        // 5. Shift+Left retrocede carácter por carácter desde esa selección
        editor.move_cursor(-1, 0, true);
        assert_eq!(editor.selected_text(), Some("beta gamm".to_string()));

        // 6. Ctrl+Shift+Left reduce selección palabra por palabra
        editor.move_cursor_by_word(-1, true);
        assert_eq!(editor.selected_text(), Some("beta ".to_string()));

        // 7. Ctrl+Backspace con selección activa borra la selección exacta
        editor.delete_word_left();
        assert_eq!(editor.documents[editor.active].lines[0], "gamma");

        // 8. Ctrl+Backspace también en prompt y barra de entrada de terminal
        editor.open_prompt(PromptPurpose::Find, "Buscar:");
        editor.prompt_push_str("hola mundo");
        editor.prompt_delete_word_left();
        assert_eq!(editor.get_prompt_text(), "hola ");

        editor.terminal.input_line = "cargo build --release".to_string();
        editor.terminal.delete_input_word_left();
        assert_eq!(editor.terminal.input_line, "cargo build --");
    }

    #[test]
    fn test_v1_horizontal_scroll_and_crlf_preservation() {
        let Some(mut editor) = create_test_editor() else { return; };
        let long_line = "a".repeat(220);
        let content = format!("{long_line}\r\nsegunda linea");
        editor.open_file_as_tab(PathBuf::from("C:/test/largo.rs"), &content);
        assert_eq!(editor.full_text_for_save(), content);

        // Mover cursor al final de la línea larga debe desplazar scroll_x hacia la derecha
        editor.move_cursor_end(false, false);
        assert!(editor.active_scroll_x() > 0.0, "scroll_x debe avanzar en líneas largas");
        let (cx_pre, _, _, _) = editor.cursor_rect();
        assert!(
            cx_pre >= PADDING + GUTTER_WIDTH && cx_pre <= editor.viewport_width - SCROLLBAR_WIDTH,
            "Cursor horizontal (pre-rebuild) fuera de rango visible: cx={cx_pre}"
        );
        editor.rebuild();
        let (cx, _, _, _) = editor.cursor_rect();
        assert!(
            cx >= PADDING + GUTTER_WIDTH && cx <= editor.viewport_width - SCROLLBAR_WIDTH,
            "Cursor horizontal (post-rebuild) fuera de rango visible: cx={cx}"
        );

        // Volver al inicio de la línea debe regresar scroll_x a 0
        editor.move_cursor_home(false, false);
        assert_eq!(editor.active_scroll_x(), 0.0);

        // Scroll horizontal manual
        editor.scroll_horizontal(120.0);
        assert!(editor.active_scroll_x() > 0.0);
    }

    #[test]
    fn test_v1_find_and_replace_all_and_single() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.documents[editor.active].lines = vec![
            "let café = 1; // CAFÉ".into(),
            "let otro = café + 2;".into(),
        ];
        editor.open_prompt(PromptPurpose::Replace, "Buscar y reemplazar:");
        editor.prompt_push_str("café");
        editor.prompt_switch_field();
        editor.prompt_push_str("té");

        // Reemplazo paso a paso: 1ra llamada selecciona, 2da reemplaza y salta al siguiente
        assert!(editor.replace_current_and_find_next());
        assert_eq!(editor.selected_text(), Some("café".to_string()));
        assert!(editor.replace_current_and_find_next());
        assert!(editor.documents[editor.active].lines[0].starts_with("let té = 1;"));

        // Reemplazar todos los restantes en un solo paso de Undo
        let count = editor.replace_all();
        assert_eq!(count, 2);
        assert_eq!(editor.documents[editor.active].lines[0], "let té = 1; // té");
        assert_eq!(editor.documents[editor.active].lines[1], "let otro = té + 2;");
        editor.undo();
        assert!(editor.documents[editor.active].lines[0].contains("CAFÉ"));
    }

    #[test]
    fn test_v1_auto_close_pairs_and_matching_brackets() {
        let Some(mut editor) = create_test_editor() else { return; };
        editor.set_active_language(Language::Rust);
        editor.settings.auto_close_pairs = true;

        // 1. Autocierre de paréntesis + sobre-escritura del cierre
        editor.insert_char('(');
        assert_eq!(editor.full_text(), "()");
        assert_eq!(editor.cursor_position(), (0, 1));
        editor.insert_char('x');
        editor.insert_char(')'); // salta el ')' existente en vez de duplicar
        assert_eq!(editor.full_text(), "(x)");
        assert_eq!(editor.cursor_position(), (0, 3));

        // 2. Resaltado de paréntesis coincidentes
        editor.rebuild();
        let rects = editor.matching_bracket_rects();
        assert_eq!(rects.len(), 2, "Debe resaltar ambos paréntesis coincidentes");

        // 3. Backspace dentro de par vacío borra ambos caracteres
        editor.documents[editor.active].lines = vec![String::new()];
        editor.documents[editor.active].cursor_col = 0;
        editor.insert_char('[');
        assert_eq!(editor.full_text(), "[]");
        editor.backspace();
        assert_eq!(editor.full_text(), "");

        // 4. Enter entre llaves `{|}` expande bloque en 3 líneas con sangría
        editor.insert_char('{');
        assert_eq!(editor.full_text(), "{}");
        editor.insert_newline();
        assert_eq!(editor.documents[editor.active].lines, vec!["{", "    ", "}"]);
        assert_eq!(editor.cursor_position(), (1, 4));

        // 5. Envolver selección con comillas
        editor.documents[editor.active].lines = vec!["hola".into()];
        editor.documents[editor.active].cursor_line = 0;
        editor.documents[editor.active].selection_anchor = Some((0, 0));
        editor.documents[editor.active].cursor_col = 4;
        editor.insert_char('"');
        assert_eq!(editor.full_text(), "\"hola\"");

        // 6. Lifetime de Rust (&'a) NO debe auto-cerrar la comilla simple
        editor.documents[editor.active].lines = vec!["&".into()];
        editor.documents[editor.active].cursor_line = 0;
        editor.documents[editor.active].cursor_col = 1;
        editor.documents[editor.active].selection_anchor = None;
        editor.insert_char('\'');
        assert_eq!(editor.full_text(), "&'");
    }

    #[test]
    fn test_v1_adaptive_tabs_and_completion_click() {
        let Some(mut editor) = create_test_editor() else { return; };
        for _ in 0..12 {
            editor.new_tab();
        }
        assert_eq!(editor.tab_count(), 13);
        // La pestaña activa debe quedar visible dentro del viewport mediante scroll/adaptación
        let (tx, ty, tw, _th) = editor.tab_rect(editor.active_tab());
        assert!(tx + tw <= editor.viewport_width + 5.0);
        assert_eq!(editor.tab_any_hit_test(tx + 4.0, ty + 4.0), Some(editor.active_tab()));

        // Clic en sugerencia del popup de autocompletado
        editor.documents[editor.active].lines = vec!["pr".into()];
        editor.documents[editor.active].cursor_line = 0;
        editor.documents[editor.active].cursor_col = 2;
        editor.show_completions(
            &[
                crate::lsp::CompletionItem { label: "print".into(), insert_text: "print".into() },
                crate::lsp::CompletionItem { label: "printf".into(), insert_text: "printf".into() },
            ],
            0,
        );
        let (rx, ry, _rw, _rh) = editor.completion_rect().unwrap();
        let row_h = LINE_HEIGHT * 0.85;
        let hit = editor.completion_hit_test(rx + 20.0, ry + 4.0 + row_h + 4.0);
        assert_eq!(hit, Some(1));
        editor.completion_select_and_accept(1);
        assert_eq!(editor.full_text(), "printf");
    }

    #[test]
    fn test_create_and_open_new_html_file() {
        let temp_dir = std::env::temp_dir();
        let html_file = temp_dir.join("test_amoxli_crear.html");
        if html_file.exists() {
            let _ = std::fs::remove_file(&html_file);
        }

        // Simular creación del archivo por el editor/CLI
        std::fs::write(&html_file, "").expect("Debe crearse el archivo");
        assert!(html_file.exists());

        let Some(mut editor) = create_test_editor() else { return; };
        editor.open_file_as_tab(html_file.clone(), "");
        assert_eq!(editor.active_title(), "test_amoxli_crear.html");
        assert_eq!(editor.active_language(), Language::Html);
        assert_eq!(editor.active_file_path(), Some(html_file.clone()));

        // Limpieza
        let _ = std::fs::remove_file(&html_file);
    }
}

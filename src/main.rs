//! AmoxliCode - Fase 1: Shell (ventana + wgpu)
//!
//! Establece el viewport acelerado por hardware. Este módulo NO realiza
//! todavía trabajo de video, texto ni sintaxis: solo garantiza que el
//! pipeline de render (Instance -> Adapter -> Device -> Queue -> Surface)
//! funciona con un loop de render a 60 FPS (V-Sync / Fifo).

mod audio;
mod color;
mod editor;
mod gradient;
mod lsp;
mod settings;
mod syntax;
mod terminal;
mod ui;
mod video;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use editor::TextState;
use log::{error, info};
use video::VideoFrame;
use winit::{
    event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowBuilder},
};

/// Color de fondo por defecto, usado solo antes de que TextState cargue
/// la configuración guardada (el color real por frame sale del tema
/// activo, ver `text_state.background_color()`).


/// Todo lo que necesita la Capa 0 (fondo) para un video o imagen en
/// particular. Se puede crear, reemplazar o quitar mientras la app corre
/// (Etapa B de personalización).
struct BackgroundLayer {
    rx: Receiver<VideoFrame>,
    texture: wgpu::Texture,
    texture_size: wgpu::Extent3d,
    bind_group: wgpu::BindGroup,
}

/// Crea la textura + bind group para un archivo de fondo nuevo (video o
/// imagen). El pipeline y el bind group layout NO cambian entre fondos,
/// así que se reciben ya creados.
fn create_background_layer(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    path: &std::path::Path,
) -> Result<BackgroundLayer> {
    let (rx, width, height) =
        video::spawn(path).context("no se pudo iniciar el decodificador de fondo")?;

    let texture_size = wgpu::Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("background-texture"),
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("background-bind-group"),
        layout: bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });

    Ok(BackgroundLayer {
        rx,
        texture,
        texture_size,
        bind_group,
    })
}

/// Estado central del renderer. Agrupa todo lo que wgpu necesita para
/// dibujar un frame. Las fases siguientes añadirán aquí las texturas de
/// video (Capa 0) y los buffers de glifos (Capa 1).
struct RenderState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    // Mantenemos la ventana viva mientras exista la Surface (requerido por
    // el lifetime 'static de wgpu::Surface en wgpu 0.19+). También la
    // usamos para cambiar el título con el nombre del archivo abierto.
    window: Arc<Window>,

    // --- Capa 0: video/imagen de fondo (ahora reemplazable en caliente) ---
    background: Option<BackgroundLayer>,
    video_bind_group_layout: wgpu::BindGroupLayout,
    video_sampler: wgpu::Sampler,
    video_pipeline: wgpu::RenderPipeline,

    // --- Capa 1: texto (Fase 3) ---
    text_state: TextState,

    // --- Cliente LSP / autocompletado (Fase 6) ---
    lsp_tx: Sender<lsp::LspRequest>,
    lsp_rx: Receiver<lsp::LspEvent>,

    // --- Cursor parpadeante ---
    start_time: std::time::Instant,

    // --- Estado de teclas modificadoras (Ctrl, Shift, etc.) ---
    modifiers: ModifiersState,

    // --- Panel de personalización (Etapa C) ---
    mouse_pos: (f32, f32),
    mouse_left_down: bool,
    text_dragging: bool,
    scrollbar_dragging: bool,
    ui_pipeline: ui::SolidQuadPipeline,
    overlay_ui_pipeline: ui::SolidQuadPipeline,

    // --- Selector de color (cuadro SV + barra de tono) ---
    gradient_pipeline: gradient::GradientPipeline,
    sv_quad: gradient::GradientQuad,
    hue_quad: gradient::GradientQuad,

    // --- Sonidos de atajos de teclado ---
    audio: Option<audio::AudioEngine>,
}

impl RenderState {
    /// Inicializa Instance, Adapter, Device, Queue y configura la Surface.
    /// Prioriza backends de alto rendimiento (Vulkan / DX12) según
    /// especifica el documento de arquitectura.
    async fn new(window: Arc<Window>, video_path: &str) -> Result<Self> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY, // Vulkan / DX12 / Metal
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .context("no se pudo crear la superficie wgpu")?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no se encontró un adaptador GPU compatible")?;

        info!("Adaptador GPU seleccionado: {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("amoxlicode-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .context("fallo al solicitar Device/Queue")?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let alpha_mode = if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            surface_caps.alpha_modes[0]
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // --- Capa 0: decidir qué fondo usar al arrancar ---
        let launch_settings = settings::Settings::load();
        let initial_bg_path: Option<PathBuf> = match &launch_settings.background {
            settings::Background::Custom(p) if p.exists() => Some(p.clone()),
            _ => {
                let fallback = PathBuf::from(video_path);
                if fallback.exists() {
                    Some(fallback)
                } else {
                    None
                }
            }
        };

        let video_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let video_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("video-bind-group-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let background = match &initial_bg_path {
            Some(p) => {
                match create_background_layer(&device, &video_bind_group_layout, &video_sampler, p)
                {
                    Ok(layer) => Some(layer),
                    Err(e) => {
                        error!("no se pudo cargar el fondo inicial: {e:?}");
                        None
                    }
                }
            }
            None => None,
        };

        let video_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("video_shader.wgsl").into()),
        });

        let video_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("video-pipeline-layout"),
                bind_group_layouts: &[&video_bind_group_layout],
                push_constant_ranges: &[],
            });

        let video_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video-pipeline"),
            layout: Some(&video_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &video_shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &video_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // Blending estándar src-alpha para que el 15% de
                    // opacidad definido en el shader se mezcle sobre el
                    // color de fondo ya dibujado (Layer 0 sobre el clear).
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let text_state = TextState::new(&device, &queue, surface_format, size.width, size.height);
        let ui_pipeline = ui::SolidQuadPipeline::new(&device, surface_format);
        let overlay_ui_pipeline = ui::SolidQuadPipeline::new(&device, surface_format);
        let gradient_pipeline = gradient::GradientPipeline::new(&device, surface_format);
        let sv_quad = gradient_pipeline.create_quad(&device);
        let hue_quad = gradient_pipeline.create_quad(&device);
        let (lsp_tx, lsp_rx) = lsp::spawn();

        // Motor de audio: si no hay dispositivo de sonido disponible,
        // seguimos sin problema (solo no habrá sonidos).
        let mut audio = audio::AudioEngine::new();
        if let Some(engine) = &mut audio {
            for (key, path) in &text_state.settings.sounds {
                if let Err(e) = engine.load(key, path) {
                    error!("no se pudo cargar el sonido de '{key}': {e:?}");
                }
            }
        }

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            window,
            background,
            video_bind_group_layout,
            video_sampler,
            video_pipeline,
            text_state,
            lsp_tx,
            lsp_rx,
            start_time: std::time::Instant::now(),
            modifiers: ModifiersState::empty(),
            mouse_pos: (0.0, 0.0),
            mouse_left_down: false,
            text_dragging: false,
            scrollbar_dragging: false,
            ui_pipeline,
            overlay_ui_pipeline,
            gradient_pipeline,
            sv_quad,
            hue_quad,
            audio,
        })
    }

    /// Reemplaza (o quita) el fondo mientras la app sigue corriendo. El
    /// hilo de decodificación anterior (si había uno) se cierra solo: al
    /// soltar su `Receiver` aquí, el próximo intento de enviar un frame
    /// desde ese hilo falla y el hilo termina por su cuenta.
    fn set_background(&mut self, path: Option<PathBuf>) {
        match path {
            Some(p) => {
                match create_background_layer(
                    &self.device,
                    &self.video_bind_group_layout,
                    &self.video_sampler,
                    &p,
                ) {
                    Ok(layer) => self.background = Some(layer),
                    Err(e) => error!("no se pudo cambiar el fondo: {e:?}"),
                }
            }
            None => {
                self.background = None;
            }
        }
    }

    /// Abre un diálogo nativo para elegir un video o imagen nuevo como
    /// fondo, y lo guarda en la configuración para que persista.
    fn choose_background(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter(
                "Video o imagen",
                &[
                    "mp4", "mov", "avi", "mkv", "webm", "gif", "png", "jpg", "jpeg", "bmp",
                ],
            )
            .pick_file();

        if let Some(path) = picked {
            self.set_background(Some(path.clone()));
            self.text_state.settings.background = settings::Background::Custom(path);
            self.text_state.settings.save();
        }
    }

    /// Quita el fondo (deja solo el color sólido del tema) y lo recuerda.
    fn clear_background(&mut self) {
        self.set_background(None);
        self.text_state.settings.background = settings::Background::None;
        self.text_state.settings.save();
    }

    /// Ejecuta lo que corresponda según la fila del panel en la que se
    /// hizo clic.
    fn apply_panel_action(&mut self, action: editor::PanelAction) {
        match action {
            editor::PanelAction::SelectTheme(theme) => self.text_state.set_theme(theme),
            editor::PanelAction::ChooseBackground => self.choose_background(),
            editor::PanelAction::ClearBackground => self.clear_background(),
            editor::PanelAction::CycleEditorOpacity => {
                self.text_state.settings.cycle_editor_opacity();
                self.text_state.settings.save();
                self.text_state.rebuild_panel_buffer();
            }
            editor::PanelAction::CycleMarkerOpacity => {
                self.text_state.settings.cycle_marker_opacity();
                self.text_state.settings.save();
                self.text_state.rebuild_panel_buffer();
            }
            editor::PanelAction::CycleTopbarOpacity => {
                self.text_state.settings.cycle_topbar_opacity();
                self.text_state.settings.save();
                self.text_state.rebuild_panel_buffer();
            }
            editor::PanelAction::CycleTerminalOpacity => {
                self.text_state.settings.cycle_terminal_opacity();
                self.text_state.settings.save();
                self.text_state.rebuild_panel_buffer();
            }
            editor::PanelAction::CycleMenuOpacity => {
                self.text_state.settings.cycle_menu_opacity();
                self.text_state.settings.save();
                self.text_state.rebuild_panel_buffer();
            }
            editor::PanelAction::CycleScrollbarOpacity => {
                self.text_state.settings.cycle_scrollbar_opacity();
                self.text_state.settings.save();
                self.text_state.rebuild_panel_buffer();
            }
            editor::PanelAction::EditColor(name) => self.text_state.open_picker(name),
            editor::PanelAction::ToggleRgbGamer => self.text_state.toggle_rgb_gamer_mode(),
            editor::PanelAction::ToggleRgbElement(elem) => {
                self.text_state.toggle_rgb_element(elem);
            }
            editor::PanelAction::ResetColors => self.text_state.reset_custom_colors(),
            editor::PanelAction::SavePresetPrompt => {
                self.text_state
                    .open_prompt(editor::PromptPurpose::SavePreset, "Nombre del preset:");
            }
            editor::PanelAction::LoadPreset(idx) => self.text_state.load_preset(idx),
            editor::PanelAction::AssignSound(key) => self.choose_sound(key),
            editor::PanelAction::ClearSounds => {
                self.text_state.settings.sounds.clear();
                self.text_state.settings.save();
                if let Some(engine) = &mut self.audio {
                    engine.clear();
                }
                self.text_state.refresh_panel();
            }
            editor::PanelAction::Close => self.text_state.close_panel(),
        }
    }

    /// Sincroniza el texto actual con clangd y le pide autocompletado en
    /// la posición actual del cursor. No bloquea: la respuesta llega más
    /// tarde por `lsp_rx` y se recoge en `render()`.
    fn request_completion(&mut self) {
        let text = self.text_state.full_text();
        let uri = self.text_state.active_uri();
        let language = self.text_state.active_language();
        let _ = self.lsp_tx.send(lsp::LspRequest::SyncDoc {
            uri: uri.clone(),
            language,
            text,
        });
        let (line, col) = self.text_state.cursor_position();
        let _ = self.lsp_tx.send(lsp::LspRequest::Completion {
            uri,
            language,
            line: line as u32,
            character: col as u32,
        });
    }

    /// Empieza una pestaña nueva en blanco (equivalente a "Nuevo" en
    /// cualquier editor de texto).
    fn new_file(&mut self) {
        self.text_state.new_tab();
        self.update_window_title();
    }

    /// Ejecuta las acciones de los menús superiores (estilo Notepad++).
    fn apply_menu_action(&mut self, action: editor::MenuAction) {
        match action {
            // Archivo
            editor::MenuAction::New => self.new_file(),
            editor::MenuAction::Open => self.open_file(),
            editor::MenuAction::Save => self.save_file(),
            editor::MenuAction::SaveAs => self.save_file_as(),
            editor::MenuAction::SaveAll => self.save_all_dirty(),
            editor::MenuAction::CloseTab => {
                let active = self.text_state.active_tab();
                self.close_tab_with_confirm(active);
            }
            editor::MenuAction::Exit => {
                if self.text_state.any_dirty() {
                    let result = rfd::MessageDialog::new()
                        .set_title("Cambios sin guardar")
                        .set_description(
                            "Tienes archivos con cambios sin guardar. ¿Quieres guardarlos antes de salir?",
                        )
                        .set_buttons(rfd::MessageButtons::YesNoCancel)
                        .show();
                    match result {
                        rfd::MessageDialogResult::Yes => {
                            self.save_all_dirty();
                            std::process::exit(0);
                        }
                        rfd::MessageDialogResult::No => std::process::exit(0),
                        _ => {}
                    }
                } else {
                    std::process::exit(0);
                }
            }

            // Editar
            editor::MenuAction::Undo => {
                self.text_state.undo();
                self.play_sound("undo");
            }
            editor::MenuAction::Redo => {
                self.text_state.redo();
                self.play_sound("redo");
            }
            editor::MenuAction::Cut => {
                if let Some(text) = self.text_state.selected_text() {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
                    self.text_state.delete_selection();
                    self.play_sound("cut");
                }
            }
            editor::MenuAction::Copy => {
                if let Some(text) = self.text_state.selected_text() {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
                    self.play_sound("copy");
                }
            }
            editor::MenuAction::Paste => {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    if let Ok(text) = cb.get_text() {
                        self.text_state.paste_text(&text);
                        self.play_sound("paste");
                    }
                }
            }
            editor::MenuAction::Indent => self.text_state.indent(),
            editor::MenuAction::Unindent => self.text_state.unindent(),
            editor::MenuAction::ToggleBookmark => {
                self.text_state.toggle_current_line_bookmark();
            }
            editor::MenuAction::SelectAll => {
                self.text_state.select_all();
                self.play_sound("select_all");
            }

            // Buscar
            editor::MenuAction::Find => {
                self.text_state.open_prompt(editor::PromptPurpose::Find, "Buscar texto:");
            }
            editor::MenuAction::FindNext => {
                if !self.text_state.last_search_query.is_empty() {
                    let q = self.text_state.last_search_query.clone();
                    self.text_state.find_next(&q);
                } else {
                    self.text_state.open_prompt(editor::PromptPurpose::Find, "Buscar texto:");
                }
            }

            // Ver
            editor::MenuAction::ZoomIn => self.text_state.zoom(0.1),
            editor::MenuAction::ZoomOut => self.text_state.zoom(-0.1),
            editor::MenuAction::ZoomReset => self.text_state.zoom_reset(),
            editor::MenuAction::ToggleTerminal => self.text_state.toggle_terminal(),

            // Lenguaje
            editor::MenuAction::SetLanguage(lang) => {
                self.text_state.set_active_language(lang);
                self.update_window_title();
                let text = self.text_state.full_text();
                let uri = self.text_state.active_uri();
                let _ = self.lsp_tx.send(lsp::LspRequest::SyncDoc {
                    uri,
                    language: lang,
                    text,
                });
            }

            // Configuración
            editor::MenuAction::SelectTheme(theme) => self.text_state.set_theme(theme),
            editor::MenuAction::ChooseBackground => self.choose_background(),
            editor::MenuAction::ClearBackground => self.clear_background(),
            editor::MenuAction::ResetColors => self.text_state.reset_custom_colors(),
            editor::MenuAction::SavePresetPrompt => {
                self.text_state.open_prompt(editor::PromptPurpose::SavePreset, "Nombre del preset:");
            }
            editor::MenuAction::OpenPanel => self.text_state.toggle_panel(),
            editor::MenuAction::ToggleRgbGamer => self.text_state.toggle_rgb_gamer_mode(),

            // Terminal
            editor::MenuAction::OpenTerminal => self.open_terminal_here(),
        }
    }

    #[allow(dead_code)]
    fn apply_toolbar_action(&mut self, action: editor::ToolbarAction) {
        self.apply_menu_action(action);
    }

    /// Abre una ventana nueva de PowerShell, ya parada en la carpeta del
    /// archivo activo (o en la carpeta actual si el archivo nunca se ha
    /// guardado). No nos importa qué compilador tenga instalado la
    /// persona — eso corre por su cuenta, aquí solo le damos la terminal.
    fn open_terminal_here(&self) {
        let dir = self
            .text_state
            .active_file_path()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .filter(|d| d.as_os_str().len() > 0)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let dir_str = dir.to_string_lossy().replace('\'', "''");

        let mut cmd = std::process::Command::new("powershell");
        cmd.arg("-NoExit")
            .arg("-Command")
            .arg(format!("Set-Location -LiteralPath '{dir_str}'"));

        // Sin esto, si AmoxliCode se lanzó desde una terminal ya abierta
        // (ej. con `cargo run`), la nueva PowerShell se "cuela" en esa
        // misma ventana en vez de abrir una propia. CREATE_NEW_CONSOLE
        // fuerza a Windows a darle su propia ventana siempre.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
            cmd.creation_flags(CREATE_NEW_CONSOLE);
        }

        if let Err(e) = cmd.spawn() {
            error!("no se pudo abrir la terminal: {e:?}");
        }
    }

    /// Guarda el contenido de la pestaña activa. Si nunca se guardó
    /// antes, primero pide una ruta con el diálogo nativo (por defecto .txt).
    fn save_file(&mut self) {
        let path = match self.text_state.active_file_path() {
            Some(p) => p,
            None => {
                let default_title = self.text_state.active_title();
                let lang = self.text_state.active_language();
                let ext = lang.default_extension();
                let filter_label = format!("{} (*.{})", lang.display_name(), ext);
                let mut dialog = rfd::FileDialog::new()
                    .set_file_name(&default_title);

                if lang == syntax::Language::PlainText {
                    dialog = dialog
                        .add_filter("Archivo de texto (*.txt)", &["txt"])
                        .add_filter("Todos los archivos (*.*)", &["*"]);
                } else {
                    dialog = dialog
                        .add_filter(&filter_label, &[ext])
                        .add_filter("Archivo de texto (*.txt)", &["txt"])
                        .add_filter("Todos los archivos (*.*)", &["*"]);
                }

                match dialog.save_file() {
                    Some(p) => p,
                    None => return,
                }
            }
        };

        let text = self.text_state.full_text();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                info!("Archivo guardado en {path:?}");
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let lang = syntax::Language::from_extension(ext);
                    self.text_state.set_active_language(lang);
                }
                self.text_state.set_active_file_path(path);
                self.text_state.mark_saved();
                self.update_window_title();
                let uri = self.text_state.active_uri();
                let lang = self.text_state.active_language();
                let _ = self.lsp_tx.send(lsp::LspRequest::SyncDoc {
                    uri,
                    language: lang,
                    text,
                });
            }
            Err(e) => error!("no se pudo guardar el archivo: {e:?}"),
        }
    }

    /// "Guardar como": siempre pregunta la ruta, aunque la pestaña ya
    /// tuviera una asignada.
    fn save_file_as(&mut self) {
        let default_title = self.text_state.active_title();
        let lang = self.text_state.active_language();
        let ext = lang.default_extension();
        let filter_label = format!("{} (*.{})", lang.display_name(), ext);
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(&default_title);

        if lang == syntax::Language::PlainText {
            dialog = dialog
                .add_filter("Archivo de texto (*.txt)", &["txt"])
                .add_filter("Todos los archivos (*.*)", &["*"]);
        } else {
            dialog = dialog
                .add_filter(&filter_label, &[ext])
                .add_filter("Archivo de texto (*.txt)", &["txt"])
                .add_filter("Todos los archivos (*.*)", &["*"]);
        }

        let path = match dialog.save_file() {
            Some(p) => p,
            None => return,
        };

        let text = self.text_state.full_text();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                info!("Archivo guardado en {path:?}");
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let lang = syntax::Language::from_extension(ext);
                    self.text_state.set_active_language(lang);
                }
                self.text_state.set_active_file_path(path);
                self.text_state.mark_saved();
                self.update_window_title();
                let uri = self.text_state.active_uri();
                let lang = self.text_state.active_language();
                let _ = self.lsp_tx.send(lsp::LspRequest::SyncDoc {
                    uri,
                    language: lang,
                    text,
                });
            }
            Err(e) => error!("no se pudo guardar el archivo: {e:?}"),
        }
    }

    /// Abre un archivo elegido con el diálogo nativo en una pestaña
    /// NUEVA (no reemplaza la que ya tenías abierta).
    fn open_file_path(&mut self, path: std::path::PathBuf) {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                self.text_state.open_file_as_tab(path, &text);
                self.update_window_title();
                let text_full = self.text_state.full_text();
                let uri = self.text_state.active_uri();
                let lang = self.text_state.active_language();
                let _ = self.lsp_tx.send(lsp::LspRequest::SyncDoc {
                    uri,
                    text: text_full,
                    language: lang,
                });
            }
            Err(e) => {
                log::error!("Error al cargar archivo: {e:?}");
            }
        }
    }

    fn open_file(&mut self) {
        let path = match rfd::FileDialog::new()
            .add_filter("Todos los archivos compatibles", &[
                "txt", "py", "rs", "c", "h", "cpp", "hpp", "java", "js", "ts", "html", "htm", "css", "sql", "json", "md", "cs", "php", "go", "sh", "bash", "xml"
            ])
            .add_filter("Archivos de texto (*.txt)", &["txt"])
            .add_filter("Código fuente (*.py, *.rs, *.c, *.js...)", &[
                "py", "rs", "c", "h", "cpp", "hpp", "java", "js", "ts", "html", "css", "sql", "json", "md", "cs", "php", "go", "sh", "bash", "xml"
            ])
            .add_filter("Todos los archivos (*.*)", &["*"])
            .pick_file()
        {
            Some(p) => p,
            None => return,
        };

        match std::fs::read(&path) {
            Ok(bytes) => {
                // Conversión "tolerante": si el archivo no es UTF-8
                // estricto (común en archivos viejos con acentos en
                // otra codificación), reemplazamos los bytes inválidos
                // en vez de fallar por completo.
                let text = String::from_utf8_lossy(&bytes).into_owned();
                self.text_state.open_file_as_tab(path, &text);
                self.update_window_title();
                let text_full = self.text_state.full_text();
                let uri = self.text_state.active_uri();
                let lang = self.text_state.active_language();
                let _ = self.lsp_tx.send(lsp::LspRequest::SyncDoc {
                    uri,
                    language: lang,
                    text: text_full,
                });
            }
            Err(e) => error!("no se pudo abrir el archivo: {e:?}"),
        }
    }

    /// Actualiza el título de la ventana según la pestaña activa.
    fn update_window_title(&self) {
        self.window
            .set_title(&format!("AmoxliCode - {}", self.text_state.active_title()));
    }

    /// Reproduce el sonido asignado a una acción (si tiene uno y si el
    /// motor de audio pudo abrir un dispositivo). No hace nada si falta
    /// cualquiera de las dos cosas.
    fn play_sound(&self, action: &str) {
        if let Some(engine) = &self.audio {
            engine.play(action);
        }
    }

    /// Abre el diálogo nativo para elegir un archivo de audio y lo
    /// asigna a una acción (ej. "paste", "keypress"...).
    fn choose_sound(&mut self, key: &'static str) {
        let picked = rfd::FileDialog::new()
            .add_filter("Audio", &["mp3", "wav", "ogg", "flac"])
            .pick_file();

        if let Some(path) = picked {
            if let Some(engine) = &mut self.audio {
                if let Err(e) = engine.load(key, &path) {
                    error!("no se pudo cargar el sonido: {e:?}");
                }
            }
            self.text_state.settings.sounds.insert(key.to_string(), path);
            self.text_state.settings.save();
            self.text_state.refresh_panel();
        }
    }

    /// Guarda todas las pestañas que tengan cambios sin guardar (se usa
    /// al cerrar la app si el usuario elige "Sí" en el aviso).
    fn save_all_dirty(&mut self) {
        for idx in 0..self.text_state.tab_count() {
            if self.text_state.is_dirty(idx) {
                self.text_state.switch_tab(idx);
                self.save_file();
            }
        }
    }

    /// Cierra una pestaña, pero si tiene cambios sin guardar primero
    /// pregunta con un diálogo nativo (Sí/No/Cancelar).
    fn close_tab_with_confirm(&mut self, idx: usize) {
        if !self.text_state.is_dirty(idx) {
            self.text_state.close_tab(idx);
            self.update_window_title();
            return;
        }

        let was_active = self.text_state.active_tab();
        self.text_state.switch_tab(idx);
        let title = self.text_state.active_title();

        let result = rfd::MessageDialog::new()
            .set_title("Cambios sin guardar")
            .set_description(format!(
                "\"{title}\" tiene cambios sin guardar. ¿Quieres guardarlos antes de cerrarla?"
            ))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();

        match result {
            rfd::MessageDialogResult::Yes => {
                self.save_file();
                self.text_state.close_tab(idx);
            }
            rfd::MessageDialogResult::No => {
                self.text_state.close_tab(idx);
            }
            _ => {
                // Cancelar: no cerramos nada; regresamos a la pestaña
                // que estaba activa antes de este intento.
                if was_active < self.text_state.tab_count() {
                    self.text_state.switch_tab(was_active);
                }
            }
        }
        self.update_window_title();
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            self.text_state.resize(new_size.width, new_size.height);
        }
    }

    /// Toma el frame de video más reciente disponible en el canal (si hay
    /// alguno) y lo sube a la textura de la Capa 0. No bloquea: si el hilo
    /// de decodificación aún no produjo un frame nuevo, simplemente sigue
    /// mostrando el último frame ya subido.
    fn update_video_texture(&mut self) {
        let Some(bg) = self.background.as_mut() else {
            return;
        };

        let mut latest = None;
        while let Ok(frame) = bg.rx.try_recv() {
            latest = Some(frame);
        }

        if let Some(frame) = latest {
            self.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &bg.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.data,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * bg.texture_size.width),
                    rows_per_image: Some(bg.texture_size.height),
                },
                bg.texture_size,
            );
        }
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.update_video_texture();

        let t = self.start_time.elapsed().as_secs_f32();
        self.text_state.set_anim_time(t);

        // Parpadeo del cursor: medio segundo prendido, medio apagado.
        let blink_on = (self.start_time.elapsed().as_millis() / 500) % 2 == 0;
        self.text_state.set_cursor_blink(blink_on);
        
        if self.text_state.terminal.poll() {
            self.text_state.rebuild_terminal_buffer();
        }

        while let Ok(event) = self.lsp_rx.try_recv() {
            match event {
                lsp::LspEvent::Completions(items) => {
                    self.text_state.show_completions(&items, 0);
                }
            }
        }

        // Subir los glifos necesarios al atlas ANTES de abrir el render
        // pass (glyphon lo requiere así).
        if let Err(e) = self
            .text_state
            .prepare(&self.device, &self.queue, self.size.width, self.size.height)
        {
            error!("no se pudo preparar el texto: {e:?}");
        }

        let [br, bg, bb] = self.text_state.background_color();
        let editor_alpha = self.text_state.settings.editor_opacity as f64;
        let clear_color = wgpu::Color {
            r: (br as f64) * editor_alpha,
            g: (bg as f64) * editor_alpha,
            b: (bb as f64) * editor_alpha,
            a: editor_alpha,
        };

        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("amoxlicode-encoder"),
            });

        // Juntamos TODOS los rectángulos sólidos del cuadro (barra,
        // pestañas, panel, selección, cursor, cajita de texto) en una
        // sola lista, para subirlos y dibujarlos de un solo golpe. Antes
        // se hacía un set_rect + draw por cada uno reutilizando el mismo
        // buffer, lo que causaba parpadeos.
        let mut ui_rects: Vec<ui::Rect> = Vec::new();
        let (mx, my) = self.mouse_pos;
        let accent = if self.text_state.settings.is_rgb_active(settings::RgbElement::Topbar) {
            crate::color::chroma_rgb_f32(t, 0.0)
        } else {
            self.text_state.settings.topbar_accent_rgb()
        };
        let topbar_bg = self.text_state.settings.topbar_bg_rgb();
        let topbar_alpha = self.text_state.settings.topbar_opacity;
        let tab_bar_bg = self.text_state.settings.tab_bar_bg_rgb();
        let tab_active_bg = self.text_state.settings.tab_active_rgb();

        let lighten = |c: [f32; 3], delta: f32| [
            (c[0] + delta).clamp(0.0, 1.0),
            (c[1] + delta).clamp(0.0, 1.0),
            (c[2] + delta).clamp(0.0, 1.0),
        ];

        // 1. Barra de menú superior (fondo base personalizable)
        let (tx, ty, tw, th) = self.text_state.toolbar_rect();
        ui_rects.push(ui::Rect { x: tx, y: ty, w: tw, h: th, color: topbar_bg, alpha: topbar_alpha });

        // Hover y estado activo de las opciones del menú superior
        for (_, menu_opt, _action_opt, x1, x2) in editor::TOOLBAR_ITEMS {
            let is_open = menu_opt.is_some() && self.text_state.active_menu() == menu_opt;
            let is_hovered = mx >= x1 && mx < x2 && my >= 0.0 && my <= editor::TOOLBAR_HEIGHT;
            if is_open {
                ui_rects.push(ui::Rect {
                    x: x1,
                    y: 3.0,
                    w: x2 - x1,
                    h: editor::TOOLBAR_HEIGHT - 6.0,
                    color: lighten(topbar_bg, 0.12),
                    alpha: topbar_alpha,
                });
                // Indicador inferior de acento
                ui_rects.push(ui::Rect {
                    x: x1 + 4.0,
                    y: editor::TOOLBAR_HEIGHT - 3.5,
                    w: x2 - x1 - 8.0,
                    h: 2.0,
                    color: accent,
                    alpha: topbar_alpha,
                });
            } else if is_hovered {
                ui_rects.push(ui::Rect {
                    x: x1,
                    y: 3.0,
                    w: x2 - x1,
                    h: editor::TOOLBAR_HEIGHT - 6.0,
                    color: lighten(topbar_bg, 0.08),
                    alpha: topbar_alpha,
                });
            }
        }

        // Insignia interactiva del lenguaje activo (chip moderno)
        let (bx, by, bw, bh, lx, ly, lw, lh) = self.text_state.lang_badge_geometry();
        let badge_hover = mx >= bx && mx < bx + bw && my >= 0.0 && my <= editor::TOOLBAR_HEIGHT;
        // Borde exterior / halo de acento
        ui_rects.push(ui::Rect {
            x: bx - 1.0,
            y: by - 1.0,
            w: bw + 2.0,
            h: bh + 2.0,
            color: if badge_hover { accent } else { [0.22, 0.25, 0.33] },
            alpha: if badge_hover { 0.95 } else { 0.6 },
        });
        // Fondo del chip
        ui_rects.push(ui::Rect {
            x: bx,
            y: by,
            w: bw,
            h: bh,
            color: if badge_hover { lighten(topbar_bg, 0.12) } else { lighten(topbar_bg, 0.05) },
            alpha: topbar_alpha,
        });
        // Halo de brillo suave para el LED
        ui_rects.push(ui::Rect {
            x: lx - 2.0,
            y: ly - 2.0,
            w: lw + 4.0,
            h: lh + 4.0,
            color: accent,
            alpha: 0.25 * topbar_alpha,
        });
        // LED indicador brillante
        ui_rects.push(ui::Rect {
            x: lx,
            y: ly,
            w: lw,
            h: lh,
            color: accent,
            alpha: 1.0,
        });

        // Botón de engranaje (⚙)
        let (gx, gy, gw, gh) = self.text_state.gear_rect();
        let gear_hover = self.text_state.gear_hit(mx, my);
        if gear_hover || self.text_state.panel_open() {
            ui_rects.push(ui::Rect {
                x: gx,
                y: gy,
                w: gw,
                h: gh,
                color: if self.text_state.panel_open() { lighten(topbar_bg, 0.15) } else { lighten(topbar_bg, 0.08) },
                alpha: 0.9 * topbar_alpha,
            });
        }

        // Línea divisoria 1: entre barra de menú y barra de pestañas
        ui_rects.push(ui::Rect {
            x: 0.0,
            y: editor::TOOLBAR_HEIGHT - 1.0,
            w: self.size.width as f32,
            h: 1.0,
            color: [0.16, 0.18, 0.23],
            alpha: 0.95,
        });

        if self.text_state.terminal.is_open {
            let (tx, ty, tw, th) = self.text_state.terminal_rect();
            let term_op = self.text_state.settings.terminal_opacity;
            let bg = self.text_state.settings.color_for(Some("terminal_bg"));
            ui_rects.push(ui::Rect { x: tx, y: ty, w: tw, h: th, color: [bg[0] as f32 / 255.0, bg[1] as f32 / 255.0, bg[2] as f32 / 255.0], alpha: term_op });
            ui_rects.push(ui::Rect { x: tx, y: ty, w: tw, h: 2.0, color: accent, alpha: 0.8 });
        }
        // 2. Barra de pestañas (fondo base)
        let (bx, by, bw, bh) = self.text_state.tab_bar_rect();
        ui_rects.push(ui::Rect { x: bx, y: by, w: bw, h: bh, color: tab_bar_bg, alpha: topbar_alpha });

        // Tarjetas individuales de pestaña (Tab Cards)
        for i in 0..self.text_state.tab_count() {
            let (tx, ty, tw, th) = self.text_state.tab_rect(i);
            let is_active = i == self.text_state.active_tab();
            let is_hovered = !is_active && mx >= tx && mx <= tx + tw && my >= ty && my <= ty + th;

            if is_active {
                // Fondo de pestaña activa (personalizable)
                ui_rects.push(ui::Rect {
                    x: tx,
                    y: ty,
                    w: tw,
                    h: th,
                    color: tab_active_bg,
                    alpha: topbar_alpha,
                });
                // Línea superior de acento brillante (personalizable)
                ui_rects.push(ui::Rect {
                    x: tx,
                    y: ty,
                    w: tw,
                    h: 2.5,
                    color: accent,
                    alpha: topbar_alpha,
                });
                // Separador lateral sutil
                ui_rects.push(ui::Rect {
                    x: tx + tw - 1.0,
                    y: ty,
                    w: 1.0,
                    h: th,
                    color: [0.22, 0.25, 0.32],
                    alpha: 0.7,
                });
            } else {
                // Fondo de pestaña inactiva
                let bg_color = if is_hovered {
                    lighten(tab_bar_bg, 0.06)
                } else {
                    tab_bar_bg
                };
                ui_rects.push(ui::Rect {
                    x: tx,
                    y: ty,
                    w: tw,
                    h: th,
                    color: bg_color,
                    alpha: 0.9 * topbar_alpha,
                });
                // Separador vertical derecho
                ui_rects.push(ui::Rect {
                    x: tx + tw - 1.0,
                    y: ty + 4.0,
                    w: 1.0,
                    h: th - 8.0,
                    color: [0.18, 0.20, 0.25],
                    alpha: 0.6,
                });
            }

            // Hover en botón de cerrar '×'
            if let Some(close_idx) = self.text_state.tab_close_hit_test(mx, my) {
                if close_idx == i {
                    let close_x = tx + tw - 22.0;
                    ui_rects.push(ui::Rect {
                        x: close_x,
                        y: ty + 3.0,
                        w: 18.0,
                        h: th - 6.0,
                        color: [0.72, 0.24, 0.27],
                        alpha: 0.85,
                    });
                }
            }
        }

        // Botón '+(Nueva pestaña rápida)
        let (ax, ay, aw, ah) = self.text_state.add_tab_rect();
        let add_hover = self.text_state.add_tab_hit_test(mx, my);
        ui_rects.push(ui::Rect {
            x: ax,
            y: ay,
            w: aw,
            h: ah,
            color: if add_hover { lighten(tab_bar_bg, 0.15) } else { lighten(tab_bar_bg, 0.05) },
            alpha: (if add_hover { 1.0 } else { 0.75 }) * topbar_alpha,
        });

        // Línea divisoria 2: entre pestañas y editor de código
        ui_rects.push(ui::Rect {
            x: 0.0,
            y: editor::TOOLBAR_HEIGHT + editor::TAB_HEIGHT - 1.0,
            w: self.size.width as f32,
            h: 1.0,
            color: [0.15, 0.17, 0.22],
            alpha: 0.95 * topbar_alpha,
        });

        // 3. Fondo con opacidad personalizable para el área de código (sobre video de fondo)
        if self.background.is_some() {
            let [bg_r, bg_g, bg_b] = self.text_state.background_color();
            ui_rects.push(ui::Rect {
                x: 0.0,
                y: editor::TOP_OFFSET,
                w: self.size.width as f32,
                h: (self.size.height as f32 - editor::TOP_OFFSET).max(0.0),
                color: [bg_r, bg_g, bg_b],
                alpha: self.text_state.settings.editor_opacity,
            });
        }

        // 4. Marcadores de línea estilo Dev-C++ (franja horizontal completa + indicador en gutter)
        let is_marker_rgb = self.text_state.settings.is_rgb_active(settings::RgbElement::Markers);
        let marker_color_u8 = self.text_state.settings.color_for(Some("line_marker"));
        let default_marker_color = [
            marker_color_u8[0] as f32 / 255.0,
            marker_color_u8[1] as f32 / 255.0,
            marker_color_u8[2] as f32 / 255.0,
        ];
        let marker_alpha = self.text_state.settings.marker_opacity;
        let line_h = self.text_state.line_height();

        for (_line_idx, line_y) in self.text_state.visible_bookmarks() {
            let marker_color = if is_marker_rgb {
                crate::color::chroma_rgb_f32(t, line_y)
            } else {
                default_marker_color
            };
            // Franja horizontal completa en el área de código (estilo Dev-C++)
            ui_rects.push(ui::Rect {
                x: editor::PADDING + editor::GUTTER_WIDTH,
                y: line_y,
                w: (self.size.width as f32 - (editor::PADDING + editor::GUTTER_WIDTH)).max(0.0),
                h: line_h,
                color: marker_color,
                alpha: marker_alpha,
            });
            // Fondo de la línea en el gutter
            ui_rects.push(ui::Rect {
                x: 0.0,
                y: line_y,
                w: editor::PADDING + editor::GUTTER_WIDTH,
                h: line_h,
                color: marker_color,
                alpha: (marker_alpha * 1.5).min(0.65),
            });
            // Pastilla/punto indicador en el gutter
            ui_rects.push(ui::Rect {
                x: 8.0,
                y: line_y + (line_h - 10.0) / 2.0,
                w: 6.0,
                h: 10.0,
                color: marker_color,
                alpha: 1.0,
            });
        }

        if let Some((line_y, alpha_multiplier)) = self.text_state.visible_search_marker() {
            let marker_color = if is_marker_rgb {
                crate::color::chroma_rgb_f32(t, line_y)
            } else {
                default_marker_color
            };
            ui_rects.push(ui::Rect {
                x: editor::PADDING + editor::GUTTER_WIDTH,
                y: line_y,
                w: (self.size.width as f32 - (editor::PADDING + editor::GUTTER_WIDTH)).max(0.0),
                h: line_h,
                color: marker_color,
                alpha: marker_alpha * alpha_multiplier * 0.8,
            });
            ui_rects.push(ui::Rect {
                x: 0.0,
                y: line_y,
                w: editor::PADDING + editor::GUTTER_WIDTH,
                h: line_h,
                color: marker_color,
                alpha: (marker_alpha * 1.5).min(0.65) * alpha_multiplier,
            });
        }

        // 5. Scrollbar en la parte izquierda (personalizable en color y opacidad + soporte RGB)
        let (sb_x, sb_y, sb_w, sb_h) = self.text_state.scrollbar_rect();
        let (th_x, th_y, th_w, th_h) = self.text_state.scrollbar_thumb_rect();
        let sb_color = if self.text_state.settings.is_rgb_active(settings::RgbElement::Scrollbar) {
            crate::color::chroma_rgb_f32(t, th_y)
        } else {
            self.text_state.settings.scrollbar_color_rgb()
        };
        let sb_alpha = self.text_state.settings.scrollbar_opacity;

        // Pista de fondo del scrollbar
        ui_rects.push(ui::Rect {
            x: sb_x,
            y: sb_y,
            w: sb_w,
            h: sb_h,
            color: [0.08, 0.09, 0.12],
            alpha: (sb_alpha * 0.4).min(0.5),
        });
        // Pulgar del scrollbar
        ui_rects.push(ui::Rect {
            x: th_x,
            y: th_y,
            w: th_w,
            h: th_h,
            color: sb_color,
            alpha: sb_alpha,
        });

        let mut overlay_rects: Vec<ui::Rect> = Vec::new();
        if self.text_state.panel_open() {
            let (px, py, pw, ph) = self.text_state.panel_rect();
            let menu_alpha = self.text_state.settings.menu_opacity;
            // Borde exterior
            overlay_rects.push(ui::Rect { x: px - 1.0, y: py - 1.0, w: pw + 2.0, h: ph + 2.0, color: [0.22, 0.25, 0.32], alpha: (0.9 * menu_alpha).min(0.85) });
            // Fondo del panel
            overlay_rects.push(ui::Rect { x: px, y: py, w: pw, h: ph, color: [0.08, 0.08, 0.10], alpha: menu_alpha });

            // Hover sobre fila interactiva
            if let Some((hx, hy, hw, hh)) = self.text_state.panel_hover_row_rect(mx, my) {
                overlay_rects.push(ui::Rect {
                    x: hx,
                    y: hy,
                    w: hw,
                    h: hh,
                    color: lighten(topbar_bg, 0.10),
                    alpha: (0.9 * menu_alpha).min(0.85),
                });
                overlay_rects.push(ui::Rect {
                    x: hx,
                    y: hy,
                    w: 3.0,
                    h: hh,
                    color: accent,
                    alpha: topbar_alpha,
                });
            }
        }

        if self.text_state.picker_open() {
            let (px, py, pw, ph) = self.text_state.picker_rect();
            let menu_alpha = self.text_state.settings.menu_opacity;
            overlay_rects.push(ui::Rect { x: px, y: py, w: pw, h: ph, color: [0.08, 0.08, 0.10], alpha: menu_alpha });
        }

        for (sx, sy, sw, sh) in self.text_state.selection_rects() {
            ui_rects.push(ui::Rect { x: sx, y: sy, w: sw, h: sh, color: [0.25, 0.45, 0.85], alpha: 0.35 });
        }

        if self.text_state.prompt_open() {
            let (qx, qy, qw, qh) = self.text_state.prompt_rect();
            let menu_alpha = self.text_state.settings.menu_opacity;
            overlay_rects.push(ui::Rect { x: qx, y: qy, w: qw, h: qh, color: [0.08, 0.08, 0.10], alpha: menu_alpha });
        }

        if self.text_state.cursor_visible() {
            let (cx, cy, cw, ch) = self.text_state.cursor_rect();
            ui_rects.push(ui::Rect { x: cx, y: cy, w: cw, h: ch, color: self.text_state.cursor_color(), alpha: 1.0 });
        }

        if self.text_state.menu_open() {
            let (mx_menu, my_menu, mw_menu, mh_menu) = self.text_state.menu_rect();
            let menu_bg = self.text_state.settings.menu_bg_rgb();
            let menu_alpha = self.text_state.settings.menu_opacity;
            let menu_border_color = if self.text_state.settings.is_rgb_active(settings::RgbElement::Menus) {
                crate::color::chroma_rgb_f32(t, 10.0)
            } else {
                [0.26, 0.30, 0.40]
            };

            overlay_rects.push(ui::Rect {
                x: mx_menu - 2.0,
                y: my_menu - 1.0,
                w: mw_menu + 4.0,
                h: mh_menu + 3.0,
                color: [0.02, 0.02, 0.03],
                alpha: (0.7 * menu_alpha).min(0.7),
            });
            overlay_rects.push(ui::Rect {
                x: mx_menu - 1.0,
                y: my_menu,
                w: mw_menu + 2.0,
                h: mh_menu + 1.0,
                color: menu_border_color,
                alpha: menu_alpha,
            });
            overlay_rects.push(ui::Rect {
                x: mx_menu,
                y: my_menu,
                w: mw_menu,
                h: mh_menu,
                color: menu_bg,
                alpha: menu_alpha,
            });

            // Hover en la fila seleccionada del menú
            if mx >= mx_menu && mx <= mx_menu + mw_menu && my >= my_menu + 6.0 && my <= my_menu + mh_menu - 6.0 {
                let row_idx = ((my - my_menu - 6.0) / editor::MENU_ROW_HEIGHT) as usize;
                if row_idx < self.text_state.menu_rows_count() {
                    let row_y = my_menu + 6.0 + row_idx as f32 * editor::MENU_ROW_HEIGHT;
                    overlay_rects.push(ui::Rect {
                        x: mx_menu + 4.0,
                        y: row_y,
                        w: mw_menu - 8.0,
                        h: editor::MENU_ROW_HEIGHT,
                        color: lighten(menu_bg, 0.12),
                        alpha: menu_alpha,
                    });
                    overlay_rects.push(ui::Rect {
                        x: mx_menu + 4.0,
                        y: row_y,
                        w: 3.0,
                        h: editor::MENU_ROW_HEIGHT,
                        color: accent,
                        alpha: 1.0,
                    });
                }
            }
        }

        self.ui_pipeline.upload(
            &self.device,
            &self.queue,
            self.size.width,
            self.size.height,
            &ui_rects,
        );

        
        if self.text_state.picker_open() {
            let hue = self.text_state.picker_hue();
            let sat = self.text_state.picker_sat();
            let val = self.text_state.picker_val();
            let current_rgb = self.text_state.picker_color_rgb();

            // 1. Indicador en el cuadro SV (retícula de selección tipo mira)
            let (sx, sy, sw, sh) = self.text_state.picker_sv_rect();
            let cx = (sx + sat * sw).clamp(sx, sx + sw);
            let cy = (sy + (1.0 - val) * sh).clamp(sy, sy + sh);

            // Cruz exterior oscura (contraste en fondos claros)
            overlay_rects.push(ui::Rect { x: cx - 7.0, y: cy - 1.0, w: 14.0, h: 2.0, color: [0.0, 0.0, 0.0], alpha: 0.85 });
            overlay_rects.push(ui::Rect { x: cx - 1.0, y: cy - 7.0, w: 2.0, h: 14.0, color: [0.0, 0.0, 0.0], alpha: 0.85 });
            // Anillo exterior blanco de alta visibilidad
            overlay_rects.push(ui::Rect { x: cx - 5.0, y: cy - 5.0, w: 10.0, h: 10.0, color: [1.0, 1.0, 1.0], alpha: 0.95 });
            // Borde oscuro del núcleo
            overlay_rects.push(ui::Rect { x: cx - 4.0, y: cy - 4.0, w: 8.0, h: 8.0, color: [0.0, 0.0, 0.0], alpha: 0.9 });
            // Núcleo con el color actualmente seleccionado
            overlay_rects.push(ui::Rect { x: cx - 3.0, y: cy - 3.0, w: 6.0, h: 6.0, color: current_rgb, alpha: 1.0 });

            // 2. Indicador en la barra de Hue (cursor/notch vertical)
            let (hx, hy, hw, hh) = self.text_state.picker_hue_rect();
            let hx_pos = (hx + (hue / 360.0) * hw).clamp(hx, hx + hw);

            // Borde exterior negro del cursor
            overlay_rects.push(ui::Rect { x: hx_pos - 3.0, y: hy - 2.0, w: 6.0, h: hh + 4.0, color: [0.0, 0.0, 0.0], alpha: 0.9 });
            // Píldora blanca indicadora
            overlay_rects.push(ui::Rect { x: hx_pos - 2.0, y: hy - 1.0, w: 4.0, h: hh + 2.0, color: [1.0, 1.0, 1.0], alpha: 1.0 });
            // Línea central con el color de tono exacto
            let [hr, hg, hb] = crate::color::hsv_to_rgb(hue, 1.0, 1.0);
            overlay_rects.push(ui::Rect {
                x: hx_pos - 1.0,
                y: hy,
                w: 2.0,
                h: hh,
                color: [hr as f32 / 255.0, hg as f32 / 255.0, hb as f32 / 255.0],
                alpha: 1.0,
            });

            // 3. Muestra de vista previa de color (swatch) en la tarjeta del selector
            let (px, py, pw, _ph) = self.text_state.picker_rect();
            let swatch_x = px + pw - 46.0;
            let swatch_y = py + 8.0;
            overlay_rects.push(ui::Rect { x: swatch_x - 1.0, y: swatch_y - 1.0, w: 34.0, h: 22.0, color: [0.35, 0.38, 0.45], alpha: 1.0 });
            overlay_rects.push(ui::Rect { x: swatch_x, y: swatch_y, w: 32.0, h: 20.0, color: current_rgb, alpha: 1.0 });
        }

        if self.text_state.anim_time < self.text_state.tooltip_expiration {
            let (tx, ty) = self.text_state.tooltip_pos;
            let tw = 400.0;
            let th = 50.0;
            // Borde y fondo
            overlay_rects.push(ui::Rect { x: tx - 4.0, y: ty - 4.0, w: tw + 8.0, h: th + 8.0, color: [0.1, 0.1, 0.15], alpha: 0.95 });
            overlay_rects.push(ui::Rect { x: tx - 2.0, y: ty - 2.0, w: tw + 4.0, h: th + 4.0, color: [0.05, 0.05, 0.08], alpha: 0.95 });
        }

        self.overlay_ui_pipeline.upload(
            &self.device,
            &self.queue,
            self.size.width,
            self.size.height,
            &overlay_rects,
        );

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            // Capa 0: quad de video/imagen de fondo (si hay uno
            // configurado), mezclado sobre el color de fondo ya limpiado.
            if let Some(bg) = &self.background {
                pass.set_pipeline(&self.video_pipeline);
                pass.set_bind_group(0, &bg.bind_group, &[]);
                pass.draw(0..3, 0..1); // Triángulo de pantalla completa.
            }

            // Todos los rectángulos sólidos (barra, pestañas, panel,
            // selección, cursor, cajita de texto) en un solo draw.
            self.ui_pipeline.draw(&mut pass);



            // Capa 1: texto base, dibujado debajo de los menús
            if let Err(e) = self.text_state.render(&mut pass) {
                error!("no se pudo dibujar el texto: {e:?}");
            }

            // Capa 2: Fondos superpuestos (menús, prompts, indicadores del picker)
            self.overlay_ui_pipeline.draw(&mut pass);

            // Selector de color: cuadro SV + barra de tono (ahora dibujado sobre el fondo del panel que está en overlay_rects)
            if self.text_state.picker_open() {
                let hue = self.text_state.picker_hue();

                let (sx, sy, sw, sh) = self.text_state.picker_sv_rect();
                self.sv_quad.update(
                    &self.queue,
                    self.size.width,
                    self.size.height,
                    sx,
                    sy,
                    sw,
                    sh,
                    hue,
                    false,
                );
                self.sv_quad.draw(&self.gradient_pipeline, &mut pass);

                let (hx, hy, hw, hh) = self.text_state.picker_hue_rect();
                self.hue_quad.update(
                    &self.queue,
                    self.size.width,
                    self.size.height,
                    hx,
                    hy,
                    hw,
                    hh,
                    hue,
                    true,
                );
                self.hue_quad.draw(&self.gradient_pipeline, &mut pass);
            }

            // Capa 3: Texto superpuesto de los menús
            if let Err(e) = self.text_state.render_overlay(&mut pass) {
                error!("no se pudo dibujar el texto superpuesto: {e:?}");
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();


        Ok(())
    }
}

fn main() -> Result<()> {
    env_logger::init();

    let event_loop = EventLoop::new().context("no se pudo crear el event loop")?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let window = Arc::new(
        WindowBuilder::new()
            .with_title("AmoxliCode")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0))
            .with_transparent(true)
            .build(&event_loop)
            .context("no se pudo crear la ventana")?,
    );

    // Ruta del video de fondo: por defecto busca "assets/background.mp4"
    // junto al ejecutable, o se puede pasar como argumento:
    //   cargo run -- ruta\a\tu\video.mp4
    let arg1 = std::env::args().nth(1);
    let mut video_path = "assets/background.mp4".to_string();
    let mut file_to_open = None;

    if let Some(arg) = arg1 {
        let arg_lower = arg.to_lowercase();
        if arg_lower.ends_with(".mp4") || arg_lower.ends_with(".mov") || arg_lower.ends_with(".webm") || arg_lower.ends_with(".avi") {
            video_path = arg;
        } else {
            file_to_open = Some(arg);
        }
    }
    info!("Usando video de fondo: {video_path}");

    let mut state = pollster::block_on(RenderState::new(window.clone(), &video_path))?;
    
    if let Some(file_path) = file_to_open {
        state.open_file_path(std::path::PathBuf::from(file_path));
    }
    
    state.update_window_title();

    event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => match event {
                WindowEvent::DroppedFile(path) => {
                    state.open_file_path(path);
                    window.request_redraw();
                }
                WindowEvent::CloseRequested => {
                    if state.text_state.any_dirty() {
                        let result = rfd::MessageDialog::new()
                            .set_title("Cambios sin guardar")
                            .set_description(
                                "Tienes archivos con cambios sin guardar. ¿Quieres guardarlos antes de salir?",
                            )
                            .set_buttons(rfd::MessageButtons::YesNoCancel)
                            .show();
                        match result {
                            rfd::MessageDialogResult::Yes => {
                                state.save_all_dirty();
                                elwt.exit();
                            }
                            rfd::MessageDialogResult::No => elwt.exit(),
                            _ => {
                                // Cancelar: la app se queda abierta.
                            }
                        }
                    } else {
                        elwt.exit();
                    }
                }
                WindowEvent::Resized(new_size) => state.resize(new_size),
                WindowEvent::ModifiersChanged(mods) => {
                    state.modifiers = mods.state();
                }
                WindowEvent::CursorMoved { position, .. } => {
                    state.mouse_pos = (position.x as f32, position.y as f32);
                    if state.mouse_left_down {
                        let (mx, my) = state.mouse_pos;
                        if state.scrollbar_dragging {
                            state.text_state.scrollbar_scroll_to_y(my);
                            window.request_redraw();
                        } else if state.text_state.prompt_dragging {
                            state.text_state.prompt_x = Some(mx - state.text_state.prompt_drag_offset.0);
                            state.text_state.prompt_y = Some(my - state.text_state.prompt_drag_offset.1);
                            window.request_redraw();
                        } else if let Some((s, v)) = state.text_state.picker_hit_sv(mx, my) {
                            state.text_state.picker_set_sv(s, v);
                            window.request_redraw();
                        } else if let Some(h) = state.text_state.picker_hit_hue(mx, my) {
                            state.text_state.picker_set_hue(h);
                            window.request_redraw();
                        } else if state.text_dragging {
                            state.text_state.drag_extend_selection(mx, my);
                            window.request_redraw();
                        }
                    }
                }
                WindowEvent::MouseInput {
                    state: btn_state,
                    button,
                    ..
                } => {
                    if button == MouseButton::Left {
                        state.mouse_left_down = btn_state == ElementState::Pressed;
                        if btn_state == ElementState::Released {
                            state.text_dragging = false;
                            state.scrollbar_dragging = false;
                            state.text_state.prompt_dragging = false;
                        }
                    }
                    if btn_state == ElementState::Pressed && button == MouseButton::Left {
                        let (mx, my) = state.mouse_pos;
                        if state.text_state.terminal_hit_test(mx, my) {
                            state.text_state.terminal_focused = true;
                        } else {
                            state.text_state.terminal_focused = false;
                        }
                        if state.text_state.prompt_open() {
                            if let Some(hit) = state.text_state.prompt_hit_test(mx, my) {
                                match hit {
                                    editor::PromptHit::Button => {
                                        let q = state.text_state.get_prompt_text().to_string();
                                        state.text_state.find_next(&q);
                                        window.request_redraw();
                                    }
                                    editor::PromptHit::Drag => {
                                        let (px, py, _, _) = state.text_state.prompt_rect();
                                        state.text_state.prompt_dragging = true;
                                        state.text_state.prompt_drag_offset = (mx - px, my - py);
                                    }
                                }
                            } else {
                                state.text_state.close_prompt();
                                window.request_redraw();
                            }
                        } else if state.text_state.menu_open() && state.text_state.menu_contains(mx, my) {
                            if let Some(action) = state.text_state.menu_hit_test(mx, my) {
                                state.apply_menu_action(action);
                            }
                            state.text_state.close_menu();
                        } else if let Some(hit) = state.text_state.toolbar_hit_test(mx, my) {
                            match hit {
                                editor::ToolbarHit::Menu(menu) => {
                                    if state.text_state.active_menu() == Some(menu) {
                                        state.text_state.close_menu();
                                    } else {
                                        state.text_state.open_menu(menu);
                                    }
                                }
                                editor::ToolbarHit::Action(action) => {
                                    state.text_state.close_menu();
                                    state.apply_menu_action(action);
                                }
                            }
                        } else {
                            if state.text_state.menu_open() {
                                state.text_state.close_menu();
                            }
                            if let Some(idx) = state.text_state.tab_close_hit_test(mx, my) {
                                state.close_tab_with_confirm(idx);
                            } else if state.text_state.add_tab_hit_test(mx, my) {
                                state.new_file();
                            } else if let Some(idx) = state.text_state.tab_hit_test(mx, my) {
                                state.text_state.switch_tab(idx);
                                state.update_window_title();
                            } else if state.text_state.gear_hit(mx, my) {
                                state.text_state.toggle_panel();
                            } else if let Some((s, v)) = state.text_state.picker_hit_sv(mx, my) {
                                state.text_state.picker_set_sv(s, v);
                            } else if let Some(h) = state.text_state.picker_hit_hue(mx, my) {
                                state.text_state.picker_set_hue(h);
                            } else if let Some(action) = state.text_state.panel_hit_test(mx, my) {
                                state.apply_panel_action(action);
                            } else if state.text_state.picker_open()
                                && !state.text_state.picker_contains(mx, my)
                            {
                                state.text_state.close_picker();
                            } else if state.text_state.panel_open()
                                && !state.text_state.panel_contains(mx, my)
                            {
                                // Clic fuera del panel: se cierra, como un
                                // menú normal.
                                state.text_state.close_panel();
                            } else if state.text_state.scrollbar_hit_test(mx, my) {
                                state.scrollbar_dragging = true;
                                state.text_state.scrollbar_scroll_to_y(my);
                            } else if let Some(line) = state.text_state.gutter_hit_test(mx, my) {
                                state.text_state.toggle_bookmark(line);
                            } else if state.text_state.is_in_text_area(mx, my) {
                                // Clic dentro del código: coloca el cursor
                                // ahí y arranca una posible selección (si el
                                // usuario arrastra el mouse antes de soltar).
                                state.text_state.click_place_cursor(mx, my);
                                if state.modifiers.control_key() {
                                    state.text_state.goto_definition_under_cursor(mx, my);
                                } else {
                                    state.text_state.click_place_cursor(mx, my);
                                    state.text_dragging = true;
                                }
                            }
                        }
                        window.request_redraw();
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let raw_y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(pos) => (pos.y / 20.0) as f32,
                    };
                    if state.text_state.panel_open()
                        && state.text_state.panel_contains(state.mouse_pos.0, state.mouse_pos.1)
                    {
                        state.text_state.scroll_panel(-(raw_y.signum() as i32) * 2);
                    } else if state.modifiers.control_key() {
                        // Ctrl + rueda = zoom (como en la mayoría de editores).
                        state.text_state.zoom(raw_y.signum() * 0.1);
                    } else {
                        // Rueda normal = scroll (3 líneas por "click" de rueda).
                        state.text_state.scroll(-(raw_y.signum() as i32) * 3);
                    }
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state != ElementState::Pressed {
                        return;
                    }
                    if state.text_state.terminal_focused {
                        match &event.logical_key {
                            Key::Named(NamedKey::Enter) => {
                                state.text_state.terminal.send_input();
                                state.text_state.rebuild_terminal_buffer();
                            }
                            Key::Named(NamedKey::Backspace) => {
                                state.text_state.terminal.input_line.pop();
                                state.text_state.rebuild_terminal_buffer();
                            }
                            Key::Named(NamedKey::Escape) => {
                                state.text_state.terminal_focused = false;
                            }
                            Key::Character(c) if !state.modifiers.control_key() && !state.modifiers.alt_key() => {
                                state.text_state.terminal.input_line.push_str(c.as_str());
                                state.text_state.rebuild_terminal_buffer();
                            }
                            _ => {}
                        }
                        if let Key::Character(c) = &event.logical_key {
                            if state.modifiers.control_key() && c.as_str() == "" {
                                state.text_state.toggle_terminal();
                            }
                        }
                        window.request_redraw();
                        return;
                    }

                    // Si la cajita de texto genérica está abierta, se
                    // lleva TODA la atención del teclado (es modal):
                    // ningún atajo de Ctrl ni edición normal debe colarse
                    // mientras el usuario está escribiendo un nombre.
                    if state.text_state.prompt_open() {
                        match &event.logical_key {
                            Key::Named(NamedKey::Enter) => {
                                if let Some((purpose, text)) = state.text_state.prompt_confirm() {
                                    let trimmed = text.trim();
                                    if !trimmed.is_empty() {
                                        match purpose {
                                            editor::PromptPurpose::SavePreset => {
                                                state
                                                    .text_state
                                                    .settings
                                                    .save_preset(trimmed.to_string());
                                                state.text_state.settings.save();
                                                state.text_state.refresh_panel();
                                            }
                                            editor::PromptPurpose::Find => {
                                                state.text_state.find_next(trimmed);
                                            }
                                        }
                                    }
                                }
                            }
                            Key::Named(NamedKey::Escape) => {
                                state.text_state.close_prompt();
                            }
                            Key::Named(NamedKey::Backspace) => {
                                state.text_state.prompt_backspace();
                            }
                            _ => {
                                if let Some(text) = &event.text {
                                    for c in text.chars() {
                                        if !c.is_control() {
                                            state.text_state.prompt_push_char(c);
                                        }
                                    }
                                }
                            }
                        }
                        window.request_redraw();
                        return;
                    }

                    if event.logical_key == Key::Named(NamedKey::Escape) {
                        if state.text_state.menu_open() {
                            state.text_state.close_menu();
                            window.request_redraw();
                            return;
                        }
                    }

                    if event.logical_key == Key::Named(NamedKey::F8) {
                        state.text_state.toggle_rgb_gamer_mode();
                        window.request_redraw();
                        return;
                    }

                    // Ctrl+S / Ctrl+O: guardar o abrir un archivo real.
                    // Se revisa antes que cualquier otra cosa, para que
                    // funcione incluso con el popup de autocompletado
                    // abierto.
                    if state.modifiers.control_key() {
                        if let Key::Character(c) = &event.logical_key {
                            match c.as_str().to_lowercase().as_str() {
                                "" => {
                                    state.text_state.toggle_terminal();
                                    window.request_redraw();
                                    return;
                                }
                                "s" => {
                                    state.save_file();
                                    window.request_redraw();
                                    return;
                                }
                                "o" => {
                                    state.open_file();
                                    window.request_redraw();
                                    return;
                                }
                                "=" | "+" => {
                                    state.text_state.zoom(0.1);
                                    window.request_redraw();
                                    return;
                                }
                                "-" => {
                                    state.text_state.zoom(-0.1);
                                    window.request_redraw();
                                    return;
                                }
                                "0" => {
                                    state.text_state.zoom_reset();
                                    window.request_redraw();
                                    return;
                                }
                                "t" => {
                                    // Temporal (Etapa A de personalización):
                                    // rota entre temas. Se reemplazará por
                                    // el panel visual en la Etapa C.
                                    state.text_state.cycle_theme();
                                    window.request_redraw();
                                    return;
                                }
                                "b" => {
                                    // Temporal (Etapa B de personalización):
                                    // Ctrl+Shift+B quita el fondo,
                                    // Ctrl+B abre el diálogo para elegir
                                    // uno nuevo. Se reemplazará por el
                                    // panel visual en la Etapa C.
                                    if state.modifiers.shift_key() {
                                        state.clear_background();
                                    } else {
                                        state.choose_background();
                                    }
                                    window.request_redraw();
                                    return;
                                }
                                "," => {
                                    // Ctrl+, abre/cierra el panel de
                                    // personalización (lo mismo que
                                    // hacer clic en el botón "⚙").
                                    state.text_state.toggle_panel();
                                    window.request_redraw();
                                    return;
                                }
                                "a" => {
                                    state.text_state.select_all();
                                    state.play_sound("select_all");
                                    window.request_redraw();
                                    return;
                                }
                                "c" => {
                                    if let Some(text) = state.text_state.selected_text() {
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            let _ = clipboard.set_text(text);
                                        }
                                        state.play_sound("copy");
                                    }
                                    return;
                                }
                                "x" => {
                                    if let Some(text) = state.text_state.selected_text() {
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            let _ = clipboard.set_text(text);
                                        }
                                        state.text_state.delete_selection();
                                        state.play_sound("cut");
                                    }
                                    window.request_redraw();
                                    return;
                                }
                                "v" => {
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        if let Ok(text) = clipboard.get_text() {
                                            state.text_state.paste_text(&text);
                                            state.play_sound("paste");
                                        }
                                    }
                                    window.request_redraw();
                                    return;
                                }
                                "z" => {
                                    if state.modifiers.shift_key() {
                                        state.text_state.redo();
                                        state.play_sound("redo");
                                    } else {
                                        state.text_state.undo();
                                        state.play_sound("undo");
                                    }
                                    window.request_redraw();
                                    return;
                                }
                                "y" => {
                                    state.text_state.redo();
                                    state.play_sound("redo");
                                    window.request_redraw();
                                    return;
                                }
                                "n" => {
                                    state.new_file();
                                    window.request_redraw();
                                    return;
                                }
                                "w" => {
                                    let idx = state.text_state.active_tab();
                                    state.close_tab_with_confirm(idx);
                                    window.request_redraw();
                                    return;
                                }
                                "f" => {
                                    state
                                        .text_state
                                        .open_prompt(editor::PromptPurpose::Find, "Buscar texto:");
                                    window.request_redraw();
                                    return;
                                }
                                digit if digit.len() == 1
                                    && digit.chars().next().unwrap().is_ascii_digit()
                                    && digit != "0" =>
                                {
                                    // Ctrl+1..9 salta directo a esa pestaña
                                    // (como en la mayoría de navegadores).
                                    let n: usize = digit.parse().unwrap_or(0);
                                    let idx = n.saturating_sub(1);
                                    if idx < state.text_state.tab_count() {
                                        state.text_state.switch_tab(idx);
                                        state.update_window_title();
                                    }
                                    window.request_redraw();
                                    return;
                                }
                                _ => {}
                            }
                        }
                    }

                    // Ctrl+Tab / Ctrl+Shift+Tab: cambiar de pestaña (Tab
                    // no es un carácter normal, así que se revisa aparte).
                    if state.modifiers.control_key()
                        && event.logical_key == Key::Named(NamedKey::Tab)
                    {
                        let count = state.text_state.tab_count();
                        if count > 1 {
                            let cur = state.text_state.active_tab();
                            let next = if state.modifiers.shift_key() {
                                (cur + count - 1) % count
                            } else {
                                (cur + 1) % count
                            };
                            state.text_state.switch_tab(next);
                            state.update_window_title();
                        }
                        window.request_redraw();
                        return;
                    }

                    // Si el popup está abierto, estas teclas controlan la
                    // selección en vez de editar texto. Cualquier OTRA
                    // tecla cierra el popup pero SIGUE procesándose abajo
                    // como edición normal, en el mismo golpe de tecla (así
                    // no hace falta presionarla dos veces).
                    let mut consumed_by_popup = false;
                    if state.text_state.completion_visible() {
                        consumed_by_popup = true;
                        match &event.logical_key {
                            Key::Named(NamedKey::ArrowDown) => {
                                state.text_state.completion_move_selection(1);
                            }
                            Key::Named(NamedKey::ArrowUp) => {
                                state.text_state.completion_move_selection(-1);
                            }
                            Key::Named(NamedKey::Tab) | Key::Named(NamedKey::Enter) => {
                                state.text_state.completion_accept();
                            }
                            Key::Named(NamedKey::Escape) => {
                                state.text_state.hide_completions();
                            }
                            _ => {
                                state.text_state.hide_completions();
                                consumed_by_popup = false;
                            }
                        }
                    }

                    if !consumed_by_popup {
                        match &event.logical_key {
                            Key::Named(NamedKey::Tab) => {
                                if state.modifiers.shift_key() {
                                    state.text_state.unindent();
                                } else {
                                    state.text_state.indent();
                                }
                                state.play_sound("keypress");
                            }
                            Key::Named(NamedKey::F2) => {
                                state.text_state.toggle_current_line_bookmark();
                            }
                            Key::Named(NamedKey::F3) => {
                                let query = state.text_state.last_search_query.clone();
                                if !query.is_empty() {
                                    state.text_state.find_next(&query);
                                }
                            }
                            Key::Named(NamedKey::Enter) => {
                                state.text_state.insert_newline();
                                state.play_sound("keypress");
                            }
                            Key::Named(NamedKey::Backspace) => {
                                state.text_state.backspace();
                                state.play_sound("keypress");
                            }
                            Key::Named(NamedKey::Delete) => {
                                state.text_state.delete_forward();
                                state.play_sound("keypress");
                            }
                            Key::Named(NamedKey::ArrowLeft) => {
                                state.text_state.move_cursor(-1, 0, state.modifiers.shift_key());
                            }
                            Key::Named(NamedKey::ArrowRight) => {
                                state.text_state.move_cursor(1, 0, state.modifiers.shift_key());
                            }
                            Key::Named(NamedKey::ArrowUp) => {
                                state.text_state.move_cursor(0, -1, state.modifiers.shift_key());
                            }
                            Key::Named(NamedKey::ArrowDown) => {
                                state.text_state.move_cursor(0, 1, state.modifiers.shift_key());
                            }
                            _ => {
                                // Caracteres normales (letras, números,
                                // símbolos): winit ya nos da el texto
                                // "traducido" según el layout de teclado
                                // del usuario en `event.text`.
                                if let Some(text) = &event.text {
                                    let mut should_trigger = false;
                                    for c in text.chars() {
                                        state.text_state.insert_char(c);
                                        if c == '.' || c.is_alphanumeric() {
                                            should_trigger = true;
                                        }
                                    }
                                    if !text.is_empty() {
                                        state.play_sound("keypress");
                                    }
                                    if should_trigger {
                                        state.request_completion();
                                    }
                                }
                            }
                        }
                    }

                    window.request_redraw();
                }
                WindowEvent::RedrawRequested => match state.render() {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        state.resize(state.size)
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        error!("wgpu se quedó sin memoria de GPU; cerrando");
                        elwt.exit();
                    }
                    Err(e) => error!("error de superficie: {e:?}"),
                },
                _ => {}
            },
            Event::AboutToWait => window.request_redraw(),
            _ => {}
        }
    })?;

    Ok(())
}
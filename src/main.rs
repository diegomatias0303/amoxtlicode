//! AmoxliCode - Fase 1: Shell (ventana + wgpu)
//!
//! Establece el viewport acelerado por hardware. Este módulo NO realiza
//! todavía trabajo de video, texto ni sintaxis: solo garantiza que el
//! pipeline de render (Instance -> Adapter -> Device -> Queue -> Surface)
//! funciona con un loop de render a 60 FPS (V-Sync / Fifo).

#![windows_subsystem = "windows"]
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

// Color de fondo por defecto viene del tema activo (`text_state.background_color()`).

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
    last_click_time: Option<std::time::Instant>,
    last_click_pos: (f32, f32),
    click_count: u8,
    should_exit: bool,
    ui_pipeline: ui::SolidQuadPipeline,
    overlay_ui_pipeline: ui::SolidQuadPipeline,
    picker_overlay_pipeline: ui::SolidQuadPipeline,

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
    async fn new(window: Arc<Window>, explicit_video: Option<&str>) -> Result<Self> {
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

        // Creamos TextState primero para reutilizar sus Settings ya cargados
        // sin leer amoxlicode_settings.json dos veces.
        let text_state = TextState::new(&device, &queue, surface_format, size.width, size.height);

        // --- Capa 0: decidir qué fondo usar al arrancar ---
        // Si el usuario pasó un video por CLI, se usa ese. Si no, se respeta
        // estrictamente la preferencia guardada (incluyendo Background::None);
        // solo en el primer arranque sin configuración previa se intenta cargar
        // el video de demostración `assets/background.mp4`.
        let resolve_default_video = || -> Option<PathBuf> {
            let cwd_default = PathBuf::from("assets/background.mp4");
            if cwd_default.exists() {
                return Some(cwd_default);
            }
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    let exe_default = dir.join("assets").join("background.mp4");
                    if exe_default.exists() {
                        return Some(exe_default);
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                if let Ok(home) = std::env::var("HOME") {
                    let user_share = PathBuf::from(home)
                        .join(".local/share/amoxlicode/assets/background.mp4");
                    if user_share.exists() {
                        return Some(user_share);
                    }
                }
                for sys_path in [
                    "/usr/local/share/amoxlicode/assets/background.mp4",
                    "/usr/share/amoxlicode/assets/background.mp4",
                ] {
                    let p = PathBuf::from(sys_path);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
            None
        };

        let initial_bg_path: Option<PathBuf> = if let Some(cli_video) = explicit_video {
            let p = PathBuf::from(cli_video);
            if p.exists() { Some(p) } else { None }
        } else {
            match &text_state.settings.background {
                settings::Background::Custom(p) if p.exists() => Some(p.clone()),
                settings::Background::Custom(_) => None,
                settings::Background::None => {
                    if settings::Settings::exists_on_disk() {
                        None
                    } else {
                        resolve_default_video()
                    }
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

        let ui_pipeline = ui::SolidQuadPipeline::new(&device, surface_format);
        let overlay_ui_pipeline = ui::SolidQuadPipeline::new(&device, surface_format);
        let picker_overlay_pipeline = ui::SolidQuadPipeline::new(&device, surface_format);
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
            last_click_time: None,
            last_click_pos: (0.0, 0.0),
            click_count: 0,
            should_exit: false,
            ui_pipeline,
            overlay_ui_pipeline,
            picker_overlay_pipeline,
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
            editor::PanelAction::ToggleAutoClosePairs => {
                self.text_state.settings.toggle_auto_close_pairs();
                self.text_state.settings.save();
                self.text_state.rebuild_panel_buffer();
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

    /// Sincroniza el texto actual con el servidor LSP correspondiente al
    /// lenguaje activo (si existe). No hace nada en lenguajes sin LSP.
    fn sync_active_doc_to_lsp(&self) {
        let language = self.text_state.active_language();
        if language.lsp_command().0.is_empty() {
            return;
        }
        let text = self.text_state.full_text();
        let uri = self.text_state.active_uri();
        let _ = self.lsp_tx.send(lsp::LspRequest::SyncDoc {
            uri,
            language,
            text,
        });
    }

    /// Sincroniza el texto actual con el servidor LSP y le pide autocompletado
    /// en la posición actual del cursor. No bloquea: la respuesta llega más
    /// tarde por `lsp_rx` y se recoge en `render()`.
    fn request_completion(&mut self) {
        let language = self.text_state.active_language();
        if language.lsp_command().0.is_empty() {
            return;
        }
        let text = self.text_state.full_text();
        let uri = self.text_state.active_uri();
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
            editor::MenuAction::OpenRecent(idx) => {
                if let Some(path) = self.text_state.settings.recent_files.get(idx).cloned() {
                    self.open_file_path(path);
                }
            }
            editor::MenuAction::Save => {
                self.save_file();
            }
            editor::MenuAction::SaveAs => {
                self.save_file_as();
            }
            editor::MenuAction::SaveAll => {
                self.save_all_dirty();
            }
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
                            if self.save_all_dirty() {
                                self.should_exit = true;
                            }
                        }
                        rfd::MessageDialogResult::No => {
                            self.should_exit = true;
                        }
                        _ => {}
                    }
                } else {
                    self.should_exit = true;
                }
            }

            // Editar
            editor::MenuAction::Undo => {
                self.text_state.undo();
                self.play_sound("undo");
                self.update_window_title();
            }
            editor::MenuAction::Redo => {
                self.text_state.redo();
                self.play_sound("redo");
                self.update_window_title();
            }
            editor::MenuAction::Cut => {
                if let Some(text) = self.text_state.selected_text() {
                    let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
                    self.text_state.delete_selection();
                    self.play_sound("cut");
                    self.update_window_title();
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
                        self.update_window_title();
                    }
                }
            }
            editor::MenuAction::Indent => {
                self.text_state.indent();
                self.update_window_title();
            }
            editor::MenuAction::Unindent => {
                self.text_state.unindent();
                self.update_window_title();
            }
            editor::MenuAction::DuplicateLine => {
                self.text_state.duplicate_line();
                self.update_window_title();
            }
            editor::MenuAction::ToggleComment => {
                self.text_state.toggle_line_comment();
                self.update_window_title();
            }
            editor::MenuAction::MoveLineUp => {
                self.text_state.move_line(-1);
                self.update_window_title();
            }
            editor::MenuAction::MoveLineDown => {
                self.text_state.move_line(1);
                self.update_window_title();
            }
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
            editor::MenuAction::Replace => {
                self.text_state
                    .open_prompt(editor::PromptPurpose::Replace, "Buscar y reemplazar:");
            }
            editor::MenuAction::GoToLine => {
                self.text_state.open_prompt(editor::PromptPurpose::GoToLine, "Ir a línea:");
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
                self.sync_active_doc_to_lsp();
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
            editor::MenuAction::RunCode => self.run_code(),
        }
    }

    #[allow(dead_code)]
    fn apply_toolbar_action(&mut self, action: editor::ToolbarAction) {
        self.apply_menu_action(action);
    }

    /// Ejecuta el archivo de la pestaña activa en la terminal integrada según su lenguaje (tipo NetBeans / VS Studio).
    fn run_code(&mut self) {
        if (self.text_state.active_file_path().is_none() || self.text_state.active_is_dirty())
            && !self.save_file()
        {
            return;
        }

        let path = match self.text_state.active_file_path() {
            Some(p) => p,
            None => return,
        };

        let lang = self.text_state.active_language();
        self.text_state.terminal.is_open = true;
        self.text_state.terminal.active_tab = crate::terminal::TerminalTab::Output;
        self.text_state.terminal_focused = true;
        self.text_state.terminal.start_run(&path, lang);
        self.text_state.update_viewport_sizes();
        self.text_state.rebuild_terminal_buffer();
    }

    /// Abre una ventana nueva de terminal externa en la carpeta del archivo activo.
    #[cfg(target_os = "windows")]
    fn open_terminal_here(&self) {
        let dir = self
            .text_state
            .active_file_path()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .filter(|d| !d.as_os_str().is_empty())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let dir_str = dir.to_string_lossy().replace('\'', "''");

        let mut cmd = std::process::Command::new("powershell");
        cmd.arg("-NoExit")
            .arg("-Command")
            .arg(format!("Set-Location -LiteralPath '{dir_str}'"));

        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        cmd.creation_flags(CREATE_NEW_CONSOLE);

        if let Err(e) = cmd.spawn() {
            error!("no se pudo abrir la terminal: {e:?}");
        }
    }

    /// Abre una ventana nueva de terminal externa en Linux/Unix en la carpeta del archivo activo.
    #[cfg(not(target_os = "windows"))]
    fn open_terminal_here(&self) {
        let dir = self
            .text_state
            .active_file_path()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .filter(|d| !d.as_os_str().is_empty())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let terminals = [
            ("x-terminal-emulator", vec!["--working-directory"]),
            ("gnome-terminal", vec!["--working-directory"]),
            ("konsole", vec!["--workdir"]),
            ("xfce4-terminal", vec!["--working-directory"]),
            ("alacritty", vec!["--working-directory"]),
            ("kitty", vec!["--directory"]),
            ("xterm", vec![]),
        ];

        let mut launched = false;
        let dir_str = dir.to_string_lossy();
        for (term, args) in terminals {
            let mut cmd = std::process::Command::new(term);
            cmd.current_dir(&dir);
            if !args.is_empty() {
                cmd.args(args).arg(dir_str.as_ref());
            }
            if cmd.spawn().is_ok() {
                launched = true;
                break;
            }
        }
        if !launched {
            if let Ok(term) = std::env::var("TERMINAL") {
                let mut cmd = std::process::Command::new(term);
                cmd.current_dir(&dir);
                let _ = cmd.spawn();
            } else {
                error!("no se encontró ningún emulador de terminal compatible en el sistema");
            }
        }
    }

    /// Guarda el contenido de la pestaña activa. Si nunca se guardó
    /// antes, primero pide una ruta con el diálogo nativo (por defecto .txt).
    /// Devuelve `true` si el archivo se guardó correctamente, o `false` si
    /// el usuario canceló el diálogo o ocurrió un error de escritura.
    fn save_file(&mut self) -> bool {
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
                    None => return false,
                }
            }
        };

        let text = self.text_state.full_text_for_save();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                info!("Archivo guardado en {path:?}");
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let lang = syntax::Language::from_extension(ext);
                    self.text_state.set_active_language(lang);
                }
                self.text_state.settings.push_recent_file(path.clone());
                self.text_state.settings.save();
                self.text_state.set_active_file_path(path);
                self.text_state.mark_saved();
                self.update_window_title();
                self.sync_active_doc_to_lsp();
                true
            }
            Err(e) => {
                error!("no se pudo guardar el archivo: {e:?}");
                rfd::MessageDialog::new()
                    .set_title("Error al guardar")
                    .set_description(format!("No se pudo guardar el archivo:\n{e}"))
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();
                false
            }
        }
    }

    /// "Guardar como": siempre pregunta la ruta, aunque la pestaña ya
    /// tuviera una asignada. Devuelve `true` si se guardó con éxito.
    fn save_file_as(&mut self) -> bool {
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
            None => return false,
        };

        let text = self.text_state.full_text_for_save();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                info!("Archivo guardado en {path:?}");
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let lang = syntax::Language::from_extension(ext);
                    self.text_state.set_active_language(lang);
                }
                self.text_state.settings.push_recent_file(path.clone());
                self.text_state.settings.save();
                self.text_state.set_active_file_path(path);
                self.text_state.mark_saved();
                self.update_window_title();
                self.sync_active_doc_to_lsp();
                true
            }
            Err(e) => {
                error!("no se pudo guardar el archivo: {e:?}");
                rfd::MessageDialog::new()
                    .set_title("Error al guardar")
                    .set_description(format!("No se pudo guardar el archivo:\n{e}"))
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();
                false
            }
        }
    }

    /// Abre un archivo en una pestaña (o pregunta si crearlo si no existe, al estilo Notepad++).
    fn open_file_path(&mut self, path: std::path::PathBuf) {
        self.open_file_path_opts(path, false);
    }

    /// Abre un archivo o lo crea (con force_create o preguntando al usuario si no existe).
    fn open_file_path_opts(&mut self, path: std::path::PathBuf, force_create: bool) {
        let abs_path = if path.is_relative() {
            std::env::current_dir().map(|cwd| cwd.join(&path)).unwrap_or(path)
        } else {
            path
        };

        if !abs_path.exists() {
            let file_name = abs_path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| abs_path.display().to_string());

            let should_create = if force_create {
                true
            } else {
                let desc = format!("El archivo \"{}\" no existe.\n\n¿Deseas crearlo?", file_name);
                let result = rfd::MessageDialog::new()
                    .set_title("AmoxliCode")
                    .set_description(&desc)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                result == rfd::MessageDialogResult::Yes
            };

            if should_create {
                if let Some(parent) = abs_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                }
                match std::fs::write(&abs_path, "") {
                    Ok(()) => {
                        info!("Archivo creado exitosamente: {:?}", abs_path);
                        self.text_state.settings.push_recent_file(abs_path.clone());
                        self.text_state.settings.save();
                        self.text_state.open_file_as_tab(abs_path, "");
                        self.update_window_title();
                        self.sync_active_doc_to_lsp();
                    }
                    Err(e) => {
                        error!("No se pudo crear el archivo {:?}: {e:?}", abs_path);
                        let err_desc = format!("No se pudo crear el archivo \"{}\":\n{}", file_name, e);
                        rfd::MessageDialog::new()
                            .set_title("Error al crear archivo")
                            .set_description(&err_desc)
                            .set_buttons(rfd::MessageButtons::Ok)
                            .show();
                    }
                }
            }
            return;
        }

        match std::fs::read(&abs_path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                self.text_state.settings.push_recent_file(abs_path.clone());
                self.text_state.settings.save();
                self.text_state.open_file_as_tab(abs_path, &text);
                self.update_window_title();
                self.sync_active_doc_to_lsp();
            }
            Err(e) => error!("No se pudo abrir el archivo {:?}: {e:?}", abs_path),
        }
    }

    fn open_file(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Todos los archivos compatibles", &[
                "txt", "py", "rs", "c", "h", "cpp", "hpp", "java", "js", "ts", "html", "htm", "css", "sql", "json", "md", "cs", "php", "go", "sh", "bash", "xml"
            ])
            .add_filter("Archivos de texto (*.txt)", &["txt"])
            .add_filter("Código fuente (*.py, *.rs, *.c, *.js...)", &[
                "py", "rs", "c", "h", "cpp", "hpp", "java", "js", "ts", "html", "css", "sql", "json", "md", "cs", "php", "go", "sh", "bash", "xml"
            ])
            .add_filter("Todos los archivos (*.*)", &["*"])
            .pick_file();

        if let Some(path) = picked {
            self.open_file_path(path);
        }
    }

    /// Actualiza el título de la ventana según la pestaña activa y su estado modificado.
    fn update_window_title(&self) {
        let dirty_mark = if self.text_state.active_is_dirty() { "*" } else { "" };
        self.window.set_title(&format!(
            "AmoxliCode - {dirty_mark}{}",
            self.text_state.active_title()
        ));
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
    /// Devuelve `true` solo si todas se guardaron sin cancelación ni error.
    fn save_all_dirty(&mut self) -> bool {
        for idx in 0..self.text_state.tab_count() {
            if self.text_state.is_dirty(idx) {
                self.text_state.switch_tab(idx);
                if !self.save_file() {
                    return false;
                }
            }
        }
        true
    }

    /// Cierra una pestaña y notifica al servidor LSP (`didClose`) para evitar
    /// fugas de documentos abiertos en memoria.
    fn close_tab_and_notify_lsp(&mut self, idx: usize) {
        if let Some((uri, language)) = self.text_state.close_tab(idx) {
            if !language.lsp_command().0.is_empty() {
                let _ = self.lsp_tx.send(lsp::LspRequest::CloseDoc { uri, language });
            }
        }
    }

    /// Cierra una pestaña, pero si tiene cambios sin guardar primero
    /// pregunta con un diálogo nativo (Sí/No/Cancelar).
    fn close_tab_with_confirm(&mut self, idx: usize) {
        if !self.text_state.is_dirty(idx) {
            self.close_tab_and_notify_lsp(idx);
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
                // Solo cerrar la pestaña si realmente se guardó (si el usuario
                // canceló el diálogo "Guardar como", NO perder su trabajo).
                if self.save_file() {
                    self.close_tab_and_notify_lsp(idx);
                } else if was_active < self.text_state.tab_count() {
                    self.text_state.switch_tab(was_active);
                }
            }
            rfd::MessageDialogResult::No => {
                self.close_tab_and_notify_lsp(idx);
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
        if self.size.width == 0
            || self.size.height == 0
            || self.window.is_minimized() == Some(true)
        {
            return Ok(());
        }

        self.update_video_texture();

        let t = self.start_time.elapsed().as_secs_f32();
        self.text_state.set_anim_time(t);

        // Parpadeo del cursor: medio segundo prendido, medio apagado.
        let blink_on = (self.start_time.elapsed().as_millis() / 500).is_multiple_of(2);
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

        // Juntamos TODOS los rectángulos sólidos en orden estricto de profundidad (Z):
        //   A) Área de código (fondo sobre video -> línea activa -> marcadores -> selección -> cursor -> scrollbar)
        //   B) Barra superior y barra de pestañas
        //   C) Dock de terminal integrada (siempre encima del área de código, jamás cubierta por ella)
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

        // --- A. ÁREA DE EDITOR (recortada exactamente hasta editor_bottom) ---
        let editor_bottom = self.text_state.editor_bottom();

        // A1. Fondo con opacidad personalizable sobre el video/imagen de fondo
        if self.background.is_some() {
            let [bg_r, bg_g, bg_b] = self.text_state.background_color();
            ui_rects.push(ui::Rect {
                x: 0.0,
                y: editor::TOP_OFFSET,
                w: self.size.width as f32,
                h: (editor_bottom - editor::TOP_OFFSET).max(0.0),
                color: [bg_r, bg_g, bg_b],
                alpha: self.text_state.settings.editor_opacity,
            });
        }

        // A2. Resaltado sutil de la línea activa (cubre gutter + código) y separador vertical del gutter
        let gutter_sep_color = match self.text_state.settings.theme {
            settings::ThemeName::Light => [0.0, 0.0, 0.0],
            _ => [1.0, 1.0, 1.0],
        };
        ui_rects.push(ui::Rect {
            x: editor::PADDING + editor::GUTTER_WIDTH - 8.0,
            y: editor::TOP_OFFSET,
            w: 1.0,
            h: (editor_bottom - editor::TOP_OFFSET).max(0.0),
            color: gutter_sep_color,
            alpha: 0.09,
        });
        if let Some((lx, ly, lw, lh)) = self.text_state.active_line_rect() {
            ui_rects.push(ui::Rect {
                x: 0.0,
                y: ly,
                w: lx + lw,
                h: lh,
                color: gutter_sep_color,
                alpha: 0.045,
            });
        }

        // A2b. Resaltado de delimitadores coincidentes (), [], {}
        for (bx, by, bw, bh) in self.text_state.matching_bracket_rects() {
            ui_rects.push(ui::Rect {
                x: bx - 1.0,
                y: by,
                w: bw + 2.0,
                h: bh,
                color: accent,
                alpha: 0.24,
            });
            ui_rects.push(ui::Rect {
                x: bx - 1.0,
                y: by + bh - 2.0,
                w: bw + 2.0,
                h: 2.0,
                color: accent,
                alpha: 0.85,
            });
        }

        // A3. Marcadores de línea estilo Dev-C++ y marcador temporal de búsqueda
        let is_marker_rgb = self.text_state.settings.is_rgb_active(settings::RgbElement::Markers);
        let marker_color_u8 = self.text_state.settings.color_for(Some("line_marker"));
        let default_marker_color = [
            marker_color_u8[0] as f32 / 255.0,
            marker_color_u8[1] as f32 / 255.0,
            marker_color_u8[2] as f32 / 255.0,
        ];
        let marker_alpha = self.text_state.settings.marker_opacity;
        let line_h = self.text_state.line_height();
        let code_area_w = (self.size.width as f32
            - (editor::PADDING + editor::GUTTER_WIDTH)
            - editor::SCROLLBAR_WIDTH)
            .max(0.0);

        for (_line_idx, line_y) in self.text_state.visible_bookmarks() {
            let marker_color = if is_marker_rgb {
                crate::color::chroma_rgb_f32(t, line_y)
            } else {
                default_marker_color
            };
            ui_rects.push(ui::Rect {
                x: editor::PADDING + editor::GUTTER_WIDTH,
                y: line_y,
                w: code_area_w,
                h: line_h,
                color: marker_color,
                alpha: marker_alpha,
            });
            ui_rects.push(ui::Rect {
                x: 0.0,
                y: line_y,
                w: editor::PADDING + editor::GUTTER_WIDTH,
                h: line_h,
                color: marker_color,
                alpha: (marker_alpha * 1.5).min(0.65),
            });
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
                w: code_area_w,
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

        // A4. Selección de texto
        for (sx, sy, sw, sh) in self.text_state.selection_rects() {
            ui_rects.push(ui::Rect {
                x: sx,
                y: sy,
                w: sw,
                h: sh,
                color: [0.25, 0.45, 0.85],
                alpha: 0.35,
            });
        }

        // A5. Cursor del editor
        if self.text_state.cursor_visible() {
            let (cx, cy, cw, ch) = self.text_state.cursor_rect();
            ui_rects.push(ui::Rect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
                color: self.text_state.cursor_color(),
                alpha: 1.0,
            });
        }

        // A6. Scrollbar en el borde derecho (personalizable en color y opacidad + soporte RGB)
        let (sb_x, sb_y, sb_w, sb_h) = self.text_state.scrollbar_rect();
        let (th_x, th_y, th_w, th_h) = self.text_state.scrollbar_thumb_rect();
        let sb_color = if self.text_state.settings.is_rgb_active(settings::RgbElement::Scrollbar) {
            crate::color::chroma_rgb_f32(t, th_y)
        } else {
            self.text_state.settings.scrollbar_color_rgb()
        };
        let sb_alpha = self.text_state.settings.scrollbar_opacity;

        ui_rects.push(ui::Rect {
            x: sb_x,
            y: sb_y,
            w: sb_w,
            h: sb_h,
            color: [0.08, 0.09, 0.12],
            alpha: (sb_alpha * 0.4).min(0.5),
        });
        ui_rects.push(ui::Rect {
            x: th_x,
            y: th_y,
            w: th_w,
            h: th_h,
            color: sb_color,
            alpha: sb_alpha,
        });

        // --- B. BARRA SUPERIOR Y BARRA DE PESTAÑAS ---
        let (tx, ty, tw, th) = self.text_state.toolbar_rect();
        ui_rects.push(ui::Rect { x: tx, y: ty, w: tw, h: th, color: topbar_bg, alpha: topbar_alpha });

        for (_, menu_opt, _action_opt, x1, x2) in editor::TOOLBAR_ITEMS {
            let is_open = menu_opt.is_some() && self.text_state.active_menu() == menu_opt;
            let is_hovered = mx >= x1 && mx < x2 && (0.0..=editor::TOOLBAR_HEIGHT).contains(&my);
            if is_open {
                ui_rects.push(ui::Rect {
                    x: x1,
                    y: 3.0,
                    w: x2 - x1,
                    h: editor::TOOLBAR_HEIGHT - 6.0,
                    color: lighten(topbar_bg, 0.12),
                    alpha: topbar_alpha,
                });
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
        let badge_hover = mx >= bx && mx < bx + bw && (0.0..=editor::TOOLBAR_HEIGHT).contains(&my);
        ui_rects.push(ui::Rect {
            x: bx - 1.0,
            y: by - 1.0,
            w: bw + 2.0,
            h: bh + 2.0,
            color: if badge_hover { accent } else { [0.22, 0.25, 0.33] },
            alpha: if badge_hover { 0.95 } else { 0.6 },
        });
        ui_rects.push(ui::Rect {
            x: bx,
            y: by,
            w: bw,
            h: bh,
            color: if badge_hover { lighten(topbar_bg, 0.12) } else { lighten(topbar_bg, 0.05) },
            alpha: topbar_alpha,
        });
        ui_rects.push(ui::Rect {
            x: lx - 2.0,
            y: ly - 2.0,
            w: lw + 4.0,
            h: lh + 4.0,
            color: accent,
            alpha: 0.25 * topbar_alpha,
        });
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

        // Barra de pestañas
        let (bx, by, bw, bh) = self.text_state.tab_bar_rect();
        ui_rects.push(ui::Rect { x: bx, y: by, w: bw, h: bh, color: tab_bar_bg, alpha: topbar_alpha });

        for i in 0..self.text_state.tab_count() {
            let (tx, ty, tw, th) = self.text_state.tab_rect(i);
            if tx + tw < 0.0 || tx > self.size.width as f32 {
                continue;
            }
            let is_active = i == self.text_state.active_tab();
            let is_hovered = !is_active && mx >= tx && mx <= tx + tw && my >= ty && my <= ty + th;

            if is_active {
                ui_rects.push(ui::Rect {
                    x: tx,
                    y: ty,
                    w: tw,
                    h: th,
                    color: tab_active_bg,
                    alpha: topbar_alpha,
                });
                ui_rects.push(ui::Rect {
                    x: tx,
                    y: ty,
                    w: tw,
                    h: 2.5,
                    color: accent,
                    alpha: topbar_alpha,
                });
                ui_rects.push(ui::Rect {
                    x: tx + tw - 1.0,
                    y: ty,
                    w: 1.0,
                    h: th,
                    color: [0.22, 0.25, 0.32],
                    alpha: 0.7,
                });
            } else {
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
                ui_rects.push(ui::Rect {
                    x: tx + tw - 1.0,
                    y: ty + 4.0,
                    w: 1.0,
                    h: th - 8.0,
                    color: [0.18, 0.20, 0.25],
                    alpha: 0.6,
                });
            }

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

        // Botón '+' (Nueva pestaña rápida)
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

        // --- C. DOCK DE TERMINAL INTEGRADA (dibujada al final de ui_rects para quedar encima del editor) ---
        if self.text_state.terminal.is_open {
            let (tx, ty, tw, th) = self.text_state.terminal_rect();
            let term_op = self.text_state.settings.terminal_opacity;
            let bg = self.text_state.settings.color_for(Some("terminal_bg"));
            let bg_f32 = [bg[0] as f32 / 255.0, bg[1] as f32 / 255.0, bg[2] as f32 / 255.0];
            let topbar_bg_u8 = self.text_state.settings.color_for(Some("topbar_bg"));
            let topbar_bg_f32 = [
                topbar_bg_u8[0] as f32 / 255.0,
                topbar_bg_u8[1] as f32 / 255.0,
                topbar_bg_u8[2] as f32 / 255.0,
            ];

            // Fondo general del dock
            ui_rects.push(ui::Rect { x: tx, y: ty, w: tw, h: th, color: bg_f32, alpha: term_op });

            // Borde superior / manija de redimensionamiento
            ui_rects.push(ui::Rect { x: tx, y: ty, w: tw, h: 2.0, color: accent, alpha: 0.9 });

            // Fondo de la barra de título / pestañas (Header)
            ui_rects.push(ui::Rect { x: tx, y: ty + 2.0, w: tw, h: 26.0, color: topbar_bg_f32, alpha: 0.95 });
            ui_rects.push(ui::Rect { x: tx, y: ty + 27.0, w: tw, h: 1.0, color: [accent[0] * 0.4, accent[1] * 0.4, accent[2] * 0.4], alpha: 0.7 });

            // Pestaña Salida
            let out_rect = self.text_state.terminal_tab_output_rect();
            if self.text_state.terminal.active_tab == crate::terminal::TerminalTab::Output {
                ui_rects.push(ui::Rect { x: out_rect.0, y: out_rect.1, w: out_rect.2, h: out_rect.3, color: bg_f32, alpha: 0.95 });
                ui_rects.push(ui::Rect { x: out_rect.0, y: out_rect.1 + out_rect.3 - 2.0, w: out_rect.2, h: 2.0, color: accent, alpha: 1.0 });
            }

            // Pestaña Terminal
            let shell_rect = self.text_state.terminal_tab_shell_rect();
            if self.text_state.terminal.active_tab == crate::terminal::TerminalTab::Shell {
                ui_rects.push(ui::Rect { x: shell_rect.0, y: shell_rect.1, w: shell_rect.2, h: shell_rect.3, color: bg_f32, alpha: 0.95 });
                ui_rects.push(ui::Rect { x: shell_rect.0, y: shell_rect.1 + shell_rect.3 - 2.0, w: shell_rect.2, h: 2.0, color: accent, alpha: 1.0 });
            }

            let in_rect = |(rx, ry, rw, rh): (f32, f32, f32, f32)| {
                mx >= rx && mx <= rx + rw && my >= ry && my <= ry + rh
            };

            let run_btn = self.text_state.terminal_btn_run_rect();
            let run_hover = in_rect(run_btn);
            let run_color = if run_hover { [0.22, 0.65, 0.35] } else { [0.18, 0.55, 0.28] };
            ui_rects.push(ui::Rect { x: run_btn.0, y: run_btn.1, w: run_btn.2, h: run_btn.3, color: run_color, alpha: if run_hover { 1.0 } else { 0.85 } });

            let stop_btn = self.text_state.terminal_btn_stop_rect();
            let stop_hover = in_rect(stop_btn);
            let stop_color = if self.text_state.terminal.is_running {
                if stop_hover { [0.85, 0.28, 0.28] } else { [0.75, 0.22, 0.22] }
            } else if stop_hover {
                [0.42, 0.42, 0.46]
            } else {
                [0.35, 0.35, 0.38]
            };
            let stop_alpha = if self.text_state.terminal.is_running {
                if stop_hover { 1.0 } else { 0.9 }
            } else {
                0.5
            };
            ui_rects.push(ui::Rect { x: stop_btn.0, y: stop_btn.1, w: stop_btn.2, h: stop_btn.3, color: stop_color, alpha: stop_alpha });

            let clear_btn = self.text_state.terminal_btn_clear_rect();
            let clear_hover = in_rect(clear_btn);
            let clear_color = if clear_hover { [0.36, 0.38, 0.48] } else { [0.28, 0.30, 0.38] };
            ui_rects.push(ui::Rect { x: clear_btn.0, y: clear_btn.1, w: clear_btn.2, h: clear_btn.3, color: clear_color, alpha: if clear_hover { 0.9 } else { 0.7 } });

            let z_out_btn = self.text_state.terminal_btn_zoom_out_rect();
            let z_out_hover = in_rect(z_out_btn);
            let z_out_color = if z_out_hover { [0.30, 0.33, 0.42] } else { [0.22, 0.24, 0.30] };
            ui_rects.push(ui::Rect { x: z_out_btn.0, y: z_out_btn.1, w: z_out_btn.2, h: z_out_btn.3, color: z_out_color, alpha: if z_out_hover { 0.95 } else { 0.7 } });

            let z_lbl_btn = self.text_state.terminal_btn_zoom_label_rect();
            let z_lbl_hover = in_rect(z_lbl_btn);
            let z_lbl_color = if z_lbl_hover { [0.26, 0.28, 0.38] } else { [0.18, 0.20, 0.26] };
            ui_rects.push(ui::Rect { x: z_lbl_btn.0, y: z_lbl_btn.1, w: z_lbl_btn.2, h: z_lbl_btn.3, color: z_lbl_color, alpha: if z_lbl_hover { 0.95 } else { 0.7 } });

            let z_in_btn = self.text_state.terminal_btn_zoom_in_rect();
            let z_in_hover = in_rect(z_in_btn);
            let z_in_color = if z_in_hover { [0.30, 0.33, 0.42] } else { [0.22, 0.24, 0.30] };
            ui_rects.push(ui::Rect { x: z_in_btn.0, y: z_in_btn.1, w: z_in_btn.2, h: z_in_btn.3, color: z_in_color, alpha: if z_in_hover { 0.95 } else { 0.7 } });

            let close_btn = self.text_state.terminal_btn_close_rect();
            let close_hover = in_rect(close_btn);
            let close_color = if close_hover { [0.70, 0.25, 0.25] } else { [0.55, 0.22, 0.22] };
            ui_rects.push(ui::Rect { x: close_btn.0, y: close_btn.1, w: close_btn.2, h: close_btn.3, color: close_color, alpha: if close_hover { 0.95 } else { 0.7 } });

            // Scrollbar track y thumb de la terminal
            let lines = self.text_state.terminal.active_lines();
            let (_cx, _cy, _cw, ch) = self.text_state.terminal_content_rect();
            let line_height = self.text_state.terminal.eff_line_height();
            let visible_lines = (ch / line_height).floor() as usize;
            if lines.len() > visible_lines && visible_lines > 0 {
                let sb_rect = self.text_state.terminal_scrollbar_rect();
                ui_rects.push(ui::Rect { x: sb_rect.0, y: sb_rect.1, w: sb_rect.2, h: sb_rect.3, color: [0.2, 0.2, 0.25], alpha: 0.3 });
                let ratio = visible_lines as f32 / lines.len() as f32;
                let thumb_h = (sb_rect.3 * ratio).max(16.0);
                let max_scroll = lines.len() - visible_lines;
                let scroll = self.text_state.terminal.active_scroll().min(max_scroll);
                let scroll_ratio = scroll as f32 / max_scroll as f32;
                let thumb_y = sb_rect.1 + (sb_rect.3 - thumb_h) * scroll_ratio;
                ui_rects.push(ui::Rect { x: sb_rect.0, y: thumb_y, w: sb_rect.2, h: thumb_h, color: accent, alpha: 0.65 });
            }

            // Barra de entrada inferior (Input bar)
            let (_ix, iy, iw, ih) = self.text_state.terminal_input_rect();
            ui_rects.push(ui::Rect { x: 0.0, y: iy, w: iw, h: ih, color: [bg_f32[0] * 0.8, bg_f32[1] * 0.8, bg_f32[2] * 0.8], alpha: 0.95 });
            ui_rects.push(ui::Rect { x: 0.0, y: iy, w: iw, h: 1.0, color: [0.3, 0.35, 0.45], alpha: 0.5 });

            // Cursor parpadeante escalado con el zoom actual de la terminal
            if self.text_state.terminal_focused && blink_on {
                let cursor_x = self.text_state.terminal_input_cursor_x();
                let cursor_h = self.text_state.terminal.eff_line_height().min(20.0);
                ui_rects.push(ui::Rect { x: cursor_x, y: iy + 5.0, w: 2.0, h: cursor_h, color: accent, alpha: 1.0 });
            }
        }

        // --- CAPA OVERLAY: Paneles flotantes, menús, prompts, autocompletado y tooltips ---
        let mut overlay_rects: Vec<ui::Rect> = Vec::new();
        let mut picker_overlay_rects: Vec<ui::Rect> = Vec::new();

        // Popup de autocompletado LSP con tarjeta oscura + borde + fila seleccionada
        if let Some((cx, cy, cw, ch)) = self.text_state.completion_rect() {
            let menu_bg = self.text_state.settings.menu_bg_rgb();
            let menu_alpha = self.text_state.settings.menu_opacity.max(0.92);
            overlay_rects.push(ui::Rect {
                x: cx - 1.0,
                y: cy - 1.0,
                w: cw + 2.0,
                h: ch + 2.0,
                color: [0.28, 0.32, 0.44],
                alpha: menu_alpha,
            });
            overlay_rects.push(ui::Rect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
                color: menu_bg,
                alpha: menu_alpha,
            });
            if let Some((rx, ry, rw, rh)) = self.text_state.completion_selected_row_rect() {
                overlay_rects.push(ui::Rect {
                    x: rx,
                    y: ry,
                    w: rw,
                    h: rh,
                    color: lighten(menu_bg, 0.14),
                    alpha: menu_alpha,
                });
                overlay_rects.push(ui::Rect {
                    x: rx,
                    y: ry,
                    w: 3.0,
                    h: rh,
                    color: accent,
                    alpha: 1.0,
                });
            }
        }

        if self.text_state.panel_open() {
            let (px, py, pw, ph) = self.text_state.panel_rect();
            let menu_alpha = self.text_state.settings.menu_opacity;
            overlay_rects.push(ui::Rect { x: px - 1.0, y: py - 1.0, w: pw + 2.0, h: ph + 2.0, color: [0.22, 0.25, 0.32], alpha: (0.9 * menu_alpha).min(0.85) });
            overlay_rects.push(ui::Rect { x: px, y: py, w: pw, h: ph, color: [0.08, 0.08, 0.10], alpha: menu_alpha });

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
            overlay_rects.push(ui::Rect { x: px - 1.0, y: py - 1.0, w: pw + 2.0, h: ph + 2.0, color: [0.28, 0.32, 0.42], alpha: menu_alpha });
            overlay_rects.push(ui::Rect { x: px, y: py, w: pw, h: ph, color: [0.08, 0.08, 0.10], alpha: menu_alpha });
        }

        if self.text_state.prompt_open() {
            let (qx, qy, qw, qh) = self.text_state.prompt_rect();
            let menu_alpha = self.text_state.settings.menu_opacity.max(0.94);
            overlay_rects.push(ui::Rect { x: qx - 1.0, y: qy - 1.0, w: qw + 2.0, h: qh + 2.0, color: accent, alpha: 0.75 });
            overlay_rects.push(ui::Rect { x: qx, y: qy, w: qw, h: qh, color: [0.08, 0.09, 0.12], alpha: menu_alpha });
            // Franja inferior interactiva para botón(es) de acción
            let hit = self.text_state.prompt_hit_test(mx, my);
            if self.text_state.prompt_purpose() == Some(editor::PromptPurpose::Replace) {
                let half_w = (qw - 32.0) * 0.5;
                let left_hover = matches!(hit, Some(editor::PromptHit::Button));
                let right_hover = matches!(hit, Some(editor::PromptHit::SecondaryButton));
                overlay_rects.push(ui::Rect {
                    x: qx + 12.0,
                    y: qy + qh - 34.0,
                    w: half_w,
                    h: 24.0,
                    color: if left_hover { accent } else { lighten(topbar_bg, 0.10) },
                    alpha: if left_hover { 0.45 } else { 0.85 },
                });
                overlay_rects.push(ui::Rect {
                    x: qx + 20.0 + half_w,
                    y: qy + qh - 34.0,
                    w: half_w,
                    h: 24.0,
                    color: if right_hover { accent } else { lighten(topbar_bg, 0.10) },
                    alpha: if right_hover { 0.45 } else { 0.85 },
                });
            } else {
                let btn_hover = matches!(hit, Some(editor::PromptHit::Button));
                overlay_rects.push(ui::Rect {
                    x: qx + 12.0,
                    y: qy + qh - 34.0,
                    w: qw - 24.0,
                    h: 24.0,
                    color: if btn_hover { accent } else { lighten(topbar_bg, 0.10) },
                    alpha: if btn_hover { 0.45 } else { 0.85 },
                });
            }
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

        // Indicadores del selector de color (van en picker_overlay_rects para dibujarse
        // ENCIMA de sv_quad y hue_quad, no debajo de ellos).
        if self.text_state.picker_open() {
            let hue = self.text_state.picker_hue();
            let sat = self.text_state.picker_sat();
            let val = self.text_state.picker_val();
            let current_rgb = self.text_state.picker_color_rgb();

            let (sx, sy, sw, sh) = self.text_state.picker_sv_rect();
            let cx = (sx + sat * sw).clamp(sx, sx + sw);
            let cy = (sy + (1.0 - val) * sh).clamp(sy, sy + sh);

            picker_overlay_rects.push(ui::Rect { x: cx - 7.0, y: cy - 1.0, w: 14.0, h: 2.0, color: [0.0, 0.0, 0.0], alpha: 0.85 });
            picker_overlay_rects.push(ui::Rect { x: cx - 1.0, y: cy - 7.0, w: 2.0, h: 14.0, color: [0.0, 0.0, 0.0], alpha: 0.85 });
            picker_overlay_rects.push(ui::Rect { x: cx - 5.0, y: cy - 5.0, w: 10.0, h: 10.0, color: [1.0, 1.0, 1.0], alpha: 0.95 });
            picker_overlay_rects.push(ui::Rect { x: cx - 4.0, y: cy - 4.0, w: 8.0, h: 8.0, color: [0.0, 0.0, 0.0], alpha: 0.9 });
            picker_overlay_rects.push(ui::Rect { x: cx - 3.0, y: cy - 3.0, w: 6.0, h: 6.0, color: current_rgb, alpha: 1.0 });

            let (hx, hy, hw, hh) = self.text_state.picker_hue_rect();
            let hx_pos = (hx + (hue / 360.0) * hw).clamp(hx, hx + hw);

            picker_overlay_rects.push(ui::Rect { x: hx_pos - 3.0, y: hy - 2.0, w: 6.0, h: hh + 4.0, color: [0.0, 0.0, 0.0], alpha: 0.9 });
            picker_overlay_rects.push(ui::Rect { x: hx_pos - 2.0, y: hy - 1.0, w: 4.0, h: hh + 2.0, color: [1.0, 1.0, 1.0], alpha: 1.0 });
            let [hr, hg, hb] = crate::color::hsv_to_rgb(hue, 1.0, 1.0);
            picker_overlay_rects.push(ui::Rect {
                x: hx_pos - 1.0,
                y: hy,
                w: 2.0,
                h: hh,
                color: [hr as f32 / 255.0, hg as f32 / 255.0, hb as f32 / 255.0],
                alpha: 1.0,
            });

            // Muestra de vista previa de color (swatch) en la esquina inferior derecha de la tarjeta
            let (px, py, pw, ph) = self.text_state.picker_rect();
            let swatch_x = px + pw - 46.0;
            let swatch_y = py + ph - 36.0;
            picker_overlay_rects.push(ui::Rect { x: swatch_x - 1.0, y: swatch_y - 1.0, w: 34.0, h: 22.0, color: [0.45, 0.48, 0.56], alpha: 1.0 });
            picker_overlay_rects.push(ui::Rect { x: swatch_x, y: swatch_y, w: 32.0, h: 20.0, color: current_rgb, alpha: 1.0 });
        }

        if self.text_state.anim_time < self.text_state.tooltip_expiration {
            let (tx, ty) = self.text_state.tooltip_pos;
            let tw = 400.0;
            let th = 50.0;
            overlay_rects.push(ui::Rect { x: tx - 4.0, y: ty - 4.0, w: tw + 8.0, h: th + 8.0, color: [0.1, 0.1, 0.15], alpha: 0.95 });
            overlay_rects.push(ui::Rect { x: tx - 2.0, y: ty - 2.0, w: tw + 4.0, h: th + 4.0, color: [0.05, 0.05, 0.08], alpha: 0.95 });
        }

        self.ui_pipeline.upload(
            &self.device,
            &self.queue,
            self.size.width,
            self.size.height,
            &ui_rects,
        );

        self.overlay_ui_pipeline.upload(
            &self.device,
            &self.queue,
            self.size.width,
            self.size.height,
            &overlay_rects,
        );

        self.picker_overlay_pipeline.upload(
            &self.device,
            &self.queue,
            self.size.width,
            self.size.height,
            &picker_overlay_rects,
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

            // Capa 0: quad de video/imagen de fondo (si hay uno configurado).
            if let Some(bg) = &self.background {
                pass.set_pipeline(&self.video_pipeline);
                pass.set_bind_group(0, &bg.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }

            // Capa 1: todos los rectángulos base (editor -> topbar -> terminal).
            self.ui_pipeline.draw(&mut pass);

            // Capa 2: texto base (código, números de línea, topbar, pestañas, terminal).
            if let Err(e) = self.text_state.render(&mut pass) {
                error!("no se pudo dibujar el texto: {e:?}");
            }

            // Capa 3: tarjetas flotantes (menús, panel, prompt, autocompletado, fondo picker).
            self.overlay_ui_pipeline.draw(&mut pass);

            // Capa 4: gradientes del selector de color + sus indicadores encima.
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

                // Retícula SV, cursor de Hue y swatch dibujados SOBRE los gradientes.
                self.picker_overlay_pipeline.draw(&mut pass);
            }

            // Capa 5: texto superpuesto (menús, panel, prompt, autocompletado, hex del picker, tooltip).
            if let Err(e) = self.text_state.render_overlay(&mut pass) {
                error!("no se pudo dibujar el texto superpuesto: {e:?}");
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

fn load_window_icon() -> Option<winit::window::Icon> {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn main() -> Result<()> {
    env_logger::init();

    // Argumentos de línea de comandos (CMD / PowerShell / Terminal Linux):
    // Soporta --version, --help, abrir o crear archivos (`amoxlicode archivo.html`),
    // múltiples archivos y banderas `-n` / `--new` / `-c` / `--create`.
    let mut explicit_video: Option<String> = None;
    let mut files_to_open = Vec::new();
    let mut force_create = false;

    for arg in std::env::args().skip(1) {
        let arg_lower = arg.to_lowercase();
        if arg_lower == "--version" || arg_lower == "-v" {
            println!("AmoxliCode {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        } else if arg_lower == "--help" || arg_lower == "-h" {
            println!(
                "AmoxliCode {} - Editor de código acelerado por GPU\n\nUso:\n  amoxlicode [OPCIONES] [ARCHIVOS...] [VIDEO_FONDO.mp4]\n\nOpciones:\n  -n, --new, -c, --create  Crear archivo(s) nuevos sin preguntar\n  -v, --version            Mostrar versión\n  -h, --help               Mostrar esta ayuda",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        } else if arg_lower == "-n" || arg_lower == "--new" || arg_lower == "-c" || arg_lower == "--create" {
            force_create = true;
        } else if arg_lower.ends_with(".mp4")
            || arg_lower.ends_with(".mov")
            || arg_lower.ends_with(".webm")
            || arg_lower.ends_with(".avi")
            || arg_lower.ends_with(".mkv")
        {
            explicit_video = Some(arg);
        } else {
            files_to_open.push(arg);
        }
    }
    if let Some(ref v) = explicit_video {
        info!("Usando video de fondo pasado por CLI: {v}");
    }

    let event_loop = EventLoop::new().context("no se pudo crear el event loop")?;

    let window = Arc::new(
        WindowBuilder::new()
            .with_title("AmoxliCode")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0))
            .with_transparent(true)
            .with_window_icon(load_window_icon())
            .build(&event_loop)
            .context("no se pudo crear la ventana")?,
    );

    let mut state =
        pollster::block_on(RenderState::new(window.clone(), explicit_video.as_deref()))?;

    for file_path in files_to_open {
        state.open_file_path_opts(std::path::PathBuf::from(file_path), force_create);
    }

    state.update_window_title();

    event_loop.run(move |event, elwt| {
        if state.should_exit {
            elwt.exit();
            return;
        }

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
                                if state.save_all_dirty() {
                                    elwt.exit();
                                }
                            }
                            rfd::MessageDialogResult::No => elwt.exit(),
                            _ => {}
                        }
                    } else {
                        elwt.exit();
                    }
                }
                WindowEvent::Resized(new_size) => {
                    state.resize(new_size);
                    window.request_redraw();
                }
                WindowEvent::Focused(false) => {
                    // Al perder el foco (Alt+Tab o diálogo nativo), limpiar modificadores
                    // y estados de arrastre para evitar teclas Ctrl/Shift/Alt pegadas.
                    state.modifiers = ModifiersState::empty();
                    state.mouse_left_down = false;
                    state.text_dragging = false;
                    state.scrollbar_dragging = false;
                    state.text_state.prompt_dragging = false;
                    state.text_state.terminal.is_resizing = false;
                }
                WindowEvent::ModifiersChanged(mods) => {
                    state.modifiers = mods.state();
                }
                WindowEvent::CursorMoved { position, .. } => {
                    state.mouse_pos = (position.x as f32, position.y as f32);
                    let (mx, my) = state.mouse_pos;
                    if state.mouse_left_down {
                        if state.text_state.terminal.is_resizing {
                            let new_h = state.text_state.viewport_height - my;
                            state.text_state.set_terminal_height(new_h);
                            window.request_redraw();
                        } else if state.scrollbar_dragging {
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
                    } else {
                        let cursor = if state.text_state.terminal.is_open
                            && (my - state.text_state.terminal_rect().1).abs() <= 4.0
                        {
                            winit::window::CursorIcon::NsResize
                        } else if !state.text_state.prompt_open()
                            && state.text_state.is_in_text_area(mx, my)
                            && !state.text_state.scrollbar_hit_test(mx, my)
                            && state.text_state.gutter_hit_test(mx, my).is_none()
                        {
                            winit::window::CursorIcon::Text
                        } else {
                            winit::window::CursorIcon::Default
                        };
                        window.set_cursor_icon(cursor);
                    }
                }
                WindowEvent::MouseInput {
                    state: btn_state,
                    button,
                    ..
                } => {
                    if button == MouseButton::Middle && btn_state == ElementState::Pressed {
                        let (mx, my) = state.mouse_pos;
                        if let Some(idx) = state.text_state.tab_any_hit_test(mx, my) {
                            state.close_tab_with_confirm(idx);
                            window.request_redraw();
                        }
                        return;
                    }
                    if button == MouseButton::Left {
                        state.mouse_left_down = btn_state == ElementState::Pressed;
                        if btn_state == ElementState::Released {
                            state.text_dragging = false;
                            state.scrollbar_dragging = false;
                            state.text_state.prompt_dragging = false;
                            state.text_state.terminal.is_resizing = false;
                            state.text_state.finalize_mouse_selection();
                        }
                    }
                    if btn_state == ElementState::Pressed && button == MouseButton::Left {
                        let (mx, my) = state.mouse_pos;
                        let now = std::time::Instant::now();
                        let near_prev = (mx - state.last_click_pos.0).abs() <= 6.0
                            && (my - state.last_click_pos.1).abs() <= 6.0;
                        let within_window = state
                            .last_click_time
                            .map(|prev| now.duration_since(prev) <= std::time::Duration::from_millis(400))
                            .unwrap_or(false);
                        state.click_count = if near_prev && within_window {
                            (state.click_count % 3) + 1
                        } else {
                            1
                        };
                        state.last_click_time = Some(now);
                        state.last_click_pos = (mx, my);

                        if let Some(hit) = state.text_state.terminal_hit_test(mx, my) {
                            state.text_state.terminal_focused = true;
                            match hit {
                                editor::TerminalHit::ResizeHandle => {
                                    state.text_state.terminal.is_resizing = true;
                                }
                                editor::TerminalHit::TabOutput => {
                                    state.text_state.terminal.active_tab = crate::terminal::TerminalTab::Output;
                                    state.text_state.rebuild_terminal_buffer();
                                }
                                editor::TerminalHit::TabShell => {
                                    state.text_state.terminal.active_tab = crate::terminal::TerminalTab::Shell;
                                    state.text_state.rebuild_terminal_buffer();
                                }
                                editor::TerminalHit::BtnRun => {
                                    state.run_code();
                                }
                                editor::TerminalHit::BtnStop => {
                                    state.text_state.terminal.stop_runner();
                                    state.text_state.rebuild_terminal_buffer();
                                }
                                editor::TerminalHit::BtnClear => {
                                    state.text_state.terminal.clear_active();
                                    state.text_state.rebuild_terminal_buffer();
                                }
                                editor::TerminalHit::BtnZoomIn => {
                                    state.text_state.terminal_zoom(0.1);
                                }
                                editor::TerminalHit::BtnZoomOut => {
                                    state.text_state.terminal_zoom(-0.1);
                                }
                                editor::TerminalHit::BtnZoomReset => {
                                    state.text_state.terminal_zoom_reset();
                                }
                                editor::TerminalHit::BtnClose => {
                                    state.text_state.toggle_terminal();
                                }
                                editor::TerminalHit::Scrollbar(ratio) => {
                                    let lines_len = state.text_state.terminal.active_lines().len();
                                    let (_cx, _cy, _cw, ch) = state.text_state.terminal_content_rect();
                                    let visible_lines = (ch / state.text_state.terminal.eff_line_height()).floor() as usize;
                                    let max_scroll = lines_len.saturating_sub(visible_lines);
                                    let target = (max_scroll as f32 * ratio).round() as usize;
                                    state.text_state.terminal.set_active_scroll(target);
                                    state.text_state.rebuild_terminal_buffer();
                                }
                                editor::TerminalHit::Input | editor::TerminalHit::Content => {}
                            }
                            window.request_redraw();
                            return;
                        } else {
                            state.text_state.terminal_focused = false;
                        }
                        if state.text_state.prompt_open() {
                            if let Some(hit) = state.text_state.prompt_hit_test(mx, my) {
                                match hit {
                                    editor::PromptHit::Button => {
                                        match state.text_state.prompt_purpose() {
                                            Some(editor::PromptPurpose::Find) => {
                                                let q = state.text_state.get_prompt_text().to_string();
                                                state.text_state.find_next(&q);
                                            }
                                            Some(editor::PromptPurpose::Replace) => {
                                                state.text_state.replace_current_and_find_next();
                                                state.update_window_title();
                                            }
                                            Some(editor::PromptPurpose::GoToLine) => {
                                                if let Some((_, text)) = state.text_state.prompt_confirm() {
                                                    if let Ok(line_num) = text.trim().parse::<usize>() {
                                                        state.text_state.goto_line(line_num);
                                                    }
                                                }
                                            }
                                            Some(editor::PromptPurpose::SavePreset) => {
                                                if let Some((_, text)) = state.text_state.prompt_confirm() {
                                                    let trimmed = text.trim();
                                                    if !trimmed.is_empty() {
                                                        state.text_state.settings.save_preset(trimmed.to_string());
                                                        state.text_state.settings.save();
                                                        state.text_state.refresh_panel();
                                                    }
                                                }
                                            }
                                            None => {}
                                        }
                                        window.request_redraw();
                                    }
                                    editor::PromptHit::SecondaryButton => {
                                        if state.text_state.prompt_purpose() == Some(editor::PromptPurpose::Replace) {
                                            state.text_state.replace_all();
                                            state.update_window_title();
                                        }
                                        window.request_redraw();
                                    }
                                    editor::PromptHit::Field(f) => {
                                        state.text_state.prompt_set_field(f);
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
                            if let Some(comp_idx) = state.text_state.completion_hit_test(mx, my) {
                                state.text_state.completion_select_and_accept(comp_idx);
                                state.update_window_title();
                            } else if let Some(idx) = state.text_state.tab_close_hit_test(mx, my) {
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
                                state.text_state.close_panel();
                            } else if state.text_state.scrollbar_hit_test(mx, my) {
                                state.scrollbar_dragging = true;
                                state.text_state.scrollbar_scroll_to_y(my);
                            } else if let Some(line) = state.text_state.gutter_hit_test(mx, my) {
                                state.text_state.toggle_bookmark(line);
                            } else if state.text_state.is_in_text_area(mx, my) {
                                state.text_state.hide_completions();
                                let ctrl_only = state.modifiers.control_key() && !state.modifiers.alt_key();
                                if ctrl_only {
                                    state.text_state.click_place_cursor(mx, my);
                                    state.text_state.goto_definition_under_cursor(mx, my);
                                } else if state.modifiers.shift_key() {
                                    state.text_state.shift_click_extend_selection(mx, my);
                                } else if state.click_count == 3 {
                                    state.text_state.select_line_at(mx, my);
                                } else if state.click_count == 2 {
                                    state.text_state.select_word_at(mx, my);
                                } else {
                                    state.text_state.click_place_cursor(mx, my);
                                    state.text_dragging = true;
                                }
                            }
                        }
                        if state.should_exit {
                            elwt.exit();
                            return;
                        }
                        window.request_redraw();
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let (raw_x, raw_y) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (x, y),
                        MouseScrollDelta::PixelDelta(pos) => ((pos.x / 20.0) as f32, (pos.y / 20.0) as f32),
                    };
                    let (mx, my) = state.mouse_pos;
                    if state.text_state.tab_bar_contains(mx, my) {
                        let d = if raw_x.abs() > 0.01 { -raw_x } else { -raw_y };
                        state.text_state.scroll_tabs(d * 48.0);
                        window.request_redraw();
                        return;
                    }
                    if state.text_state.terminal.is_open && state.text_state.terminal_hit_test(mx, my).is_some() {
                        if state.modifiers.control_key() {
                            state.text_state.terminal_zoom(raw_y.signum() * 0.1);
                        } else {
                            state.text_state.terminal_scroll(-(raw_y.signum() as i32) * 3);
                        }
                        window.request_redraw();
                        return;
                    }
                    if state.text_state.panel_open()
                        && state.text_state.panel_contains(state.mouse_pos.0, state.mouse_pos.1)
                    {
                        state.text_state.scroll_panel(-(raw_y.signum() as i32) * 2);
                    } else if state.modifiers.control_key() {
                        state.text_state.zoom(raw_y.signum() * 0.1);
                    } else if state.modifiers.shift_key() || (raw_x.abs() > raw_y.abs() && raw_x.abs() > 0.01) {
                        let horiz = if raw_x.abs() > 0.01 { -raw_x } else { -raw_y };
                        state.text_state.scroll_horizontal(horiz * state.text_state.char_width() * 4.0);
                    } else {
                        state.text_state.scroll(-(raw_y.signum() as i32) * 3);
                    }
                    window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state != ElementState::Pressed {
                        return;
                    }
                    // En Windows, AltGr envía Ctrl+Alt simultáneamente. Solo tratamos
                    // un atajo como Ctrl+<tecla> cuando Alt NO está presionado, para que
                    // teclados en español/latinoamericano escriban @, #, {, }, [, ], \, ~ sin problema.
                    let ctrl_only = state.modifiers.control_key() && !state.modifiers.alt_key();
                    let alt_only = state.modifiers.alt_key() && !state.modifiers.control_key();

                    if state.text_state.terminal_focused {
                        match &event.logical_key {
                            Key::Named(NamedKey::Enter) => {
                                state.text_state.terminal.send_input();
                                state.text_state.rebuild_terminal_buffer();
                            }
                            Key::Named(NamedKey::Backspace) => {
                                if ctrl_only {
                                    state.text_state.terminal.delete_input_word_left();
                                } else {
                                    state.text_state.terminal.input_line.pop();
                                }
                                state.text_state.rebuild_terminal_buffer();
                            }
                            Key::Named(NamedKey::Escape) => {
                                state.text_state.terminal_focused = false;
                            }
                            Key::Named(NamedKey::ArrowUp) => {
                                state.text_state.terminal.history_up();
                                state.text_state.rebuild_terminal_buffer();
                            }
                            Key::Named(NamedKey::ArrowDown) => {
                                state.text_state.terminal.history_down();
                                state.text_state.rebuild_terminal_buffer();
                            }
                            Key::Named(NamedKey::F5) => {
                                if state.modifiers.shift_key() {
                                    state.text_state.terminal.stop_runner();
                                } else {
                                    state.run_code();
                                }
                                state.text_state.rebuild_terminal_buffer();
                            }
                            _ => {
                                if ctrl_only {
                                    if let Key::Character(c) = &event.logical_key {
                                        match c.as_str().to_lowercase().as_str() {
                                            "+" | "=" => state.text_state.terminal_zoom(0.1),
                                            "-" => state.text_state.terminal_zoom(-0.1),
                                            "0" => state.text_state.terminal_zoom_reset(),
                                            "v" => {
                                                if let Ok(mut clip) = arboard::Clipboard::new() {
                                                    if let Ok(text) = clip.get_text() {
                                                        let clean: String = text
                                                            .chars()
                                                            .filter(|&ch| !ch.is_control())
                                                            .collect();
                                                        state.text_state.terminal.input_line.push_str(&clean);
                                                        state.text_state.rebuild_terminal_buffer();
                                                    }
                                                }
                                            }
                                            "c" => {
                                                if state.text_state.terminal.is_running {
                                                    state.text_state.terminal.stop_runner();
                                                    state.text_state.rebuild_terminal_buffer();
                                                }
                                            }
                                            "`" | "~" | "j" | "ñ" => {
                                                state.text_state.toggle_terminal();
                                            }
                                            _ => {}
                                        }
                                    }
                                } else if !alt_only {
                                    if let Some(text) = &event.text {
                                        for c in text.chars() {
                                            if !c.is_control() {
                                                state.text_state.terminal.input_line.push(c);
                                            }
                                        }
                                        state.text_state.rebuild_terminal_buffer();
                                    }
                                }
                            }
                        }
                        window.request_redraw();
                        return;
                    }

                    // Prompt modal (Buscar, Reemplazar, Ir a línea, Guardar preset)
                    if state.text_state.prompt_open() {
                        match &event.logical_key {
                            Key::Named(NamedKey::Tab) => {
                                state.text_state.prompt_switch_field();
                            }
                            Key::Named(NamedKey::Enter) => {
                                if state.text_state.prompt_purpose() == Some(editor::PromptPurpose::Replace) {
                                    if state.modifiers.shift_key() {
                                        state.text_state.replace_all();
                                    } else {
                                        state.text_state.replace_current_and_find_next();
                                    }
                                    state.update_window_title();
                                } else if let Some((purpose, text)) = state.text_state.prompt_confirm() {
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
                                            editor::PromptPurpose::GoToLine => {
                                                if let Ok(line_num) = trimmed.parse::<usize>() {
                                                    state.text_state.goto_line(line_num);
                                                }
                                            }
                                            editor::PromptPurpose::Replace => {}
                                        }
                                    }
                                }
                            }
                            Key::Named(NamedKey::Escape) => {
                                state.text_state.close_prompt();
                            }
                            Key::Named(NamedKey::Backspace) => {
                                if ctrl_only {
                                    state.text_state.prompt_delete_word_left();
                                } else {
                                    state.text_state.prompt_backspace();
                                }
                            }
                            _ => {
                                if alt_only {
                                    if let Key::Character(c) = &event.logical_key {
                                        if c.eq_ignore_ascii_case("a")
                                            && state.text_state.prompt_purpose()
                                                == Some(editor::PromptPurpose::Replace)
                                        {
                                            state.text_state.replace_all();
                                            state.update_window_title();
                                        }
                                    }
                                } else if ctrl_only {
                                    if let Key::Character(c) = &event.logical_key {
                                        if c.eq_ignore_ascii_case("v") {
                                            if let Ok(mut clip) = arboard::Clipboard::new() {
                                                if let Ok(text) = clip.get_text() {
                                                    state.text_state.prompt_push_str(&text);
                                                }
                                            }
                                        }
                                    }
                                } else if let Some(text) = &event.text {
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

                    // Cascada de Escape: menú -> selector de color -> panel -> autocompletado -> selección
                    if event.logical_key == Key::Named(NamedKey::Escape) {
                        if state.text_state.menu_open() {
                            state.text_state.close_menu();
                        } else if state.text_state.picker_open() {
                            state.text_state.close_picker();
                        } else if state.text_state.panel_open() {
                            state.text_state.close_panel();
                        } else if state.text_state.completion_visible() {
                            state.text_state.hide_completions();
                        } else {
                            state.text_state.clear_selection();
                        }
                        window.request_redraw();
                        return;
                    }

                    if event.logical_key == Key::Named(NamedKey::F5) {
                        if state.modifiers.shift_key() {
                            state.text_state.terminal.stop_runner();
                            state.text_state.rebuild_terminal_buffer();
                        } else {
                            state.run_code();
                        }
                        window.request_redraw();
                        return;
                    }

                    if event.logical_key == Key::Named(NamedKey::F8) {
                        state.text_state.toggle_rgb_gamer_mode();
                        window.request_redraw();
                        return;
                    }

                    // Atajos Ctrl+<tecla> (excluyendo AltGr = Ctrl+Alt)
                    if ctrl_only {
                        if let Key::Character(c) = &event.logical_key {
                            match c.as_str().to_lowercase().as_str() {
                                "`" | "~" | "j" | "ñ" => {
                                    state.text_state.toggle_terminal();
                                    window.request_redraw();
                                    return;
                                }
                                "s" => {
                                    if state.modifiers.shift_key() {
                                        state.save_file_as();
                                    } else {
                                        state.save_file();
                                    }
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
                                    state.text_state.cycle_theme();
                                    window.request_redraw();
                                    return;
                                }
                                "b" => {
                                    if state.modifiers.shift_key() {
                                        state.clear_background();
                                    } else {
                                        state.choose_background();
                                    }
                                    window.request_redraw();
                                    return;
                                }
                                "," => {
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
                                        state.update_window_title();
                                    }
                                    window.request_redraw();
                                    return;
                                }
                                "v" => {
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        if let Ok(text) = clipboard.get_text() {
                                            state.text_state.paste_text(&text);
                                            state.play_sound("paste");
                                            state.update_window_title();
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
                                    state.update_window_title();
                                    window.request_redraw();
                                    return;
                                }
                                "y" => {
                                    state.text_state.redo();
                                    state.play_sound("redo");
                                    state.update_window_title();
                                    window.request_redraw();
                                    return;
                                }
                                "d" => {
                                    state.text_state.duplicate_line();
                                    state.update_window_title();
                                    window.request_redraw();
                                    return;
                                }
                                "/" => {
                                    state.text_state.toggle_line_comment();
                                    state.update_window_title();
                                    window.request_redraw();
                                    return;
                                }
                                "g" => {
                                    state
                                        .text_state
                                        .open_prompt(editor::PromptPurpose::GoToLine, "Ir a línea:");
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
                                "h" => {
                                    state.text_state.open_prompt(
                                        editor::PromptPurpose::Replace,
                                        "Buscar y reemplazar:",
                                    );
                                    window.request_redraw();
                                    return;
                                }
                                digit if digit.len() == 1
                                    && digit.chars().next().unwrap().is_ascii_digit()
                                    && digit != "0" =>
                                {
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

                    // Ctrl+Tab / Ctrl+Shift+Tab: cambiar de pestaña
                    if ctrl_only && event.logical_key == Key::Named(NamedKey::Tab) {
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

                    let mut consumed_by_popup = false;
                    if state.text_state.completion_visible() {
                        let plain_nav = !state.modifiers.shift_key() && !ctrl_only && !alt_only;
                        consumed_by_popup = true;
                        match &event.logical_key {
                            Key::Named(NamedKey::ArrowDown) if plain_nav => {
                                state.text_state.completion_move_selection(1);
                            }
                            Key::Named(NamedKey::ArrowUp) if plain_nav => {
                                state.text_state.completion_move_selection(-1);
                            }
                            Key::Named(NamedKey::Tab) | Key::Named(NamedKey::Enter) if plain_nav => {
                                state.text_state.completion_accept();
                                state.update_window_title();
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
                                state.update_window_title();
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
                                state.update_window_title();
                            }
                            Key::Named(NamedKey::Backspace) => {
                                if ctrl_only {
                                    state.text_state.delete_word_left();
                                } else {
                                    state.text_state.backspace();
                                }
                                state.play_sound("keypress");
                                state.update_window_title();
                            }
                            Key::Named(NamedKey::Delete) => {
                                if ctrl_only {
                                    state.text_state.delete_word_right();
                                } else {
                                    state.text_state.delete_forward();
                                }
                                state.play_sound("keypress");
                                state.update_window_title();
                            }
                            Key::Named(NamedKey::ArrowLeft) => {
                                if ctrl_only {
                                    state.text_state.move_cursor_by_word(-1, state.modifiers.shift_key());
                                } else {
                                    state.text_state.move_cursor(-1, 0, state.modifiers.shift_key());
                                }
                            }
                            Key::Named(NamedKey::ArrowRight) => {
                                if ctrl_only {
                                    state.text_state.move_cursor_by_word(1, state.modifiers.shift_key());
                                } else {
                                    state.text_state.move_cursor(1, 0, state.modifiers.shift_key());
                                }
                            }
                            Key::Named(NamedKey::ArrowUp) => {
                                if alt_only {
                                    state.text_state.move_line(-1);
                                    state.update_window_title();
                                } else if ctrl_only {
                                    state.text_state.move_cursor_paragraph(-1, state.modifiers.shift_key());
                                } else {
                                    state.text_state.move_cursor(0, -1, state.modifiers.shift_key());
                                }
                            }
                            Key::Named(NamedKey::ArrowDown) => {
                                if alt_only {
                                    state.text_state.move_line(1);
                                    state.update_window_title();
                                } else if ctrl_only {
                                    state.text_state.move_cursor_paragraph(1, state.modifiers.shift_key());
                                } else {
                                    state.text_state.move_cursor(0, 1, state.modifiers.shift_key());
                                }
                            }
                            Key::Named(NamedKey::PageUp) => {
                                state.text_state.move_cursor_page(-1, state.modifiers.shift_key());
                            }
                            Key::Named(NamedKey::PageDown) => {
                                state.text_state.move_cursor_page(1, state.modifiers.shift_key());
                            }
                            Key::Named(NamedKey::Home) => {
                                state.text_state.move_cursor_home(ctrl_only, state.modifiers.shift_key());
                            }
                            Key::Named(NamedKey::End) => {
                                state.text_state.move_cursor_end(ctrl_only, state.modifiers.shift_key());
                            }
                            _ => {
                                // Caracteres normales: nunca colar bytes de control ni atajos Ctrl+tecla.
                                // AltGr (Ctrl+Alt) SÍ pasa por aquí y escribe @ # { } [ ] \ ~ normalmente.
                                if !ctrl_only && !alt_only {
                                    if let Some(text) = &event.text {
                                        let mut inserted_any = false;
                                        let mut should_trigger = false;
                                        for c in text.chars() {
                                            if c.is_control() {
                                                continue;
                                            }
                                            state.text_state.insert_char(c);
                                            inserted_any = true;
                                            if c == '.' || c.is_alphanumeric() {
                                                should_trigger = true;
                                            }
                                        }
                                        if inserted_any {
                                            state.play_sound("keypress");
                                            state.update_window_title();
                                        }
                                        if should_trigger {
                                            state.request_completion();
                                        }
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
            Event::AboutToWait => {
                if state.should_exit {
                    elwt.exit();
                    return;
                }
                // Pacing inteligente: 60fps cuando hay video, RGB, runner o animaciones;
                // reposo eficiente (~50-200ms) cuando el editor está quieto para no quemar CPU.
                let interval = if state.background.is_some()
                    || state.text_state.settings.rgb_gamer_mode
                    || state.text_state.terminal.is_running
                    || state.text_state.has_active_animation()
                {
                    std::time::Duration::from_millis(16)
                } else if state.text_state.terminal.is_open {
                    std::time::Duration::from_millis(60)
                } else {
                    std::time::Duration::from_millis(200)
                };
                elwt.set_control_flow(ControlFlow::WaitUntil(std::time::Instant::now() + interval));
                window.request_redraw();
            }
            _ => {}
        }
    })?;

    Ok(())
}
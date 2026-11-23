//! AmoxliCode - Fase 2: Motor de video (Capa 0 / fondo)
//!
//! Decodifica un archivo de video o imagen en un hilo aparte y envía frames
//! RGBA listos para subir a una textura de wgpu, respetando el framerate real
//! del video (sin acelerar videos de 24/30 FPS en monitores de 60/144 Hz).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};

/// Un frame de video ya convertido a RGBA8 "empaquetado" (sin padding de
/// stride), listo para `queue.write_texture`.
pub struct VideoFrame {
    pub data: Vec<u8>,
}

/// Abre el archivo indicado (video O imagen estática) y devuelve el receptor
/// del canal junto con el ancho/alto.
pub fn spawn(path: impl AsRef<Path>) -> Result<(Receiver<VideoFrame>, u32, u32)> {
    let path = path.as_ref();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "tga" | "webp" => spawn_image(path),
        _ => spawn_video(path),
    }
}

/// Carga una imagen estática una sola vez.
fn spawn_image(path: &Path) -> Result<(Receiver<VideoFrame>, u32, u32)> {
    let img = image::open(path)
        .context("no se pudo abrir la imagen")?
        .into_rgba8();
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 {
        bail!("la imagen tiene dimensiones inválidas ({width}x{height})");
    }

    let (tx, rx) = bounded::<VideoFrame>(1);
    let _ = tx.send(VideoFrame {
        data: img.into_raw(),
    });

    Ok((rx, width, height))
}

fn spawn_video(path: impl AsRef<Path>) -> Result<(Receiver<VideoFrame>, u32, u32)> {
    ffmpeg_next::init().context("no se pudo inicializar ffmpeg")?;

    let path: PathBuf = path.as_ref().to_path_buf();
    let (width, height) = probe_dimensions(&path)?;
    if width == 0 || height == 0 {
        bail!("el video tiene dimensiones inválidas ({width}x{height})");
    }

    let (tx, rx) = bounded::<VideoFrame>(2);

    let decode_path = path.clone();
    std::thread::spawn(move || {
        if let Err(e) = decode_loop(&decode_path, tx) {
            log::error!("el hilo de video terminó con un error: {e:?}");
        }
    });

    Ok((rx, width, height))
}

fn probe_dimensions(path: &Path) -> Result<(u32, u32)> {
    let ictx = ffmpeg_next::format::input(path).context("no se pudo abrir el archivo de video")?;
    let stream = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .context("el archivo no contiene una pista de video")?;

    let context_decoder =
        ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())
            .context("no se pudo leer los parámetros del códec")?;
    let decoder = context_decoder
        .decoder()
        .video()
        .context("no se pudo abrir el decodificador de video")?;

    Ok((decoder.width(), decoder.height()))
}

fn stream_frame_duration(stream: &ffmpeg_next::Stream) -> Duration {
    let rate = stream.avg_frame_rate();
    let fps = if rate.denominator() > 0 && rate.numerator() > 0 {
        rate.numerator() as f64 / rate.denominator() as f64
    } else {
        let r = stream.rate();
        if r.denominator() > 0 && r.numerator() > 0 {
            r.numerator() as f64 / r.denominator() as f64
        } else {
            30.0
        }
    };
    let clamped = fps.clamp(5.0, 120.0);
    Duration::from_secs_f64(1.0 / clamped)
}

fn emit_decoded_frames(
    decoder: &mut ffmpeg_next::decoder::Video,
    scaler: &mut ffmpeg_next::software::scaling::Context,
    tx: &Sender<VideoFrame>,
    frame_duration: Duration,
    next_deadline: &mut Instant,
) -> Result<bool> {
    let mut decoded = ffmpeg_next::util::frame::video::Video::empty();
    while decoder.receive_frame(&mut decoded).is_ok() {
        let mut rgba_frame = ffmpeg_next::util::frame::video::Video::empty();
        if let Err(e) = scaler.run(&decoded, &mut rgba_frame) {
            log::warn!("frame omitido al convertir a RGBA: {e:?}");
            continue;
        }

        let width = rgba_frame.width() as usize;
        let height = rgba_frame.height() as usize;
        let stride = rgba_frame.stride(0);
        let src = rgba_frame.data(0);
        let row_bytes = width * 4;
        if stride < row_bytes || src.len() < height.saturating_sub(1) * stride + row_bytes {
            continue;
        }

        let mut packed = Vec::with_capacity(row_bytes * height);
        for row in 0..height {
            let start = row * stride;
            let end = start + row_bytes;
            packed.extend_from_slice(&src[start..end]);
        }

        // Ritmo real del video: evita reproducir a 2x-5x en pantallas de 60/144Hz.
        let now = Instant::now();
        if *next_deadline > now {
            std::thread::sleep(*next_deadline - now);
        }
        *next_deadline = Instant::now() + frame_duration;

        if tx.send(VideoFrame { data: packed }).is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Ciclo principal del hilo de decodificación con loop infinito y drenado EOF.
fn decode_loop(path: &Path, tx: Sender<VideoFrame>) -> Result<()> {
    let mut next_deadline = Instant::now();

    loop {
        let mut ictx = ffmpeg_next::format::input(path).context("no se pudo reabrir el video")?;
        let input = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .context("sin pista de video")?;
        let video_stream_index = input.index();
        let frame_duration = stream_frame_duration(&input);

        let context_decoder =
            ffmpeg_next::codec::context::Context::from_parameters(input.parameters())?;
        let mut decoder = context_decoder.decoder().video()?;

        let mut scaler = ffmpeg_next::software::scaling::context::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ffmpeg_next::format::Pixel::RGBA,
            decoder.width(),
            decoder.height(),
            ffmpeg_next::software::scaling::flag::Flags::BILINEAR,
        )
        .context("no se pudo crear el conversor de color (scaler)")?;

        for (stream, packet) in ictx.packets() {
            if stream.index() != video_stream_index {
                continue;
            }

            // Si un paquete está dañado, lo registramos y seguimos en vez de matar el hilo.
            if let Err(e) = decoder.send_packet(&packet) {
                log::warn!("paquete de video omitido: {e:?}");
                continue;
            }

            if !emit_decoded_frames(&mut decoder, &mut scaler, &tx, frame_duration, &mut next_deadline)? {
                return Ok(());
            }
        }

        // Drenar los últimos B-frames acumulados antes de reiniciar el loop.
        if decoder.send_eof().is_ok()
            && !emit_decoded_frames(&mut decoder, &mut scaler, &tx, frame_duration, &mut next_deadline)?
        {
            return Ok(());
        }
    }
}

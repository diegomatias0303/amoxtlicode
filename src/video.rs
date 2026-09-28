//! AmoxliCode - Fase 2: Motor de video (Capa 0 / fondo)
//!
//! Decodifica un archivo de video en un hilo aparte y envía frames RGBA
//! listos para subir a una textura de wgpu, sin bloquear el hilo principal
//! de renderizado.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};

/// Un frame de video ya convertido a RGBA8 "empaquetado" (sin padding de
/// stride), listo para `queue.write_texture`.
pub struct VideoFrame {
    pub data: Vec<u8>,
}

/// Abre el archivo indicado (video O imagen estática) y devuelve el
/// receptor del canal junto con el ancho/alto (necesarios para crear la
/// textura de wgpu con el tamaño correcto). Detecta cuál es según la
/// extensión del archivo.
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

/// Carga una imagen estática una sola vez (no hace falta un hilo de
/// decodificación continua: el "video" de fondo, en este caso, es
/// literalmente un solo frame fijo).
fn spawn_image(path: &Path) -> Result<(Receiver<VideoFrame>, u32, u32)> {
    let img = image::open(path)
        .context("no se pudo abrir la imagen")?
        .into_rgba8();
    let (width, height) = img.dimensions();

    let (tx, rx) = bounded::<VideoFrame>(1);
    let _ = tx.send(VideoFrame {
        data: img.into_raw(),
    });
    // No hace falta guardar `tx` en un hilo: el único frame ya quedó
    // encolado en el canal, y `rx` lo puede leer en cualquier momento.

    Ok((rx, width, height))
}

fn spawn_video(path: impl AsRef<Path>) -> Result<(Receiver<VideoFrame>, u32, u32)> {
    ffmpeg_next::init().context("no se pudo inicializar ffmpeg")?;

    let path: PathBuf = path.as_ref().to_path_buf();

    // Abrimos una vez, solo para leer las dimensiones del video antes de
    // lanzar el hilo (así el llamador puede crear la textura del tamaño
    // correcto de inmediato).
    let (width, height) = probe_dimensions(&path)?;

    // Canal con capacidad pequeña: si el hilo principal se atrasa, el
    // decodificador se bloquea al enviar en vez de acumular frames en
    // memoria indefinidamente (esto evita el problema de "backpressure"
    // sin límite).
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

/// Ciclo principal del hilo de decodificación. Cuando el video termina, lo
/// vuelve a abrir desde el inicio (loop infinito), tal como se espera de
/// un fondo animado continuo.
fn decode_loop(path: &Path, tx: Sender<VideoFrame>) -> Result<()> {
    loop {
        let mut ictx = ffmpeg_next::format::input(path).context("no se pudo reabrir el video")?;
        let input = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .context("sin pista de video")?;
        let video_stream_index = input.index();

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

            decoder.send_packet(&packet)?;

            let mut decoded = ffmpeg_next::util::frame::video::Video::empty();
            while decoder.receive_frame(&mut decoded).is_ok() {
                let mut rgba_frame = ffmpeg_next::util::frame::video::Video::empty();
                scaler
                    .run(&decoded, &mut rgba_frame)
                    .context("fallo al convertir el frame a RGBA")?;

                let width = rgba_frame.width() as usize;
                let height = rgba_frame.height() as usize;
                let stride = rgba_frame.stride(0);
                let src = rgba_frame.data(0);

                // FFmpeg puede alinear cada fila con relleno extra
                // (stride > width * 4 bytes). Copiamos fila por fila para
                // obtener un buffer "empaquetado" sin ese relleno, que es
                // lo que `write_texture` espera.
                let mut packed = Vec::with_capacity(width * height * 4);
                for row in 0..height {
                    let start = row * stride;
                    let end = start + width * 4;
                    packed.extend_from_slice(&src[start..end]);
                }

                if tx.send(VideoFrame { data: packed }).is_err() {
                    // El receptor se cerró (la ventana se cerró): terminamos
                    // el hilo con normalidad.
                    return Ok(());
                }
            }
        }
        // Fin del archivo: el `loop` externo lo vuelve a abrir desde cero.
    }
}
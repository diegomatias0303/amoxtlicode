//! AmoxliCode - motor de sonido para los atajos de teclado.
//!
//! Guarda en memoria los bytes de cada sonido asignado en un `Arc<Vec<u8>>`
//! (cargados una sola vez y compartidos sin clonar al reproducir). Si no hay
//! dispositivo de audio disponible, `AudioEngine::new()` devuelve `None` y la
//! aplicación continúa funcionando con normalidad.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use rodio::{OutputStream, OutputStreamHandle, Source};

/// Envoltorio zero-copy para pasar un `Arc<Vec<u8>>` a `std::io::Cursor` sin
/// clonar el buffer de audio en cada pulsación de tecla.
#[derive(Clone)]
struct SharedBytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

pub struct AudioEngine {
    // Debe mantenerse viva mientras el programa corra, o se corta el audio.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    cache: HashMap<String, Arc<Vec<u8>>>,
}

impl AudioEngine {
    /// Intenta abrir el dispositivo de audio por defecto.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Self {
            _stream: stream,
            handle,
            cache: HashMap::new(),
        })
    }

    /// Carga (o reemplaza) el sonido asignado a una acción desde disco una sola vez.
    pub fn load(&mut self, action: &str, path: &Path) -> anyhow::Result<()> {
        let bytes = std::fs::read(path)?;
        self.cache.insert(action.to_string(), Arc::new(bytes));
        Ok(())
    }

    #[allow(dead_code)]
    pub fn unload(&mut self, action: &str) {
        self.cache.remove(action);
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Reproduce el sonido de una acción sin clonar el buffer de bytes ni
    /// bloquear el hilo principal.
    pub fn play(&self, action: &str) {
        let Some(bytes) = self.cache.get(action) else {
            return;
        };
        let cursor = Cursor::new(SharedBytes(Arc::clone(bytes)));
        if let Ok(source) = rodio::Decoder::new(cursor) {
            let _ = self.handle.play_raw(source.convert_samples());
        }
    }
}

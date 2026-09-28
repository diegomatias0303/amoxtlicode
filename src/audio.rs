//! AmoxliCode - motor de sonido para los atajos de teclado.
//!
//! Guarda en memoria los bytes de cada sonido asignado (cargados una sola
//! vez, no en cada tecla) y los reproduce bajo demanda. Si no hay
//! dispositivo de audio disponible, todo el motor simplemente no existe
//! (`AudioEngine::new()` devuelve `None`) y el programa sigue funcionando
//! normal, solo sin sonidos.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use rodio::{OutputStream, OutputStreamHandle, Source};

pub struct AudioEngine {
    // Debe mantenerse viva mientras el programa corra, o se corta el
    // audio (aunque no la usemos directamente después de crearla).
    _stream: OutputStream,
    handle: OutputStreamHandle,
    cache: HashMap<String, Vec<u8>>,
}

impl AudioEngine {
    /// Intenta abrir el dispositivo de audio por defecto. Si no hay
    /// ninguno disponible (o falla por cualquier razón), devuelve `None`
    /// en vez de tronar la app entera por algo tan no-esencial como el
    /// sonido.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Self {
            _stream: stream,
            handle,
            cache: HashMap::new(),
        })
    }

    /// Carga (o reemplaza) el sonido asignado a una acción. Se lee del
    /// disco una sola vez aquí, no cada vez que se reproduce.
    pub fn load(&mut self, action: &str, path: &Path) -> anyhow::Result<()> {
        let bytes = std::fs::read(path)?;
        self.cache.insert(action.to_string(), bytes);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn unload(&mut self, action: &str) {
        self.cache.remove(action);
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Reproduce el sonido de una acción, si tiene uno asignado. No
    /// bloquea: el audio se reproduce en un hilo interno de `rodio`.
    pub fn play(&self, action: &str) {
        let Some(bytes) = self.cache.get(action) else {
            return;
        };
        // Clonamos los bytes porque el decodificador necesita algo que
        // implemente Read + Seek "dueño" de los datos.
        let cursor = Cursor::new(bytes.clone());
        if let Ok(source) = rodio::Decoder::new(cursor) {
            let _ = self.handle.play_raw(source.convert_samples());
        }
    }
}

//! AmoxliCode - Terminal integrada y motor de ejecución (Output + Shell)
//!
//! Arreglos clave:
//! - Sin deadlock al detener procesos (mata por PID directamente sin bloquear el mutex de `wait()`).
//! - Sin procesos zombi de `cmd.exe`/`bash` (implementa `Drop` para cerrar el runner y la shell).
//! - Decodificación UTF-8 segura entre chunks de lectura y limpieza de secuencias ANSI (CSI/OSC).
//! - Soporte UTF-8 en Windows (`chcp 65001`) para acentos y ñ sin mojibake.
//! - Detección de compiladores/intérpretes cacheada (`OnceLock`) para evitar bloqueos al presionar F5.
//! - Scroll seguro sin overflow `usize::MAX as i32`.

use crossbeam_channel::{unbounded, Receiver, Sender};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Instant;

static C_COMPILER_CACHE: OnceLock<Option<String>> = OnceLock::new();
static CPP_COMPILER_CACHE: OnceLock<Option<String>> = OnceLock::new();
static PYTHON_CACHE: OnceLock<Option<String>> = OnceLock::new();

/// Pestañas disponibles en el panel inferior integrado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalTab {
    /// Salida directa de compilación/ejecución (estilo NetBeans / Visual Studio)
    Output,
    /// Terminal interactiva de comandos (CMD / Bash)
    Shell,
}

/// Eventos recibidos de la ejecución de un programa en segundo plano.
#[allow(dead_code)]
pub enum RunEvent {
    Stdout(String),
    Stderr(String),
    Finished { code: i32, duration_secs: f32 },
    Stopped,
    Error(String),
}

/// Decodifica un flujo arbitrario de bytes a UTF-8 sin romper caracteres
/// multibyte que queden partidos entre llamadas a `read()`.
struct Utf8ChunkDecoder {
    pending: Vec<u8>,
}

impl Utf8ChunkDecoder {
    fn new() -> Self {
        Self { pending: Vec::new() }
    }

    fn push(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    out.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(e) => {
                    let valid_len = e.valid_up_to();
                    if valid_len > 0 {
                        out.push_str(&String::from_utf8_lossy(&self.pending[..valid_len]));
                        self.pending.drain(..valid_len);
                    }
                    match e.error_len() {
                        Some(invalid_len) => {
                            out.push('\u{FFFD}');
                            self.pending.drain(..invalid_len);
                        }
                        None => {
                            // Secuencia UTF-8 incompleta al final del chunk: esperar siguiente lectura.
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    fn flush(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let s = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        s
    }
}

/// Limpia secuencias de escape ANSI (colores CSI `\x1b[...m` y títulos OSC `\x1b]...`).
pub fn strip_ansi_codes(input: &str) -> String {
    if !input.contains('\x1b') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    // Secuencia CSI: termina en byte 0x40..=0x7E (`@`..`~`)
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    // Secuencia OSC: termina en BEL (\x07) o ST (\x1b\\)
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Mata un árbol de procesos por PID sin bloquear ningún mutex de `Child::wait()`.
fn kill_process_tree(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("pkill")
            .args(["-KILL", "-P", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Manejador de un proceso en ejecución para la pestaña de Salida (Run).
#[allow(dead_code)]
pub struct RunnerProcess {
    pub pid: u32,
    pub stopped_by_user: Arc<AtomicBool>,
    pub tx_stdin: Sender<String>,
    pub rx_events: Receiver<RunEvent>,
    pub is_running: bool,
    pub command: String,
    pub start_time: Instant,
}

impl RunnerProcess {
    pub fn stop(&mut self) {
        if !self.is_running {
            return;
        }
        self.is_running = false;
        self.stopped_by_user.store(true, Ordering::SeqCst);
        kill_process_tree(self.pid);
    }
}

/// Estado global del panel dock inferior (Salida + Terminal interactiva).
pub struct TerminalState {
    pub is_open: bool,
    pub height: f32,
    pub active_tab: TerminalTab,

    // --- Pestaña: Salida (NetBeans / VS Studio Runner) ---
    pub output_lines: Vec<String>,
    pub output_scroll: usize,
    pub auto_scroll_output: bool,
    pub runner: Option<RunnerProcess>,
    pub is_running: bool,
    pub last_exit_code: Option<i32>,
    pub last_duration_secs: Option<f32>,
    pub current_running_file: Option<String>,

    // --- Pestaña: Terminal interactiva (Shell) ---
    pub shell_lines: Vec<String>,
    pub shell_scroll: usize,
    pub auto_scroll_shell: bool,
    shell_child: Option<Child>,
    shell_tx_stdin: Option<Sender<String>>,
    shell_rx_stdout: Option<Receiver<String>>,

    // --- Barra de entrada (Input Prompt) ---
    pub input_line: String,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,

    // --- Estado de interacción UI ---
    pub is_resizing: bool,
    pub font_scale: f32,
}

impl Drop for TerminalState {
    fn drop(&mut self) {
        self.stop_runner();
        self.shell_tx_stdin = None;
        if let Some(mut child) = self.shell_child.take() {
            let pid = child.id();
            kill_process_tree(pid);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl TerminalState {
    pub const DEFAULT_HEIGHT: f32 = 240.0;
    pub const MIN_HEIGHT: f32 = 120.0;

    pub fn new() -> Self {
        let mut state = Self {
            is_open: false,
            height: Self::DEFAULT_HEIGHT,
            active_tab: TerminalTab::Output,

            output_lines: vec![
                "================================================================================".to_string(),
                " AmoxliCode Output Engine v2.1 - [Modo NetBeans / Visual Studio]".to_string(),
                " Presiona F5 o haz clic en [▶ Ejecutar] para compilar y correr el archivo activo.".to_string(),
                "================================================================================".to_string(),
            ],
            output_scroll: 0,
            auto_scroll_output: true,
            runner: None,
            is_running: false,
            last_exit_code: None,
            last_duration_secs: None,
            current_running_file: None,

            shell_lines: Vec::new(),
            shell_scroll: 0,
            auto_scroll_shell: true,
            shell_child: None,
            shell_tx_stdin: None,
            shell_rx_stdout: None,

            input_line: String::new(),
            history: Vec::new(),
            history_idx: None,

            is_resizing: false,
            font_scale: 1.0,
        };

        state.init_shell();
        state
    }

    pub fn zoom(&mut self, delta: f32) {
        self.font_scale = (self.font_scale + delta).clamp(0.6, 2.5);
    }

    pub fn zoom_reset(&mut self) {
        self.font_scale = 1.0;
    }

    pub fn eff_font_size(&self) -> f32 {
        13.0 * self.font_scale
    }

    pub fn eff_line_height(&self) -> f32 {
        18.0 * self.font_scale
    }

    /// Inicializa la shell interactiva en segundo plano (con UTF-8 en Windows).
    pub fn init_shell(&mut self) {
        let (tx_out, rx_stdout) = unbounded();
        let (tx_stdin, rx_in) = unbounded::<String>();

        #[cfg(target_os = "windows")]
        let mut cmd = {
            use std::os::windows::process::CommandExt;
            let mut c = Command::new("cmd.exe");
            c.args(["/K", "chcp 65001 >nul"]);
            c.creation_flags(0x0800_0000);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = Command::new("bash");

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.shell_lines.push(format!("Error al iniciar shell: {e}"));
                return;
            }
        };

        let mut stdout = match child.stdout.take() {
            Some(s) => s,
            None => return,
        };
        let mut stderr = match child.stderr.take() {
            Some(s) => s,
            None => return,
        };
        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => return,
        };

        self.shell_child = Some(child);

        let tx_out_clone = tx_out.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut dec = Utf8ChunkDecoder::new();
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let s = dec.push(&buf[..n]);
                if !s.is_empty() {
                    let _ = tx_out_clone.send(s);
                }
            }
            let rem = dec.flush();
            if !rem.is_empty() {
                let _ = tx_out_clone.send(rem);
            }
        });

        let tx_out_stderr = tx_out.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut dec = Utf8ChunkDecoder::new();
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let s = dec.push(&buf[..n]);
                if !s.is_empty() {
                    let _ = tx_out_stderr.send(s);
                }
            }
            let rem = dec.flush();
            if !rem.is_empty() {
                let _ = tx_out_stderr.send(rem);
            }
        });

        thread::spawn(move || {
            while let Ok(mut cmd_str) = rx_in.recv() {
                cmd_str.push('\n');
                if stdin.write_all(cmd_str.as_bytes()).is_err() {
                    break;
                }
                if stdin.flush().is_err() {
                    break;
                }
            }
        });

        self.shell_tx_stdin = Some(tx_stdin);
        self.shell_rx_stdout = Some(rx_stdout);
        self.shell_lines
            .push("Terminal interactiva lista. Escribe comandos abajo.".to_string());
    }

    /// Ejecuta el archivo actual con el compilador/intérprete apropiado.
    pub fn start_run(&mut self, file_path: &Path, lang: crate::syntax::Language) {
        self.stop_runner();

        let parent_dir = file_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| Path::new(".").to_path_buf());

        let file_name = file_path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archivo".to_string());

        let stem = file_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "programa".to_string());

        let command_str = Self::build_command_for_file(&file_name, &stem, file_path, lang);

        let separator = "================================================================================".to_string();
        self.output_lines.push(separator.clone());
        self.output_lines.push(format!("[EJECUTANDO]: {command_str}"));
        self.output_lines.push(format!("[DIRECTORIO]: {}", parent_dir.display()));
        self.output_lines.push(separator);
        self.output_lines.push(String::new());

        let (tx_events, rx_events) = unbounded::<RunEvent>();
        let (tx_stdin, rx_stdin) = unbounded::<String>();

        #[cfg(target_os = "windows")]
        let mut cmd = {
            use std::os::windows::process::CommandExt;
            let mut c = Command::new("cmd");
            c.raw_arg(format!("/C chcp 65001 >nul & {command_str}"));
            c.creation_flags(0x0800_0000);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&command_str);
            c
        };

        cmd.current_dir(&parent_dir);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.env("PYTHONIOENCODING", "utf-8");

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.output_lines.push(format!("[ERROR]: Error al iniciar ejecución: {e}"));
                self.output_lines.push("--------------------------------------------------------------------------------".to_string());
                self.scroll_output_to_bottom();
                return;
            }
        };

        let pid = child.id();
        let Some(mut stdout) = child.stdout.take() else { return; };
        let Some(mut stderr) = child.stderr.take() else { return; };
        let Some(mut stdin) = child.stdin.take() else { return; };

        let stopped_by_user = Arc::new(AtomicBool::new(false));

        // Hilo lector de stdout
        let tx_out = tx_events.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut dec = Utf8ChunkDecoder::new();
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let s = dec.push(&buf[..n]);
                if !s.is_empty() {
                    let _ = tx_out.send(RunEvent::Stdout(s));
                }
            }
            let rem = dec.flush();
            if !rem.is_empty() {
                let _ = tx_out.send(RunEvent::Stdout(rem));
            }
        });

        // Hilo lector de stderr
        let tx_err = tx_events.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut dec = Utf8ChunkDecoder::new();
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let s = dec.push(&buf[..n]);
                if !s.is_empty() {
                    let _ = tx_err.send(RunEvent::Stderr(s));
                }
            }
            let rem = dec.flush();
            if !rem.is_empty() {
                let _ = tx_err.send(RunEvent::Stderr(rem));
            }
        });

        // Hilo escritor de stdin
        thread::spawn(move || {
            while let Ok(mut line) = rx_stdin.recv() {
                line.push('\n');
                if stdin.write_all(line.as_bytes()).is_err() {
                    break;
                }
                if stdin.flush().is_err() {
                    break;
                }
            }
        });

        // Hilo de espera: es dueño exclusivo de `child`, sin Mutex -> CERO deadlocks en stop()!
        let tx_finished = tx_events.clone();
        let stopped_flag = Arc::clone(&stopped_by_user);
        let start_time = Instant::now();
        thread::spawn(move || {
            let status = child.wait().ok();
            if stopped_flag.load(Ordering::SeqCst) {
                return;
            }
            let duration_secs = start_time.elapsed().as_secs_f32();
            let code = status.and_then(|s| s.code()).unwrap_or(-1);
            let _ = tx_finished.send(RunEvent::Finished {
                code,
                duration_secs,
            });
        });

        self.runner = Some(RunnerProcess {
            pid,
            stopped_by_user,
            tx_stdin,
            rx_events,
            is_running: true,
            command: command_str,
            start_time,
        });

        self.is_running = true;
        self.current_running_file = Some(file_name);
        self.active_tab = TerminalTab::Output;
        self.is_open = true;
        self.scroll_output_to_bottom();
    }

    /// Detiene el proceso actual inmediatamente sin bloquear la UI.
    pub fn stop_runner(&mut self) {
        if let Some(ref mut runner) = self.runner {
            if runner.is_running {
                runner.stop();
                self.output_lines.push("--------------------------------------------------------------------------------".to_string());
                self.output_lines.push("[PROCESO DETENIDO POR EL USUARIO]".to_string());
                self.output_lines.push("================================================================================".to_string());
                self.scroll_output_to_bottom();
            }
        }
        self.runner = None;
        self.is_running = false;
    }

    fn is_executable_available(cmd: &str) -> bool {
        let mut check = Command::new(cmd);
        check.arg("--version");
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            check.creation_flags(0x0800_0000);
        }
        match check.output() {
            Ok(out) => out.status.success() || !out.stdout.is_empty() || !out.stderr.is_empty(),
            Err(_) => false,
        }
    }

    fn detect_c_compiler() -> Option<String> {
        C_COMPILER_CACHE
            .get_or_init(|| {
                if Self::is_executable_available("gcc") {
                    return Some("gcc".to_string());
                }
                if Self::is_executable_available("clang") {
                    return Some("clang".to_string());
                }
                #[cfg(target_os = "windows")]
                {
                    let common_paths = [
                        r"C:\Program Files\LLVM\bin\clang.exe",
                        r"C:\LLVM\bin\clang.exe",
                        r"C:\msys64\ucrt64\bin\gcc.exe",
                        r"C:\msys64\mingw64\bin\gcc.exe",
                        r"C:\MinGW\bin\gcc.exe",
                        r"C:\Strawberry\c\bin\gcc.exe",
                        r"C:\Program Files\CodeBlocks\MinGW\bin\gcc.exe",
                    ];
                    for path in common_paths {
                        if Path::new(path).exists() {
                            return Some(format!("\"{path}\""));
                        }
                    }
                    if Self::is_executable_available("cl") {
                        return Some("cl".to_string());
                    }
                }
                None
            })
            .clone()
    }

    fn detect_cpp_compiler() -> Option<String> {
        CPP_COMPILER_CACHE
            .get_or_init(|| {
                if Self::is_executable_available("g++") {
                    return Some("g++".to_string());
                }
                if Self::is_executable_available("clang++") {
                    return Some("clang++".to_string());
                }
                #[cfg(target_os = "windows")]
                {
                    let common_paths = [
                        r"C:\Program Files\LLVM\bin\clang++.exe",
                        r"C:\LLVM\bin\clang++.exe",
                        r"C:\msys64\ucrt64\bin\g++.exe",
                        r"C:\msys64\mingw64\bin\g++.exe",
                        r"C:\MinGW\bin\g++.exe",
                        r"C:\Strawberry\c\bin\g++.exe",
                        r"C:\Program Files\CodeBlocks\MinGW\bin\g++.exe",
                    ];
                    for path in common_paths {
                        if Path::new(path).exists() {
                            return Some(format!("\"{path}\""));
                        }
                    }
                    if Self::is_executable_available("cl") {
                        return Some("cl".to_string());
                    }
                }
                None
            })
            .clone()
    }

    fn detect_python_interpreter() -> Option<String> {
        PYTHON_CACHE
            .get_or_init(|| {
                if Self::is_executable_available("python") {
                    return Some("python".to_string());
                }
                if Self::is_executable_available("py") {
                    return Some("py".to_string());
                }
                if Self::is_executable_available("python3") {
                    return Some("python3".to_string());
                }
                None
            })
            .clone()
    }

    fn build_command_for_file(
        file_name: &str,
        stem: &str,
        path: &Path,
        lang: crate::syntax::Language,
    ) -> String {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match (lang, ext.as_str()) {
            (crate::syntax::Language::Python, _) | (_, "py") => {
                if let Some(py) = Self::detect_python_interpreter() {
                    format!("{py} -u \"{file_name}\"")
                } else {
                    "echo [ERROR]: No se encontro ningun interprete de Python (python o py) en el PATH.".to_string()
                }
            }
            (crate::syntax::Language::Rust, _) | (_, "rs") => {
                let mut dir = path.parent();
                let mut has_cargo = false;
                while let Some(d) = dir {
                    if d.join("Cargo.toml").exists() {
                        has_cargo = true;
                        break;
                    }
                    dir = d.parent();
                }
                if has_cargo {
                    "cargo run".to_string()
                } else {
                    #[cfg(target_os = "windows")]
                    {
                        format!("rustc \"{file_name}\" -o \"{stem}_run.exe\" && .\\\"{stem}_run.exe\"")
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        format!("rustc \"{file_name}\" -o \"{stem}_run\" && ./\"{stem}_run\"")
                    }
                }
            }
            (crate::syntax::Language::C, _) | (_, "c") => {
                if let Some(cc) = Self::detect_c_compiler() {
                    #[cfg(target_os = "windows")]
                    {
                        if cc == "cl" {
                            format!("{cc} /nologo \"{file_name}\" /Fe:\"{stem}_run.exe\" && .\\\"{stem}_run.exe\"")
                        } else {
                            format!("{cc} \"{file_name}\" -o \"{stem}_run.exe\" && .\\\"{stem}_run.exe\"")
                        }
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        format!("{cc} \"{file_name}\" -o \"{stem}_run\" && ./\"{stem}_run\"")
                    }
                } else {
                    #[cfg(target_os = "windows")]
                    {
                        "echo [ERROR]: No se encontro ningun compilador de C (gcc, clang o cl) en el sistema.".to_string()
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        "echo [ERROR]: No se encontro gcc ni clang en el sistema. Instala build-essential.".to_string()
                    }
                }
            }
            (crate::syntax::Language::Cpp, _) | (_, "cpp") | (_, "cc") | (_, "cxx") => {
                if let Some(cxx) = Self::detect_cpp_compiler() {
                    #[cfg(target_os = "windows")]
                    {
                        if cxx == "cl" {
                            format!("{cxx} /EHsc /nologo \"{file_name}\" /Fe:\"{stem}_run.exe\" && .\\\"{stem}_run.exe\"")
                        } else {
                            format!("{cxx} \"{file_name}\" -o \"{stem}_run.exe\" && .\\\"{stem}_run.exe\"")
                        }
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        format!("{cxx} \"{file_name}\" -o \"{stem}_run\" && ./\"{stem}_run\"")
                    }
                } else {
                    #[cfg(target_os = "windows")]
                    {
                        "echo [ERROR]: No se encontro ningun compilador de C++ (g++, clang++ o cl) en el sistema.".to_string()
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        "echo [ERROR]: No se encontro g++ ni clang++ en el sistema. Instala build-essential.".to_string()
                    }
                }
            }
            (crate::syntax::Language::Java, _) | (_, "java") => {
                format!("javac \"{file_name}\" && java \"{stem}\"")
            }
            (crate::syntax::Language::JavaScript, _) | (_, "js") | (_, "mjs") | (_, "cjs") => {
                format!("node \"{file_name}\"")
            }
            (crate::syntax::Language::TypeScript, _) | (_, "ts") => {
                format!("npx ts-node \"{file_name}\" || node \"{file_name}\"")
            }
            (crate::syntax::Language::Go, _) | (_, "go") => {
                format!("go run \"{file_name}\"")
            }
            (crate::syntax::Language::CSharp, _) | (_, "cs") => "dotnet run".to_string(),
            (crate::syntax::Language::Php, _) | (_, "php") => {
                format!("php \"{file_name}\"")
            }
            (crate::syntax::Language::Bash, _) | (_, "sh") => {
                format!("bash \"{file_name}\"")
            }
            (_, "bat") | (_, "cmd") => {
                format!("call \"{file_name}\"")
            }
            (_, "ps1") => {
                format!("powershell -ExecutionPolicy Bypass -File \"{file_name}\"")
            }
            (crate::syntax::Language::Html, _) | (_, "html") | (_, "htm") => {
                #[cfg(target_os = "windows")]
                {
                    format!("start \"\" \"{file_name}\"")
                }
                #[cfg(not(target_os = "windows"))]
                {
                    format!("xdg-open \"{file_name}\"")
                }
            }
            (crate::syntax::Language::Sql, _) | (_, "sql") => {
                format!("sqlite3 db.sqlite3 < \"{file_name}\"")
            }
            _ => format!("echo [Archivo guardado: {file_name}. No hay comando predeterminado configurado]"),
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;

        let mut events = Vec::new();
        if let Some(ref runner) = self.runner {
            while let Ok(event) = runner.rx_events.try_recv() {
                events.push(event);
            }
        }

        for event in events {
            changed = true;
            match event {
                RunEvent::Stdout(text) | RunEvent::Stderr(text) => {
                    let clean = strip_ansi_codes(&text);
                    Self::append_chunk(&mut self.output_lines, &clean);
                    if self.auto_scroll_output {
                        self.scroll_output_to_bottom();
                    }
                }
                RunEvent::Finished { code, duration_secs } => {
                    self.is_running = false;
                    if let Some(ref mut r) = self.runner {
                        r.is_running = false;
                    }
                    self.last_exit_code = Some(code);
                    self.last_duration_secs = Some(duration_secs);

                    self.output_lines.push("--------------------------------------------------------------------------------".to_string());
                    if code == 0 {
                        self.output_lines.push(format!(
                            "[PROCESO FINALIZADO CON ÉXITO] Código: 0 (Duración: {:.2}s)",
                            duration_secs
                        ));
                    } else {
                        self.output_lines.push(format!(
                            "[PROCESO FINALIZÓ CON ERROR] Código: {} (Duración: {:.2}s)",
                            code, duration_secs
                        ));
                    }
                    self.output_lines.push("================================================================================".to_string());
                    if self.auto_scroll_output {
                        self.scroll_output_to_bottom();
                    }
                }
                RunEvent::Stopped => {
                    self.is_running = false;
                    self.output_lines.push("--------------------------------------------------------------------------------".to_string());
                    self.output_lines.push("[PROCESO DETENIDO]".to_string());
                    self.output_lines.push("================================================================================".to_string());
                    if self.auto_scroll_output {
                        self.scroll_output_to_bottom();
                    }
                }
                RunEvent::Error(err) => {
                    self.is_running = false;
                    self.output_lines.push(format!("[ERROR]: {err}"));
                    if self.auto_scroll_output {
                        self.scroll_output_to_bottom();
                    }
                }
            }
        }

        let mut shell_texts = Vec::new();
        if let Some(ref rx) = self.shell_rx_stdout {
            while let Ok(text) = rx.try_recv() {
                shell_texts.push(text);
            }
        }

        for text in shell_texts {
            changed = true;
            let clean = strip_ansi_codes(&text);
            Self::append_chunk(&mut self.shell_lines, &clean);
            if self.auto_scroll_shell {
                self.scroll_shell_to_bottom();
            }
        }

        changed
    }

    /// Añade texto respetando `\n` y sobrescritura de línea por `\r` solitario.
    fn append_chunk(lines: &mut Vec<String>, text: &str) {
        if lines.is_empty() {
            lines.push(String::new());
        }

        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    // CRLF normal: ignorar \r y dejar que \n abra línea nueva.
                    continue;
                }
                // \r solitario (ej. barra de progreso): reiniciar línea actual.
                if let Some(last) = lines.last_mut() {
                    last.clear();
                }
                continue;
            }
            if ch == '\n' {
                lines.push(String::new());
            } else if ch == '\t' {
                if let Some(last) = lines.last_mut() {
                    last.push_str("    ");
                }
            } else if !ch.is_control() {
                if let Some(last) = lines.last_mut() {
                    last.push(ch);
                }
            }
        }

        if lines.len() > 4000 {
            let drain = lines.len() - 3000;
            lines.drain(0..drain);
        }
    }

    pub fn send_input(&mut self) {
        let input = self.input_line.trim_end_matches(['\r', '\n']).to_string();
        if !input.is_empty() {
            if self.history.last() != Some(&input) {
                self.history.push(input.clone());
                if self.history.len() > 200 {
                    self.history.remove(0);
                }
            }
            self.history_idx = None;
        }

        match self.active_tab {
            TerminalTab::Output => {
                if self.is_running {
                    if let Some(ref runner) = self.runner {
                        let _ = runner.tx_stdin.send(input.clone());
                    }
                    self.output_lines.push(format!("> {input}"));
                    self.scroll_output_to_bottom();
                } else {
                    self.output_lines.push(format!("> {input}"));
                    self.output_lines.push("[No hay ningún proceso en ejecución. Presiona F5 o Ejecutar]".to_string());
                    self.scroll_output_to_bottom();
                }
            }
            TerminalTab::Shell => {
                if let Some(ref tx) = self.shell_tx_stdin {
                    let _ = tx.send(input.clone());
                }
                self.shell_lines.push(format!("$ {input}"));
                self.scroll_shell_to_bottom();
            }
        }

        self.input_line.clear();
    }

    pub fn clear_active(&mut self) {
        match self.active_tab {
            TerminalTab::Output => {
                self.output_lines.clear();
                self.output_lines.push("Salida limpiada.".to_string());
                self.output_scroll = 0;
            }
            TerminalTab::Shell => {
                self.shell_lines.clear();
                self.shell_scroll = 0;
            }
        }
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next_idx = match self.history_idx {
            None => self.history.len().saturating_sub(1),
            Some(i) => i.saturating_sub(1),
        };
        self.history_idx = Some(next_idx);
        if let Some(cmd) = self.history.get(next_idx) {
            self.input_line = cmd.clone();
        }
    }

    pub fn history_down(&mut self) {
        if let Some(i) = self.history_idx {
            if i + 1 < self.history.len() {
                let next_idx = i + 1;
                self.history_idx = Some(next_idx);
                if let Some(cmd) = self.history.get(next_idx) {
                    self.input_line = cmd.clone();
                }
            } else {
                self.history_idx = None;
                self.input_line.clear();
            }
        }
    }

    /// Desplaza el scroll de forma segura (sin overflow cuando el scroll está en `usize::MAX`).
    pub fn scroll(&mut self, delta: i32, visible_lines: usize) {
        match self.active_tab {
            TerminalTab::Output => {
                let total = self.output_lines.len();
                let max_scroll = total.saturating_sub(visible_lines);
                let current = self.output_scroll.min(max_scroll);
                let new_scroll = (current as isize + delta as isize).clamp(0, max_scroll as isize) as usize;
                self.output_scroll = new_scroll;
                self.auto_scroll_output = new_scroll >= max_scroll;
            }
            TerminalTab::Shell => {
                let total = self.shell_lines.len();
                let max_scroll = total.saturating_sub(visible_lines);
                let current = self.shell_scroll.min(max_scroll);
                let new_scroll = (current as isize + delta as isize).clamp(0, max_scroll as isize) as usize;
                self.shell_scroll = new_scroll;
                self.auto_scroll_shell = new_scroll >= max_scroll;
            }
        }
    }

    pub fn scroll_output_to_bottom(&mut self) {
        self.auto_scroll_output = true;
        self.output_scroll = self.output_lines.len();
    }

    pub fn scroll_shell_to_bottom(&mut self) {
        self.auto_scroll_shell = true;
        self.shell_scroll = self.shell_lines.len();
    }

    pub fn active_lines(&self) -> &Vec<String> {
        match self.active_tab {
            TerminalTab::Output => &self.output_lines,
            TerminalTab::Shell => &self.shell_lines,
        }
    }

    pub fn active_scroll(&self) -> usize {
        match self.active_tab {
            TerminalTab::Output => self.output_scroll,
            TerminalTab::Shell => self.shell_scroll,
        }
    }

    pub fn set_active_scroll(&mut self, scroll: usize) {
        match self.active_tab {
            TerminalTab::Output => {
                self.output_scroll = scroll;
                let max_s = self.output_lines.len().saturating_sub(1);
                self.auto_scroll_output = scroll >= max_s;
            }
            TerminalTab::Shell => {
                self.shell_scroll = scroll;
                let max_s = self.shell_lines.len().saturating_sub(1);
                self.auto_scroll_shell = scroll >= max_s;
            }
        }
    }

    /// Borra la palabra anterior en la barra de entrada de la terminal (`Ctrl+Backspace`).
    pub fn delete_input_word_left(&mut self) {
        let trimmed_end = self.input_line.trim_end_matches(char::is_whitespace).len();
        if trimmed_end < self.input_line.len() && trimmed_end == 0 {
            self.input_line.clear();
            return;
        }
        let mut chars: Vec<char> = self.input_line.chars().collect();
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
        self.input_line = chars.into_iter().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::Language;

    #[test]
    fn test_strip_ansi_codes() {
        assert_eq!(strip_ansi_codes("\x1b[32mOK\x1b[0m normal"), "OK normal");
        assert_eq!(strip_ansi_codes("\x1b]0;Titulo\x07Hola"), "Hola");
        assert_eq!(strip_ansi_codes("sin códigos ñ"), "sin códigos ñ");
    }

    #[test]
    fn test_utf8_chunk_decoder_split_multibyte() {
        let mut dec = Utf8ChunkDecoder::new();
        let bytes = "áéíóú 🚀 ñ".as_bytes();
        // Partir justo a mitad de un carácter UTF-8 de 2 bytes ('á' = [0xC3, 0xA1]) y de 4 bytes ('🚀')
        let p1 = dec.push(&bytes[..1]);
        let p2 = dec.push(&bytes[1..12]);
        let p3 = dec.push(&bytes[12..]);
        let rem = dec.flush();
        assert_eq!(format!("{p1}{p2}{p3}{rem}"), "áéíóú 🚀 ñ");
    }

    #[test]
    fn test_scroll_from_bottom_does_not_jump_to_zero() {
        let mut term = TerminalState::new();
        term.output_lines = (0..100).map(|i| format!("linea {i}")).collect();
        term.scroll_output_to_bottom();
        term.scroll(-3, 20);
        // max_scroll = 80; subir 3 líneas debe dar 77, nunca 0
        assert_eq!(term.output_scroll, 77);
        assert!(!term.auto_scroll_output);
    }

    #[test]
    fn test_stop_runner_does_not_deadlock() {
        let temp_dir = std::env::temp_dir();
        let py_path = temp_dir.join("amoxli_test_sleep_stop.py");
        std::fs::write(&py_path, "import time\nprint('iniciando', flush=True)\ntime.sleep(30)\n")
            .expect("escribir script sleep");

        let mut term = TerminalState::new();
        term.start_run(&py_path, Language::Python);
        if term.is_running {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let t0 = Instant::now();
            term.stop_runner();
            assert!(
                t0.elapsed() < std::time::Duration::from_secs(5),
                "stop_runner tardó demasiado (posible deadlock)"
            );
            assert!(!term.is_running);
        }
        let _ = std::fs::remove_file(&py_path);
    }
}

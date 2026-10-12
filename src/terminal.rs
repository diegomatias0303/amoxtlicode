use crossbeam_channel::{unbounded, Receiver, Sender};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

/// Pestañas disponibles en el panel inferior integrado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalTab {
    /// Salida directa de compilación/ejecución (estilo NetBeans / Visual Studio)
    Output,
    /// Terminal interactiva de comandos (PowerShell / CMD)
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

/// Manejador de un proceso en ejecución para la pestaña de Salida (Run).
#[allow(dead_code)]
pub struct RunnerProcess {
    pub child: Arc<Mutex<Option<Child>>>,
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
        if let Ok(mut lock) = self.child.lock() {
            if let Some(ref mut c) = *lock {
                let pid = c.id();
                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::process::CommandExt;
                    let _ = Command::new("taskkill")
                        .args(["/F", "/T", "/PID", &pid.to_string()])
                        .creation_flags(0x0800_0000)
                        .status();
                }
                let _ = c.kill();
            }
        }
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
    shell_tx_stdin: Option<Sender<String>>,
    shell_rx_stdout: Option<Receiver<String>>,

    // --- Barra de entrada (Input Prompt) ---
    pub input_line: String,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,

    // --- Estado de interacción UI ---
    pub is_resizing: bool,
    /// Escala de zoom de la fuente del panel (1.0 = 100%).
    pub font_scale: f32,
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
                " AmoxliCode Output Engine v2.0 - [Modo NetBeans / Visual Studio]".to_string(),
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

    /// Modifica el nivel de zoom de la terminal (acotado entre 60% y 250%).
    pub fn zoom(&mut self, delta: f32) {
        self.font_scale = (self.font_scale + delta).clamp(0.6, 2.5);
    }

    /// Restablece el zoom de la terminal al 100%.
    pub fn zoom_reset(&mut self) {
        self.font_scale = 1.0;
    }

    /// Tamaño de fuente efectivo según el nivel de zoom.
    pub fn eff_font_size(&self) -> f32 {
        13.0 * self.font_scale
    }

    /// Altura de línea efectiva según el nivel de zoom.
    pub fn eff_line_height(&self) -> f32 {
        18.0 * self.font_scale
    }

    /// Inicializa la shell interactiva en segundo plano
    pub fn init_shell(&mut self) {
        let (tx_out, rx_stdout) = unbounded();
        let (tx_stdin, rx_in) = unbounded::<String>();

        #[cfg(target_os = "windows")]
        let shell = "cmd.exe";
        #[cfg(not(target_os = "windows"))]
        let shell = "bash";

        let mut cmd = Command::new(shell);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.shell_lines.push(format!("Error al iniciar shell: {e}"));
                return;
            }
        };

        let mut stdout = match child.stdout {
            Some(s) => s,
            None => return,
        };
        let mut stderr = match child.stderr {
            Some(s) => s,
            None => return,
        };
        let mut stdin = match child.stdin {
            Some(s) => s,
            None => return,
        };

        let tx_out_clone = tx_out.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx_out_clone.send(s);
            }
        });

        let tx_out_stderr = tx_out.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx_out_stderr.send(s);
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
        self.shell_lines.push("Terminal interactiva lista. Escribe comandos abajo.".to_string());
    }

    /// Ejecuta el archivo actual con el compilador/intérprete apropiado.
    pub fn start_run(&mut self, file_path: &Path, lang: crate::syntax::Language) {
        // Si hay un proceso anterior ejecutándose, detenerlo limpiamente
        self.stop_runner();

        let parent_dir = file_path
            .parent()
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
        self.output_lines.push(String::new()); // Inicia línea nueva para la salida del programa

        let (tx_events, rx_events) = unbounded::<RunEvent>();
        let (tx_stdin, rx_stdin) = unbounded::<String>();

        #[cfg(target_os = "windows")]
        let mut cmd = {
            use std::os::windows::process::CommandExt;
            let mut c = Command::new("cmd");
            c.raw_arg(format!("/C {}", command_str));
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

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW evita ventana negra flotante
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("Error al iniciar ejecución: {e}");
                self.output_lines.push(format!("[ERROR]: {err_msg}"));
                self.output_lines.push("--------------------------------------------------------------------------------".to_string());
                return;
            }
        };

        let mut stdout = child.stdout.take().expect("stdout");
        let mut stderr = child.stderr.take().expect("stderr");
        let mut stdin = child.stdin.take().expect("stdin");

        let child_arc = Arc::new(Mutex::new(Some(child)));
        let child_for_wait = Arc::clone(&child_arc);

        // Hilo lector de stdout
        let tx_out = tx_events.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx_out.send(RunEvent::Stdout(s));
            }
        });

        // Hilo lector de stderr
        let tx_err = tx_events.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx_err.send(RunEvent::Stderr(s));
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

        // Hilo de espera de finalización del proceso
        let tx_finished = tx_events.clone();
        let start_time = Instant::now();
        thread::spawn(move || {
            let status = {
                let mut lock = child_for_wait.lock().unwrap();
                if let Some(ref mut c) = *lock {
                    c.wait().ok()
                } else {
                    None
                }
            };
            let duration_secs = start_time.elapsed().as_secs_f32();
            let code = status.and_then(|s| s.code()).unwrap_or(-1);
            let _ = tx_finished.send(RunEvent::Finished {
                code,
                duration_secs,
            });
        });

        self.runner = Some(RunnerProcess {
            child: child_arc,
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

    /// Detiene el proceso actual si está ejecutándose.
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
        self.is_running = false;
    }

    /// Comprueba si un comando o ejecutable responde en el sistema sin mostrar ventanas.
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

    /// Detecta el compilador de C disponible (gcc, clang, cl o rutas comunes en Windows).
    fn detect_c_compiler() -> Option<String> {
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
    }

    /// Detecta el compilador de C++ disponible (g++, clang++, cl o rutas comunes en Windows).
    fn detect_cpp_compiler() -> Option<String> {
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
    }

    /// Detecta el intérprete de Python disponible (python, py, python3).
    fn detect_python_interpreter() -> Option<String> {
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
    }

    /// Construye el comando de ejecución según el lenguaje o la extensión.
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
                // Si hay Cargo.toml en la carpeta o ascendentes, cargo run; sino rustc
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
                        "echo [ERROR]: No se encontro ningun compilador de C (gcc, clang o cl) en el sistema. Instala LLVM (clang) o MinGW (gcc) y agregalo al PATH.".to_string()
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
                        "echo [ERROR]: No se encontro ningun compilador de C++ (g++, clang++ o cl) en el sistema. Instala LLVM (clang) o MinGW (g++) y agregalo al PATH.".to_string()
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
            (crate::syntax::Language::CSharp, _) | (_, "cs") => {
                "dotnet run".to_string()
            }
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

    /// Recibe nuevos datos de salida del runner y de la shell interactiva.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;

        // 1. Recibir eventos del Runner
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
                    Self::append_chunk(&mut self.output_lines, &text);
                    if self.auto_scroll_output {
                        self.scroll_output_to_bottom();
                    }
                }
                RunEvent::Finished { code, duration_secs } => {
                    self.is_running = false;
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

        // 2. Recibir salida de la shell interactiva
        let mut shell_texts = Vec::new();
        if let Some(ref rx) = self.shell_rx_stdout {
            while let Ok(text) = rx.try_recv() {
                shell_texts.push(text);
            }
        }

        for text in shell_texts {
            changed = true;
            Self::append_chunk(&mut self.shell_lines, &text);
            if self.auto_scroll_shell {
                self.scroll_shell_to_bottom();
            }
        }

        changed
    }

    /// Añade texto a una lista de líneas respetando saltos de línea (\n y \r).
    fn append_chunk(lines: &mut Vec<String>, text: &str) {
        if lines.is_empty() {
            lines.push(String::new());
        }

        for ch in text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                lines.push(String::new());
            } else if let Some(last) = lines.last_mut() {
                last.push(ch);
            }
        }

        // Limitar memoria máxima (máximo 4000 líneas)
        if lines.len() > 4000 {
            let drain = lines.len() - 3000;
            lines.drain(0..drain);
        }
    }

    /// Envía la línea actual de entrada del usuario (a stdin del runner o a la shell).
    pub fn send_input(&mut self) {
        let input = self.input_line.trim_end_matches(['\r', '\n']).to_string();
        if !input.is_empty() {
            self.history.push(input.clone());
            self.history_idx = None;
        }

        match self.active_tab {
            TerminalTab::Output => {
                if self.is_running {
                    if let Some(ref runner) = self.runner {
                        let _ = runner.tx_stdin.send(input.clone());
                    }
                    // Mostrar en pantalla la entrada tecleada por el usuario
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

    /// Limpia el contenido de la pestaña activa.
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

    /// Historial: flecha arriba
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

    /// Historial: flecha abajo
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

    /// Desplaza el scroll del contenido activo
    pub fn scroll(&mut self, delta: i32, visible_lines: usize) {
        match self.active_tab {
            TerminalTab::Output => {
                let total = self.output_lines.len();
                let max_scroll = total.saturating_sub(visible_lines);
                let new_scroll = (self.output_scroll as i32 + delta).clamp(0, max_scroll as i32) as usize;
                self.output_scroll = new_scroll;
                self.auto_scroll_output = new_scroll >= max_scroll;
            }
            TerminalTab::Shell => {
                let total = self.shell_lines.len();
                let max_scroll = total.saturating_sub(visible_lines);
                let new_scroll = (self.shell_scroll as i32 + delta).clamp(0, max_scroll as i32) as usize;
                self.shell_scroll = new_scroll;
                self.auto_scroll_shell = new_scroll >= max_scroll;
            }
        }
    }

    pub fn scroll_output_to_bottom(&mut self) {
        self.auto_scroll_output = true;
        self.output_scroll = usize::MAX; // Se clampea en rebuild
    }

    pub fn scroll_shell_to_bottom(&mut self) {
        self.auto_scroll_shell = true;
        self.shell_scroll = usize::MAX;
    }

    /// Devuelve las líneas de la pestaña actualmente activa.
    pub fn active_lines(&self) -> &Vec<String> {
        match self.active_tab {
            TerminalTab::Output => &self.output_lines,
            TerminalTab::Shell => &self.shell_lines,
        }
    }

    /// Devuelve el scroll de la pestaña activa.
    pub fn active_scroll(&self) -> usize {
        match self.active_tab {
            TerminalTab::Output => self.output_scroll,
            TerminalTab::Shell => self.shell_scroll,
        }
    }

    pub fn set_active_scroll(&mut self, scroll: usize) {
        match self.active_tab {
            TerminalTab::Output => self.output_scroll = scroll,
            TerminalTab::Shell => self.shell_scroll = scroll,
        }
    }
}

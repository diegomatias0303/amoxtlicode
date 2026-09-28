use crossbeam_channel::{unbounded, Receiver, Sender};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;

pub struct TerminalState {
    pub is_open: bool,
    pub content: String,
    pub input_line: String,
    pub rx_stdout: Receiver<String>,
    pub tx_stdin: Sender<String>,
    pub history: Vec<String>,
}

impl TerminalState {
    pub fn new() -> Self {
        let (tx_out, rx_stdout) = unbounded();
        let (tx_stdin, rx_in) = unbounded::<String>();

        // Spawn shell
        #[cfg(target_os = "windows")]
        let shell = "cmd.exe";
        #[cfg(not(target_os = "windows"))]
        let shell = "bash";

        let mut child = Command::new(shell)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to start shell");

        let mut stdout = child.stdout.take().expect("Failed to capture stdout");
        let mut stderr = child.stderr.take().expect("Failed to capture stderr");
        let mut stdin = child.stdin.take().expect("Failed to capture stdin");

        // Thread to read stdout
        let tx_out_clone = tx_out.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 { break; }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx_out_clone.send(s);
            }
        });

        // Thread to read stderr
        thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 { break; }
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx_out.send(s);
            }
        });

        // Thread to write stdin
        thread::spawn(move || {
            while let Ok(mut cmd) = rx_in.recv() {
                cmd.push('\n');
                if stdin.write_all(cmd.as_bytes()).is_err() {
                    break;
                }
                if stdin.flush().is_err() {
                    break;
                }
            }
        });

        Self {
            is_open: false,
            content: String::new(),
            input_line: String::new(),
            rx_stdout,
            tx_stdin,
            history: Vec::new(),
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(text) = self.rx_stdout.try_recv() {
            self.content.push_str(&text);
            changed = true;
        }
        if changed {
            // Keep content within reasonable limit to avoid memory overflow
            if self.content.len() > 100_000 {
                let start = self.content.len() - 50_000;
                if let Some(idx) = self.content[start..].find('\n') {
                    self.content = self.content[start + idx + 1..].to_string();
                }
            }
        }
        changed
    }

    pub fn send_input(&mut self) {
        let cmd = self.input_line.clone();
        let _ = self.tx_stdin.send(cmd);
        self.input_line.clear();
    }
}

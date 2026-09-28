//! AmoxliCode - Fase 6: cliente LSP multilenguaje (IntelliSense)
//!
//! Habla el protocolo LSP (JSON-RPC sobre stdin/stdout) con servidores
//! de lenguaje en un hilo aparte: el hilo principal nunca espera bloqueado.
//! Soporta múltiples lenguajes (C con clangd, JS/TS, Java, HTML, CSS, SQL)
//! y gestiona dinámicamente las sesiones según los archivos abiertos.
//!
//! Nota de alcance: usamos `serde_json::Value` "a mano" en vez del crate
//! `lsp-types` para reducir el riesgo de choques de versión — hablamos el
//! mismo protocolo igual, solo que sin la capa de tipos estrictos.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{anyhow, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use log::{error, info, warn};
use serde_json::{json, Value};

use crate::syntax::Language;

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub insert_text: String,
}

pub enum LspRequest {
    /// Sincroniza el contenido completo de un documento abierto en su respectivo LSP.
    SyncDoc {
        uri: String,
        language: Language,
        text: String,
    },
    /// Pide autocompletado en la posición dada (línea/columna en base 0).
    Completion {
        uri: String,
        language: Language,
        line: u32,
        character: u32,
    },
}

pub enum LspEvent {
    Completions(Vec<CompletionItem>),
}

/// Lanza el supervisor de LSP en un hilo de fondo y devuelve los canales para
/// comunicarse con él sin bloquear el render loop.
pub fn spawn() -> (Sender<LspRequest>, Receiver<LspEvent>) {
    let (req_tx, req_rx) = unbounded::<LspRequest>();
    let (evt_tx, evt_rx) = unbounded::<LspEvent>();

    std::thread::spawn(move || {
        if let Err(e) = run(req_rx, evt_tx) {
            error!("el hilo de LSP terminó con un error: {e:?}");
        }
    });

    (req_tx, evt_rx)
}

struct LspSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    open_docs: HashSet<String>,
    doc_versions: HashMap<String, i64>,
}

impl LspSession {
    fn spawn(language: Language) -> Result<Self> {
        let (cmd, args) = language.lsp_command();
        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("no se pudo iniciar '{cmd}' para {language:?}: {e}"))?;

        let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("sin stdin"))?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or_else(|| anyhow!("sin stdout"))?);

        let mut next_id: i64 = 1;
        let init_id = next_id;
        next_id += 1;

        send(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": init_id,
                "method": "initialize",
                "params": {
                    "processId": std::process::id(),
                    "rootUri": null,
                    "capabilities": {}
                }
            }),
        )?;

        loop {
            let msg = read(&mut stdout)?;
            if msg.get("id") == Some(&json!(init_id)) {
                break;
            }
        }

        send(
            &mut stdin,
            &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
        )?;

        info!("Servidor LSP '{cmd}' iniciado exitosamente para {language:?}");

        Ok(Self {
            child,
            stdin,
            stdout,
            next_id,
            open_docs: HashSet::new(),
            doc_versions: HashMap::new(),
        })
    }

    fn sync_doc(&mut self, uri: &str, language: Language, text: &str) -> Result<()> {
        if !self.open_docs.contains(uri) {
            self.open_docs.insert(uri.to_string());
            self.doc_versions.insert(uri.to_string(), 1);
            send(
                &mut self.stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didOpen",
                    "params": {
                        "textDocument": {
                            "uri": uri,
                            "languageId": language.language_id(),
                            "version": 1,
                            "text": text
                        }
                    }
                }),
            )
        } else {
            let version = self.doc_versions.entry(uri.to_string()).or_insert(1);
            *version += 1;
            let v = *version;
            send(
                &mut self.stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didChange",
                    "params": {
                        "textDocument": { "uri": uri, "version": v },
                        "contentChanges": [{ "text": text }]
                    }
                }),
            )
        }
    }

    fn completion(&mut self, uri: &str, line: u32, character: u32) -> Result<Vec<CompletionItem>> {
        let id = self.next_id;
        self.next_id += 1;

        send(
            &mut self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": character }
                }
            }),
        )?;

        loop {
            let msg = read(&mut self.stdout)?;
            if msg.get("id") == Some(&json!(id)) {
                return Ok(parse_completions(&msg));
            }
        }
    }
}

fn run(req_rx: Receiver<LspRequest>, evt_tx: Sender<LspEvent>) -> Result<()> {
    let mut sessions: HashMap<Language, LspSession> = HashMap::new();
    let mut failed_languages: HashSet<Language> = HashSet::new();

    while let Ok(request) = req_rx.recv() {
        match request {
            LspRequest::SyncDoc { uri, language, text } => {
                let (cmd, _) = language.lsp_command();
                if cmd.is_empty() || failed_languages.contains(&language) {
                    continue;
                }
                let session = match sessions.entry(language) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        match LspSession::spawn(language) {
                            Ok(s) => e.insert(s),
                            Err(err) => {
                                warn!("LSP no disponible para {language:?}: {err}");
                                failed_languages.insert(language);
                                continue;
                            }
                        }
                    }
                };

                if let Err(e) = session.sync_doc(&uri, language, &text) {
                    warn!("Error al sincronizar documento con LSP para {language:?}: {e}");
                    sessions.remove(&language);
                }
            }
            LspRequest::Completion { uri, language, line, character } => {
                let (cmd, _) = language.lsp_command();
                if cmd.is_empty() {
                    continue;
                }
                if let Some(session) = sessions.get_mut(&language) {
                    match session.completion(&uri, line, character) {
                        Ok(items) => {
                            let _ = evt_tx.send(LspEvent::Completions(items));
                        }
                        Err(e) => {
                            warn!("Error en completion de LSP para {language:?}: {e}");
                            sessions.remove(&language);
                        }
                    }
                }
            }
        }
    }

    for (_, mut session) in sessions {
        let _ = session.child.kill();
    }

    Ok(())
}

fn parse_completions(msg: &Value) -> Vec<CompletionItem> {
    let raw_items = msg
        .get("result")
        .and_then(|r| r.get("items").or(Some(r)))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut items: Vec<(String, CompletionItem)> = raw_items
        .iter()
        .filter_map(|item| {
            let label = item.get("label")?.as_str()?.to_string();
            let insert_text = item
                .get("insertText")
                .and_then(|v| v.as_str())
                .unwrap_or(&label)
                .to_string();
            let sort_key = item
                .get("sortText")
                .and_then(|v| v.as_str())
                .unwrap_or(&label)
                .to_string();
            Some((sort_key, CompletionItem { label, insert_text }))
        })
        .collect();

    items.sort_by(|a, b| a.0.cmp(&b.0));

    items.into_iter().map(|(_, item)| item).take(15).collect()
}

fn send(stdin: &mut ChildStdin, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(&body)?;
    stdin.flush()?;
    Ok(())
}

fn read(reader: &mut BufReader<ChildStdout>) -> Result<Value> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.is_empty() {
            return Err(anyhow!("el servidor LSP cerró la conexión inesperadamente"));
        }
        if line == "\r\n" {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = content_length.ok_or_else(|| anyhow!("mensaje LSP sin Content-Length"))?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}
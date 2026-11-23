//! AmoxliCode - Fase 6: cliente LSP multilenguaje (IntelliSense)
//!
//! Habla el protocolo LSP (JSON-RPC sobre stdin/stdout) con servidores de
//! lenguaje en un hilo aparte: el hilo principal nunca espera bloqueado.
//! Maneja respuestas a peticiones server->client, limpieza de snippets LSP,
//! cierre limpio de documentos y terminación de procesos hijos sin zombis.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{anyhow, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use log::{error, info, warn};
use serde_json::{json, Value};

use crate::syntax::Language;

const MAX_LSP_BODY_BYTES: usize = 16 * 1024 * 1024;

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
    /// Notifica al LSP que el documento fue cerrado (libera memoria en el servidor).
    CloseDoc {
        uri: String,
        language: Language,
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

impl Drop for LspSession {
    fn drop(&mut self) {
        let _ = send(
            &mut self.stdin,
            &json!({ "jsonrpc": "2.0", "method": "exit", "params": null }),
        );
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl LspSession {
    fn spawn(language: Language) -> Result<Self> {
        let (cmd, args) = language.lsp_command();
        let mut command = Command::new(cmd);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let mut child = command
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
                    "capabilities": {
                        "textDocument": {
                            "completion": {
                                "completionItem": {
                                    "snippetSupport": false
                                }
                            }
                        }
                    }
                }
            }),
        )?;

        loop {
            let msg = read(&mut stdout)?;
            if msg.get("id") == Some(&json!(init_id)) && msg.get("method").is_none() {
                break;
            }
            reply_if_server_request(&mut stdin, &msg);
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

    fn close_doc(&mut self, uri: &str) -> Result<()> {
        if self.open_docs.remove(uri) {
            self.doc_versions.remove(uri);
            send(
                &mut self.stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didClose",
                    "params": {
                        "textDocument": { "uri": uri }
                    }
                }),
            )?;
        }
        Ok(())
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
            if msg.get("id") == Some(&json!(id)) && msg.get("method").is_none() {
                return Ok(parse_completions(&msg));
            }
            reply_if_server_request(&mut self.stdin, &msg);
        }
    }
}

/// Si el servidor LSP envía una petición al cliente (tiene `id` y `method`,
/// como `workspace/configuration` o `client/registerCapability`), respondemos
/// con `result: null` para evitar bloqueos del servidor.
fn reply_if_server_request(stdin: &mut ChildStdin, msg: &Value) {
    if let (Some(req_id), Some(_method)) = (msg.get("id"), msg.get("method")) {
        let _ = send(
            stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": null
            }),
        );
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
            LspRequest::CloseDoc { uri, language } => {
                if let Some(session) = sessions.get_mut(&language) {
                    if let Err(e) = session.close_doc(&uri) {
                        warn!("Error al cerrar documento en LSP para {language:?}: {e}");
                        sessions.remove(&language);
                    }
                }
            }
            LspRequest::Completion { uri, language, line, character } => {
                let (cmd, _) = language.lsp_command();
                if cmd.is_empty() || failed_languages.contains(&language) {
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

    // `Drop` de cada `LspSession` termina limpiamente cada proceso hijo.
    sessions.clear();
    Ok(())
}

/// Limpia marcadores de snippet LSP (`$0`, `${1:param}`, `${1|a,b|}`) para
/// insertar texto limpio en el editor.
pub fn strip_lsp_snippet(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            let next = raw[i + 1..].chars().next().unwrap();
            if matches!(next, '$' | '}' | '\\') {
                out.push(next);
                i += 1 + next.len_utf8();
                continue;
            }
        }
        if bytes[i] == b'$' {
            if i + 1 < len && bytes[i + 1] == b'{' {
                // ${N:default} o ${N|opt1,opt2|} o ${N}
                let mut j = i + 2;
                while j < len && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j < len && bytes[j] == b':' {
                    j += 1;
                    let start_def = j;
                    let mut depth = 1usize;
                    while j < len && depth > 0 {
                        if bytes[j] == b'\\' && j + 1 < len {
                            j += 2;
                            continue;
                        }
                        if bytes[j] == b'{' {
                            depth += 1;
                        } else if bytes[j] == b'}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        j += 1;
                    }
                    if j <= len && raw.is_char_boundary(start_def) && raw.is_char_boundary(j) {
                        out.push_str(&strip_lsp_snippet(&raw[start_def..j]));
                    }
                    i = (j + 1).min(len);
                    continue;
                } else if j < len && bytes[j] == b'|' {
                    j += 1;
                    let start_opt = j;
                    while j < len && bytes[j] != b',' && bytes[j] != b'|' {
                        j += 1;
                    }
                    if raw.is_char_boundary(start_opt) && raw.is_char_boundary(j) {
                        out.push_str(&raw[start_opt..j]);
                    }
                    while j < len && bytes[j] != b'}' {
                        j += 1;
                    }
                    i = (j + 1).min(len);
                    continue;
                } else {
                    while j < len && bytes[j] != b'}' {
                        j += 1;
                    }
                    i = (j + 1).min(len);
                    continue;
                }
            } else if i + 1 < len && bytes[i + 1].is_ascii_digit() {
                i += 2;
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                continue;
            }
        }
        let ch = raw[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
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
            let raw_insert = item
                .get("insertText")
                .and_then(|v| v.as_str())
                .unwrap_or(&label);
            let is_snippet = item
                .get("insertTextFormat")
                .and_then(|v| v.as_i64())
                == Some(2);
            let insert_text = if is_snippet {
                strip_lsp_snippet(raw_insert)
            } else {
                raw_insert.to_string()
            };
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
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, val)) = trimmed.split_once(':') {
            if key.trim().eq_ignore_ascii_case("content-length") {
                content_length = val.trim().parse().ok();
            }
        }
    }
    let len = content_length.ok_or_else(|| anyhow!("mensaje LSP sin Content-Length"))?;
    if len > MAX_LSP_BODY_BYTES {
        return Err(anyhow!("mensaje LSP demasiado grande ({len} bytes)"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_lsp_snippet_placeholders() {
        assert_eq!(strip_lsp_snippet("printf($1)$0"), "printf()");
        assert_eq!(strip_lsp_snippet("fn ${1:nombre}(${2:valor}: i32)$0"), "fn nombre(valor: i32)");
        assert_eq!(strip_lsp_snippet("color: ${1|red,green,blue|};"), "color: red;");
        assert_eq!(strip_lsp_snippet(r"cost = \$100"), "cost = $100");
        assert_eq!(strip_lsp_snippet("simple_ident"), "simple_ident");
    }
}

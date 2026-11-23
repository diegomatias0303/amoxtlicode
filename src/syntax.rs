//! AmoxliCode - Fase 5: Resaltado de sintaxis y soporte de lenguajes
//!
//! Soporta 18 lenguajes con gramáticas nativas tree-sitter y un analizador
//! léxico UTF-8 seguro y zero-allocation para el resto de lenguajes populares.

use std::ops::Range;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Nombres de "highlight" estándar.
pub const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "function",
    "function.builtin",
    "keyword",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

/// Lenguajes de programación soportados en AmoxliCode.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Language {
    PlainText,
    Python,
    Rust,
    C,
    Cpp,
    Java,
    JavaScript,
    TypeScript,
    Html,
    Css,
    Sql,
    Json,
    Markdown,
    CSharp,
    Php,
    Go,
    Bash,
    Xml,
}

impl Language {
    pub const ALL_POPULAR: [Language; 18] = [
        Language::PlainText,
        Language::Python,
        Language::Rust,
        Language::C,
        Language::Cpp,
        Language::Java,
        Language::JavaScript,
        Language::TypeScript,
        Language::Html,
        Language::Css,
        Language::Sql,
        Language::Json,
        Language::Markdown,
        Language::CSharp,
        Language::Php,
        Language::Go,
        Language::Bash,
        Language::Xml,
    ];

    pub fn display_name(&self) -> &'static str {
        match self {
            Language::PlainText => "Texto plano (.txt)",
            Language::Python => "Python (.py)",
            Language::Rust => "Rust (.rs)",
            Language::C => "C (.c)",
            Language::Cpp => "C++ (.cpp)",
            Language::Java => "Java (.java)",
            Language::JavaScript => "JavaScript (.js)",
            Language::TypeScript => "TypeScript (.ts)",
            Language::Html => "HTML (.html)",
            Language::Css => "CSS (.css)",
            Language::Sql => "SQL (.sql)",
            Language::Json => "JSON (.json)",
            Language::Markdown => "Markdown (.md)",
            Language::CSharp => "C# (.cs)",
            Language::Php => "PHP (.php)",
            Language::Go => "Go (.go)",
            Language::Bash => "Bash / Shell (.sh)",
            Language::Xml => "XML (.xml)",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Language::PlainText => "Texto plano",
            Language::Python => "Python",
            Language::Rust => "Rust",
            Language::C => "C",
            Language::Cpp => "C++",
            Language::Java => "Java",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Html => "HTML",
            Language::Css => "CSS",
            Language::Sql => "SQL",
            Language::Json => "JSON",
            Language::Markdown => "Markdown",
            Language::CSharp => "C#",
            Language::Php => "PHP",
            Language::Go => "Go",
            Language::Bash => "Bash",
            Language::Xml => "XML",
        }
    }

    pub fn default_extension(&self) -> &'static str {
        match self {
            Language::PlainText => "txt",
            Language::Python => "py",
            Language::Rust => "rs",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Java => "java",
            Language::JavaScript => "js",
            Language::TypeScript => "ts",
            Language::Html => "html",
            Language::Css => "css",
            Language::Sql => "sql",
            Language::Json => "json",
            Language::Markdown => "md",
            Language::CSharp => "cs",
            Language::Php => "php",
            Language::Go => "go",
            Language::Bash => "sh",
            Language::Xml => "xml",
        }
    }

    pub fn language_id(&self) -> &'static str {
        match self {
            Language::PlainText => "plaintext",
            Language::Python => "python",
            Language::Rust => "rust",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Java => "java",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Html => "html",
            Language::Css => "css",
            Language::Sql => "sql",
            Language::Json => "json",
            Language::Markdown => "markdown",
            Language::CSharp => "csharp",
            Language::Php => "php",
            Language::Go => "go",
            Language::Bash => "shellscript",
            Language::Xml => "xml",
        }
    }

    /// Prefijo de comentario de línea para alternar comentarios con Ctrl+/.
    pub fn line_comment_prefix(&self) -> Option<&'static str> {
        match self {
            Language::Python | Language::Bash => Some("# "),
            Language::Sql => Some("-- "),
            Language::Rust
            | Language::C
            | Language::Cpp
            | Language::Java
            | Language::JavaScript
            | Language::TypeScript
            | Language::CSharp
            | Language::Php
            | Language::Go => Some("// "),
            _ => None,
        }
    }

    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "txt" | "text" | "log" => Language::PlainText,
            "py" | "pyw" => Language::Python,
            "rs" => Language::Rust,
            "c" | "h" => Language::C,
            "cpp" | "cxx" | "cc" | "hpp" | "hh" => Language::Cpp,
            "java" => Language::Java,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "ts" | "tsx" => Language::TypeScript,
            "html" | "htm" => Language::Html,
            "css" | "scss" => Language::Css,
            "sql" => Language::Sql,
            "json" | "jsonc" => Language::Json,
            "md" | "markdown" => Language::Markdown,
            "cs" => Language::CSharp,
            "php" => Language::Php,
            "go" => Language::Go,
            "sh" | "bash" | "zsh" => Language::Bash,
            "xml" | "svg" => Language::Xml,
            _ => Language::PlainText,
        }
    }

    pub fn lsp_command(&self) -> (&'static str, &'static [&'static str]) {
        match self {
            Language::PlainText => ("", &[]),
            Language::Python => ("pylsp", &[]),
            Language::Rust => ("rust-analyzer", &[]),
            Language::C => ("clangd", &[]),
            Language::Cpp => ("clangd", &[]),
            Language::Java => ("jdtls", &[]),
            Language::JavaScript => ("typescript-language-server", &["--stdio"]),
            Language::TypeScript => ("typescript-language-server", &["--stdio"]),
            Language::Html => ("vscode-html-language-server", &["--stdio"]),
            Language::Css => ("vscode-css-language-server", &["--stdio"]),
            Language::Sql => ("sqls", &[]),
            Language::Json => ("vscode-json-language-server", &["--stdio"]),
            Language::Markdown => ("", &[]),
            Language::CSharp => ("omnisharp", &[]),
            Language::Php => ("intelephense", &["--stdio"]),
            Language::Go => ("gopls", &[]),
            Language::Bash => ("bash-language-server", &["start"]),
            Language::Xml => ("", &[]),
        }
    }
}

pub enum SyntaxHighlighter {
    TreeSitter {
        highlighter: Highlighter,
        config: Box<HighlightConfiguration>,
    },
    Lexer(Language),
    None,
}

impl SyntaxHighlighter {
    pub fn new(language: Language) -> Self {
        match language {
            Language::PlainText => SyntaxHighlighter::None,
            Language::C => Self::init_tree_sitter(
                tree_sitter_c::LANGUAGE.into(),
                "c",
                tree_sitter_c::HIGHLIGHT_QUERY,
                "",
                "",
            ),
            // C++ usa el lexer con palabras clave/tipos completos de C++ (class, template,
            // namespace, nullptr, std::vector, etc.) y directivas de preprocesador.
            Language::Cpp => SyntaxHighlighter::Lexer(Language::Cpp),
            Language::Java => Self::init_tree_sitter(
                tree_sitter_java::LANGUAGE.into(),
                "java",
                tree_sitter_java::HIGHLIGHTS_QUERY,
                "",
                "",
            ),
            Language::JavaScript => Self::init_tree_sitter(
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            ),
            Language::TypeScript => Self::init_tree_sitter(
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            ),
            Language::Html => Self::init_tree_sitter(
                tree_sitter_html::LANGUAGE.into(),
                "html",
                tree_sitter_html::HIGHLIGHTS_QUERY,
                tree_sitter_html::INJECTIONS_QUERY,
                "",
            ),
            Language::Css => Self::init_tree_sitter(
                tree_sitter_css::LANGUAGE.into(),
                "css",
                tree_sitter_css::HIGHLIGHTS_QUERY,
                "",
                "",
            ),
            Language::Sql => Self::init_tree_sitter(
                tree_sitter_sequel::LANGUAGE.into(),
                "sql",
                tree_sitter_sequel::HIGHLIGHTS_QUERY,
                "",
                "",
            ),
            _ => SyntaxHighlighter::Lexer(language),
        }
    }

    fn init_tree_sitter(
        language: tree_sitter::Language,
        name: &str,
        highlights_query: &str,
        injections_query: &str,
        locals_query: &str,
    ) -> Self {
        match HighlightConfiguration::new(
            language,
            name,
            highlights_query,
            injections_query,
            locals_query,
        ) {
            Ok(mut config) => {
                config.configure(HIGHLIGHT_NAMES);
                SyntaxHighlighter::TreeSitter {
                    highlighter: Highlighter::new(),
                    config: Box::new(config),
                }
            }
            Err(e) => {
                log::error!("no se pudo cargar la gramática tree-sitter '{name}': {e:?}");
                SyntaxHighlighter::None
            }
        }
    }

    /// Analiza `source` y devuelve una lista de tramos válidos UTF-8 que cubre
    /// TODO el texto sin huecos ni panics.
    pub fn highlight(&mut self, source: &str) -> Vec<(Range<usize>, Option<&'static str>)> {
        if source.is_empty() {
            return Vec::new();
        }
        match self {
            SyntaxHighlighter::None => vec![(0..source.len(), None)],
            SyntaxHighlighter::TreeSitter {
                highlighter,
                config,
            } => {
                let events = match highlighter.highlight(config, source.as_bytes(), None, |_| None) {
                    Ok(events) => events,
                    Err(_) => return vec![(0..source.len(), None)],
                };

                let mut spans = Vec::new();
                let mut stack: Vec<&'static str> = Vec::new();
                let mut covered = 0usize;

                for event in events {
                    match event {
                        Ok(HighlightEvent::Source { start, end })
                            if start < end
                                && end <= source.len()
                                && source.is_char_boundary(start)
                                && source.is_char_boundary(end) =>
                        {
                            spans.push((start..end, stack.last().copied()));
                            covered = covered.max(end);
                        }
                        Ok(HighlightEvent::Source { .. }) => {}
                        Ok(HighlightEvent::HighlightStart(h)) => {
                            if let Some(&name) = HIGHLIGHT_NAMES.get(h.0) {
                                stack.push(name);
                            }
                        }
                        Ok(HighlightEvent::HighlightEnd) => {
                            stack.pop();
                        }
                        Err(_) => break,
                    }
                }

                // Garantizar cobertura completa hasta el final si tree-sitter abortó por error
                if covered < source.len() && source.is_char_boundary(covered) {
                    spans.push((covered..source.len(), None));
                }
                if spans.is_empty() {
                    spans.push((0..source.len(), None));
                }
                spans
            }
            SyntaxHighlighter::Lexer(lang) => lex_highlight(source, *lang),
        }
    }
}

/// Avanza un carácter UTF-8 completo a partir de `i`.
#[inline]
fn next_char_end(source: &str, i: usize) -> usize {
    source[i..]
        .chars()
        .next()
        .map(|c| i + c.len_utf8())
        .unwrap_or(source.len())
}

/// Analizador léxico rápido, 100% seguro con UTF-8 (acentos, ñ, emojis) y sin
/// asignaciones de `HashSet` por llamada.
fn lex_highlight(source: &str, lang: Language) -> Vec<(Range<usize>, Option<&'static str>)> {
    if source.is_empty() {
        return Vec::new();
    }

    if lang == Language::Markdown {
        return lex_markdown(source);
    }
    if lang == Language::Xml {
        return lex_xml(source);
    }

    let (keywords, types) = keywords_and_types_for(lang);
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut spans: Vec<(Range<usize>, Option<&'static str>)> = Vec::new();

    while i < len {
        let b = bytes[i];

        // 1. Espacios en blanco
        if b.is_ascii_whitespace() {
            let start = i;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            spans.push((start..i, None));
            continue;
        }

        // 2. Comentarios
        if (b == b'/' && i + 1 < len && (bytes[i + 1] == b'/' || bytes[i + 1] == b'*'))
            || (b == b'#' && matches!(lang, Language::Python | Language::Bash | Language::Php))
            || (b == b'<' && i + 3 < len && &bytes[i..i + 4] == b"<!--")
        {
            let start = i;
            if b == b'#' || (b == b'/' && bytes[i + 1] == b'/') {
                while i < len && bytes[i] != b'\n' {
                    i = next_char_end(source, i);
                }
            } else if b == b'<' {
                i += 4;
                while i + 2 < len && &bytes[i..i + 3] != b"-->" {
                    i = next_char_end(source, i);
                }
                i = if i + 2 < len { i + 3 } else { len };
            } else {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i = next_char_end(source, i);
                }
                i = if i + 1 < len { i + 2 } else { len };
            }
            spans.push((start..i, Some("comment")));
            continue;
        }

        // 2b. Directivas de preprocesador en C++ / C# (#include, #define, #region)
        if b == b'#' && matches!(lang, Language::Cpp | Language::CSharp) {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            spans.push((start..i, Some("keyword")));
            continue;
        }

        // 2c. Decoradores / atributos (@decorator en Python, #[...] en Rust)
        if b == b'@' && lang == Language::Python {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                i += 1;
            }
            spans.push((start..i, Some("attribute")));
            continue;
        }

        // 3. Cadenas triple-comilla en Python ("""...""" o '''...''')
        if lang == Language::Python
            && (b == b'"' || b == b'\'')
            && i + 2 < len
            && bytes[i + 1] == b
            && bytes[i + 2] == b
        {
            let q = b;
            let start = i;
            i += 3;
            while i + 2 < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i = next_char_end(source, i + 1);
                } else if bytes[i] == q && bytes[i + 1] == q && bytes[i + 2] == q {
                    i += 3;
                    break;
                } else {
                    i = next_char_end(source, i);
                }
            }
            if i + 2 >= len && i < len && !(i >= start + 6 && bytes[i - 1] == q && bytes[i - 2] == q && bytes[i - 3] == q) {
                i = len;
            }
            spans.push((start..i, Some("string")));
            continue;
        }

        // 3b. Lifetimes en Rust ('a, 'static) vs literales char ('x', '\n')
        if lang == Language::Rust && b == b'\'' {
            let start = i;
            let after_q = i + 1;
            // Verificar si es literal de carácter: 'x' o '\x' o '\u{...}'
            let is_char_lit = if after_q < len && bytes[after_q] == b'\\' {
                true
            } else if after_q < len {
                let c_end = next_char_end(source, after_q);
                c_end < len && bytes[c_end] == b'\''
            } else {
                false
            };

            if is_char_lit {
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i = next_char_end(source, i + 1);
                    } else if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    } else if bytes[i] == b'\n' {
                        break;
                    } else {
                        i = next_char_end(source, i);
                    }
                }
                spans.push((start..i, Some("string")));
            } else {
                // Lifetime 'a, 'static
                i += 1;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                spans.push((start..i, Some("attribute")));
            }
            continue;
        }

        // 3c. Cadenas de texto normales ("...", '...', `...`)
        if b == b'"' || b == b'\'' || b == b'`' {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i = next_char_end(source, i + 1);
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else if bytes[i] == b'\n' && quote != b'`' {
                    break;
                } else {
                    i = next_char_end(source, i);
                }
            }
            // En JSON, las claves seguidas de ':' se destacan como "property"
            let kind = if lang == Language::Json && quote == b'"' {
                let mut peek = i;
                while peek < len && (bytes[peek] == b' ' || bytes[peek] == b'\t') {
                    peek += 1;
                }
                if peek < len && bytes[peek] == b':' {
                    Some("property")
                } else {
                    Some("string")
                }
            } else {
                Some("string")
            };
            spans.push((start..i, kind));
            continue;
        }

        // 4. Números
        if b.is_ascii_digit() {
            let start = i;
            if b == b'0'
                && i + 1 < len
                && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
            {
                i += 2;
                while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                    i += 1;
                }
            } else {
                while i < len
                    && (bytes[i].is_ascii_digit()
                        || bytes[i] == b'.'
                        || bytes[i] == b'_'
                        || bytes[i] == b'e'
                        || bytes[i] == b'E')
                {
                    i += 1;
                }
            }
            spans.push((start..i, Some("number")));
            continue;
        }

        // 5. Identificadores y palabras clave
        if b.is_ascii_alphabetic()
            || b == b'_'
            || (b == b'$' && (lang == Language::Php || lang == Language::Bash))
        {
            let start = i;
            if b == b'$' {
                i += 1;
            }
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &source[start..i];

            let kind = if b == b'$' && word.len() > 1 {
                Some("variable.parameter")
            } else if keywords.contains(&word) {
                Some("keyword")
            } else if types.contains(&word) {
                Some("type")
            } else {
                let mut peek = i;
                while peek < len && (bytes[peek] == b' ' || bytes[peek] == b'\t') {
                    peek += 1;
                }
                if peek < len && (bytes[peek] == b'(' || (lang == Language::Rust && bytes[peek] == b'!')) {
                    Some("function")
                } else {
                    None
                }
            };
            spans.push((start..i, kind));
            continue;
        }

        // 6. Operadores, signos de puntuación y cualquier carácter UTF-8 multibyte
        let start = i;
        let next = next_char_end(source, i);
        i = next;
        let kind = if b.is_ascii_punctuation() {
            Some("operator")
        } else {
            None
        };
        spans.push((start..i, kind));
    }

    if spans.is_empty() {
        spans.push((0..source.len(), None));
    }

    spans
}

fn lex_markdown(source: &str) -> Vec<(Range<usize>, Option<&'static str>)> {
    let mut spans = Vec::new();
    let mut offset = 0usize;
    let mut in_fence = false;

    for raw_line in source.split_inclusive('\n') {
        let line_len = raw_line.len();
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            spans.push((offset..offset + line_len, Some("comment")));
        } else if in_fence {
            spans.push((offset..offset + line_len, Some("string")));
        } else if trimmed.starts_with('#') {
            spans.push((offset..offset + line_len, Some("keyword")));
        } else if trimmed.starts_with('>') {
            spans.push((offset..offset + line_len, Some("comment")));
        } else {
            // Resaltar inline code `...` y negritas **...** dentro de la línea
            let bytes = raw_line.as_bytes();
            let mut i = 0;
            let mut plain_start = 0;
            while i < line_len {
                if bytes[i] == b'`' {
                    if plain_start < i {
                        spans.push((offset + plain_start..offset + i, None));
                    }
                    let start = i;
                    i += 1;
                    while i < line_len && bytes[i] != b'`' && bytes[i] != b'\n' {
                        i = next_char_end(raw_line, i);
                    }
                    if i < line_len && bytes[i] == b'`' {
                        i += 1;
                    }
                    spans.push((offset + start..offset + i, Some("string")));
                    plain_start = i;
                } else {
                    i = next_char_end(raw_line, i);
                }
            }
            if plain_start < line_len {
                spans.push((offset + plain_start..offset + line_len, None));
            }
        }
        offset += line_len;
    }
    if spans.is_empty() {
        spans.push((0..source.len(), None));
    }
    spans
}

fn lex_xml(source: &str) -> Vec<(Range<usize>, Option<&'static str>)> {
    let mut spans = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' && i + 3 < len && &bytes[i..i + 4] == b"<!--" {
            let start = i;
            i += 4;
            while i + 2 < len && &bytes[i..i + 3] != b"-->" {
                i = next_char_end(source, i);
            }
            i = if i + 2 < len { i + 3 } else { len };
            spans.push((start..i, Some("comment")));
        } else if bytes[i] == b'<' {
            let start = i;
            i += 1;
            if i < len && (bytes[i] == b'/' || bytes[i] == b'?' || bytes[i] == b'!') {
                i += 1;
            }
            while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' && bytes[i] != b'/' {
                i = next_char_end(source, i);
            }
            spans.push((start..i, Some("tag")));
            // Atributos dentro de la etiqueta
            while i < len && bytes[i] != b'>' {
                if bytes[i].is_ascii_whitespace() {
                    let ws = i;
                    while i < len && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    spans.push((ws..i, None));
                } else if bytes[i] == b'"' || bytes[i] == b'\'' {
                    let q = bytes[i];
                    let s = i;
                    i += 1;
                    while i < len && bytes[i] != q && bytes[i] != b'\n' {
                        i = next_char_end(source, i);
                    }
                    if i < len && bytes[i] == q {
                        i += 1;
                    }
                    spans.push((s..i, Some("string")));
                } else if bytes[i] == b'=' || bytes[i] == b'/' || bytes[i] == b'?' {
                    let s = i;
                    i += 1;
                    spans.push((s..i, Some("operator")));
                } else {
                    let s = i;
                    while i < len
                        && !bytes[i].is_ascii_whitespace()
                        && !matches!(bytes[i], b'=' | b'>' | b'/' | b'?' | b'"' | b'\'')
                    {
                        i = next_char_end(source, i);
                    }
                    spans.push((s..i, Some("attribute")));
                }
            }
            if i < len && bytes[i] == b'>' {
                spans.push((i..i + 1, Some("tag")));
                i += 1;
            }
        } else {
            let start = i;
            while i < len && bytes[i] != b'<' {
                i = next_char_end(source, i);
            }
            spans.push((start..i, None));
        }
    }
    if spans.is_empty() {
        spans.push((0..source.len(), None));
    }
    spans
}

fn keywords_and_types_for(lang: Language) -> (&'static [&'static str], &'static [&'static str]) {
    match lang {
        Language::Python => (
            &[
                "def", "class", "import", "from", "as", "return", "if", "elif", "else", "while",
                "for", "in", "try", "except", "finally", "with", "pass", "break", "continue",
                "lambda", "yield", "raise", "async", "await", "assert", "global", "nonlocal",
                "not", "and", "or", "is", "match", "case", "del",
            ],
            &[
                "int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "self",
                "cls", "True", "False", "None", "object", "Exception",
            ],
        ),
        Language::Rust => (
            &[
                "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "for", "in",
                "while", "loop", "match", "if", "else", "return", "break", "continue", "use",
                "mod", "crate", "as", "const", "static", "type", "where", "unsafe", "async",
                "await", "move", "ref", "super", "extern", "dyn",
            ],
            &[
                "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128",
                "usize", "f32", "f64", "bool", "char", "str", "String", "Vec", "Option", "Result",
                "Some", "None", "Ok", "Err", "self", "Self", "true", "false", "Box", "Arc", "Rc",
            ],
        ),
        Language::Cpp => (
            &[
                "auto", "break", "case", "catch", "class", "const", "constexpr", "consteval",
                "continue", "default", "delete", "do", "else", "enum", "explicit", "extern",
                "for", "friend", "goto", "if", "inline", "namespace", "new", "noexcept",
                "nullptr", "operator", "override", "private", "protected", "public", "return",
                "sizeof", "static", "struct", "switch", "template", "this", "throw", "try",
                "typedef", "typename", "union", "using", "virtual", "volatile", "while",
            ],
            &[
                "int", "char", "float", "double", "bool", "void", "short", "long", "unsigned",
                "signed", "size_t", "int32_t", "int64_t", "uint32_t", "uint64_t", "string",
                "vector", "map", "set", "std", "true", "false", "NULL",
            ],
        ),
        Language::CSharp => (
            &[
                "using", "namespace", "class", "struct", "interface", "enum", "record", "public",
                "private", "protected", "internal", "static", "readonly", "const", "void",
                "return", "if", "else", "switch", "case", "default", "for", "foreach", "in",
                "while", "do", "break", "continue", "new", "var", "async", "await", "get", "set",
                "try", "catch", "finally", "throw", "override", "virtual", "sealed", "abstract",
                "out", "ref", "this", "base", "delegate", "event",
            ],
            &[
                "int", "long", "short", "byte", "string", "char", "bool", "double", "float",
                "decimal", "object", "dynamic", "null", "true", "false", "Task", "List",
                "Dictionary", "Console",
            ],
        ),
        Language::Php => (
            &[
                "function", "class", "interface", "trait", "extends", "implements", "public",
                "private", "protected", "static", "const", "return", "if", "else", "elseif",
                "switch", "case", "default", "match", "for", "foreach", "as", "while", "do",
                "break", "continue", "new", "echo", "print", "include", "include_once", "require",
                "require_once", "namespace", "use", "try", "catch", "finally", "throw", "declare",
                "fn", "readonly", "isset", "empty", "unset",
            ],
            &[
                "int", "float", "string", "bool", "array", "object", "mixed", "void", "never",
                "null", "true", "false", "self", "parent", "PDO", "PDOException", "DateTime",
            ],
        ),
        Language::Go => (
            &[
                "func", "package", "import", "var", "const", "type", "struct", "interface",
                "return", "if", "else", "for", "range", "switch", "case", "default", "break",
                "continue", "go", "defer", "chan", "map", "select", "fallthrough",
            ],
            &[
                "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32",
                "uint64", "byte", "rune", "string", "bool", "float32", "float64", "error", "any",
                "nil", "true", "false",
            ],
        ),
        Language::Bash => (
            &[
                "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until",
                "case", "esac", "function", "return", "exit", "export", "echo", "local", "set",
                "unset", "source", "shift", "break", "continue",
            ],
            &["true", "false"],
        ),
        Language::Json => (&[], &["true", "false", "null"]),
        _ => (&[], &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_full_valid_coverage(lang: Language, source: &str) {
        let mut hl = SyntaxHighlighter::new(lang);
        let spans = hl.highlight(source);
        assert!(!spans.is_empty(), "Debe producir al menos un span para {lang:?}");
        let mut cursor = 0usize;
        for (range, _) in &spans {
            assert!(source.is_char_boundary(range.start), "start inválido en {lang:?}: {range:?}");
            assert!(source.is_char_boundary(range.end), "end inválido en {lang:?}: {range:?}");
            assert_eq!(range.start, cursor, "hueco o solapamiento en {lang:?}: esperado {cursor}, obtenido {range:?}");
            cursor = range.end;
        }
        assert_eq!(cursor, source.len(), "cobertura incompleta en {lang:?}");
    }

    #[test]
    fn test_utf8_multibyte_safety_across_languages() {
        let samples = [
            (Language::Python, "# ¡Hola año nuevo! 🚀\n@decorador\ndef función_español(ñ = \"café ☕\"):\n    \"\"\"doc ñ\"\"\"\n    return 42\n"),
            (Language::Rust, "fn saludo<'a>(s: &'a str) -> char { // ñandú 🦀\n    let c = 'ñ';\n    \"acción\"\n}\n"),
            (Language::Cpp, "namespace español { class Árbol { public: int* p = nullptr; }; }\n"),
            (Language::Json, "{\"título\": \"AmoxliCode 🚀\", \"activo\": true, \"n\": 123}\n"),
            (Language::Markdown, "# Título con ñ\nTexto con `código` y emoji 🔥\n```rust\nlet x = 1;\n```\n"),
            (Language::Xml, "<!-- comentario ñ --><etiqueta attr=\"café\">¡Hola!</etiqueta>\n"),
            (Language::C, "/* comentario áéíóú */\nint main() { return 0; }\n"),
            (Language::PlainText, "Texto simple con ñ, áéíóú y emojis 🎉\n"),
        ];
        for (lang, src) in samples {
            assert_full_valid_coverage(lang, src);
        }
    }

    #[test]
    fn test_cpp_and_rust_specific_tokens() {
        let mut cpp = SyntaxHighlighter::new(Language::Cpp);
        let cpp_src = "class Demo { public: nullptr_t x = nullptr; };";
        let cpp_spans = cpp.highlight(cpp_src);
        assert!(cpp_spans.iter().any(|(r, k)| &cpp_src[r.clone()] == "class" && *k == Some("keyword")));
        assert!(cpp_spans.iter().any(|(r, k)| &cpp_src[r.clone()] == "nullptr" && *k == Some("keyword")));

        let mut rust = SyntaxHighlighter::new(Language::Rust);
        let rust_src = "fn f<'a>(x: &'a str) -> char { 'z' }";
        let rust_spans = rust.highlight(rust_src);
        // El lifetime 'a no debe tragarse el resto de la línea como string sin cerrar.
        assert!(rust_spans.iter().any(|(r, k)| &rust_src[r.clone()] == "str" && *k == Some("type")));
        assert!(rust_spans.iter().any(|(r, k)| &rust_src[r.clone()] == "'z'" && *k == Some("string")));
    }
}

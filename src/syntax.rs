//! AmoxliCode - Fase 5: Resaltado de sintaxis y soporte de lenguajes
//!
//! Soporta los lenguajes de programación más populares:
//! - Texto plano (.txt, predeterminado)
//! - Python (.py)
//! - Rust (.rs)
//! - C (.c)
//! - C++ (.cpp)
//! - Java (.java)
//! - JavaScript (.js)
//! - TypeScript (.ts)
//! - HTML (.html)
//! - CSS (.css)
//! - SQL (.sql)
//! - JSON (.json)
//! - Markdown (.md)
//! - C# (.cs)
//! - PHP (.php)
//! - Go (.go)
//! - Bash / Shell (.sh)
//! - XML (.xml)
//!
//! Utiliza tree-sitter para las gramáticas nativas compiladas y un analizador léxico
//! de alto rendimiento para el resto de lenguajes populares.

use std::collections::HashSet;
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
    /// Lista ordenada de los lenguajes más populares para la barra y menús tipo Notepad++.
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

    /// Nombre amigable completo para menús y selectores.
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

    /// Nombre corto para la barra de estado y pestañas.
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

    /// Extensión típica asociada al lenguaje (sin punto).
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

    /// Identificador estándar del lenguaje según la especificación LSP.
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

    /// Adivina el lenguaje a partir de la extensión del archivo. Si no la reconoce,
    /// usa Texto plano (.txt) por defecto.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "txt" | "text" | "log" => Language::PlainText,
            "py" | "pyw" => Language::Python,
            "rs" => Language::Rust,
            "c" | "h" => Language::C,
            "cpp" | "cxx" | "cc" | "hpp" => Language::Cpp,
            "java" => Language::Java,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "ts" | "tsx" => Language::TypeScript,
            "html" | "htm" => Language::Html,
            "css" => Language::Css,
            "sql" => Language::Sql,
            "json" => Language::Json,
            "md" | "markdown" => Language::Markdown,
            "cs" => Language::CSharp,
            "php" => Language::Php,
            "go" => Language::Go,
            "sh" | "bash" => Language::Bash,
            "xml" => Language::Xml,
            _ => Language::PlainText,
        }
    }

    /// Comando y argumentos recomendados para lanzar el servidor LSP.
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
        config: HighlightConfiguration,
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
            Language::Cpp => Self::init_tree_sitter(
                tree_sitter_c::LANGUAGE.into(),
                "c",
                tree_sitter_c::HIGHLIGHT_QUERY,
                "",
                "",
            ),
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
        let mut config = HighlightConfiguration::new(
            language,
            name,
            highlights_query,
            injections_query,
            locals_query,
        )
        .expect("no se pudo cargar la gramática para resaltado");

        config.configure(HIGHLIGHT_NAMES);

        SyntaxHighlighter::TreeSitter {
            highlighter: Highlighter::new(),
            config,
        }
    }

    /// Analiza `source` y devuelve una lista de tramos (rango de bytes +
    /// nombre de highlight opcional) que cubre TODO el texto sin huecos.
    pub fn highlight(&mut self, source: &str) -> Vec<(Range<usize>, Option<&'static str>)> {
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

                for event in events {
                    match event {
                        Ok(HighlightEvent::Source { start, end }) if start < end => {
                            spans.push((start..end, stack.last().copied()));
                        }
                        Ok(HighlightEvent::Source { .. }) => {}
                        Ok(HighlightEvent::HighlightStart(h)) => stack.push(HIGHLIGHT_NAMES[h.0]),
                        Ok(HighlightEvent::HighlightEnd) => {
                            stack.pop();
                        }
                        Err(_) => break,
                    }
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

/// Analizador léxico rápido para lenguajes populares sin gramática nativa tree-sitter.
fn lex_highlight(source: &str, lang: Language) -> Vec<(Range<usize>, Option<&'static str>)> {
    if source.is_empty() {
        return Vec::new();
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
        // // o /* */
        if (b == b'/' && i + 1 < len && (bytes[i + 1] == b'/' || bytes[i + 1] == b'*'))
            || (b == b'#' && (lang == Language::Python || lang == Language::Bash))
            || (b == b'<' && i + 3 < len && &bytes[i..i + 4] == b"<!--")
        {
            let start = i;
            if b == b'#' || (b == b'/' && bytes[i + 1] == b'/') {
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            } else if b == b'<' {
                i += 4;
                while i + 2 < len && &bytes[i..i + 3] != b"-->" {
                    i += 1;
                }
                if i + 2 < len {
                    i += 3;
                } else {
                    i = len;
                }
            } else {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                if i + 1 < len {
                    i += 2;
                } else {
                    i = len;
                }
            }
            spans.push((start..i, Some("comment")));
            continue;
        }

        // 3. Cadenas de texto
        if b == b'"' || b == b'\'' || b == b'`' {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else if bytes[i] == b'\n' && quote != b'`' {
                    break;
                } else {
                    i += 1;
                }
            }
            spans.push((start..i, Some("string")));
            continue;
        }

        // 4. Números
        if b.is_ascii_digit() {
            let start = i;
            if b == b'0' && i + 1 < len && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X' || bytes[i + 1] == b'b' || bytes[i + 1] == b'o') {
                i += 2;
                while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                    i += 1;
                }
            } else {
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_' || bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                }
            }
            spans.push((start..i, Some("number")));
            continue;
        }

        // 5. Identificadores y palabras clave
        if b.is_ascii_alphabetic() || b == b'_' || (b == b'$' && (lang == Language::Php || lang == Language::Bash)) {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &source[start..i];

            let kind = if keywords.contains(word) {
                Some("keyword")
            } else if types.contains(word) {
                Some("type")
            } else {
                // Si va seguido de '(' es una función
                let mut peek = i;
                while peek < len && (bytes[peek] == b' ' || bytes[peek] == b'\t') {
                    peek += 1;
                }
                if peek < len && bytes[peek] == b'(' {
                    Some("function")
                } else {
                    None
                }
            };
            spans.push((start..i, kind));
            continue;
        }

        // 6. Operadores y signos de puntuación
        let start = i;
        i += 1;
        spans.push((start..i, Some("operator")));
    }

    if spans.is_empty() {
        spans.push((0..source.len(), None));
    }

    spans
}

fn keywords_and_types_for(lang: Language) -> (HashSet<&'static str>, HashSet<&'static str>) {
    let mut kw = HashSet::new();
    let mut ty = HashSet::new();

    match lang {
        Language::Python => {
            for k in &[
                "def", "class", "import", "from", "as", "return", "if", "elif", "else", "while",
                "for", "in", "try", "except", "finally", "with", "pass", "break", "continue",
                "lambda", "yield", "raise", "async", "await", "assert", "global", "nonlocal", "not", "and", "or", "is",
            ] {
                kw.insert(*k);
            }
            for t in &["int", "float", "str", "bool", "list", "dict", "set", "tuple", "self", "cls", "True", "False", "None"] {
                ty.insert(*t);
            }
        }
        Language::Rust => {
            for k in &[
                "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "for", "in", "while",
                "loop", "match", "if", "else", "return", "break", "continue", "use", "mod",
                "crate", "as", "const", "static", "type", "where", "unsafe", "async", "await", "move", "ref",
            ] {
                kw.insert(*k);
            }
            for t in &[
                "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128",
                "usize", "f32", "f64", "bool", "char", "str", "String", "Vec", "Option", "Result",
                "Some", "None", "Ok", "Err", "self", "Self", "true", "false",
            ] {
                ty.insert(*t);
            }
        }
        Language::Cpp => {
            for k in &[
                "auto", "break", "case", "class", "const", "continue", "default", "delete", "do",
                "else", "enum", "for", "if", "new", "operator", "private", "protected", "public",
                "return", "sizeof", "static", "struct", "switch", "template", "this", "throw",
                "try", "typedef", "typename", "virtual", "while", "constexpr", "nullptr", "namespace", "using",
            ] {
                kw.insert(*k);
            }
            for t in &["int", "char", "float", "double", "bool", "void", "size_t", "string", "vector", "true", "false"] {
                ty.insert(*t);
            }
        }
        Language::CSharp => {
            for k in &[
                "using", "namespace", "class", "struct", "interface", "enum", "public", "private",
                "protected", "internal", "static", "void", "return", "if", "else", "for", "foreach",
                "while", "new", "var", "async", "await", "get", "set",
            ] {
                kw.insert(*k);
            }
            for t in &["int", "string", "bool", "double", "float", "object", "null", "true", "false"] {
                ty.insert(*t);
            }
        }
        Language::Php => {
            for k in &[
                "function", "class", "public", "private", "protected", "return", "if", "else",
                "elseif", "foreach", "while", "new", "echo", "include", "require", "namespace", "use",
            ] {
                kw.insert(*k);
            }
            for t in &["int", "string", "bool", "array", "null", "true", "false"] {
                ty.insert(*t);
            }
        }
        Language::Go => {
            for k in &[
                "func", "package", "import", "var", "const", "type", "struct", "interface", "return",
                "if", "else", "for", "range", "switch", "case", "default", "go", "defer", "chan",
                "map", "select",
            ] {
                kw.insert(*k);
            }
            for t in &["int", "string", "bool", "float64", "error", "nil", "true", "false"] {
                ty.insert(*t);
            }
        }
        Language::Bash => {
            for k in &[
                "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until",
                "case", "esac", "function", "return", "exit", "export", "echo", "local",
            ] {
                kw.insert(*k);
            }
        }
        Language::Json => {
            for t in &["true", "false", "null"] {
                ty.insert(*t);
            }
        }
        _ => {}
    }

    (kw, ty)
}
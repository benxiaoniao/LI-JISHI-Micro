//! `micc lsp`：语言服务器（LSP over stdio，JSON-RPC 2.0）。
//!
//! 诊断/补全/定义/格式化都从这里出，VS Code 扩展只做客户端。
//! 不引第三方 crate：报文用 `json.rs`，传输用 Content-Length 头。
//!
//! 支持：
//! - `initialize` / `initialized` / `shutdown` / `exit`
//! - `textDocument/didOpen|didChange|didSave|didClose` → 编译诊断
//! - `textDocument/completion` → 关键字 / 内建 / 标准库模块
//! - `textDocument/definition` → 本文件、依赖包入口、原生模块的语言卡声明
//! - `textDocument/formatting` → `format::format_source`

use crate::format;
use crate::json::{self, obj, s, Json};
use crate::langcard;
use crate::lexer;
use crate::native_spec;
use crate::stdlib_spec;
use crate::symbols;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

pub fn run() -> i32 {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut server = Server {
        docs: HashMap::new(),
        roots: Vec::new(),
        langcard: None,
        shutdown: false,
    };
    loop {
        let Some(body) = read_message(&mut reader) else {
            return 0;
        };
        let Ok(msg) = json::parse(&body) else {
            continue;
        };
        let method = msg.string("method").unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(Json::Null);
        if let Some(id) = msg.get("id") {
            let result = server.request(&method, &params);
            match result {
                Ok(value) => send(&obj(vec![
                    ("jsonrpc", s("2.0")),
                    ("id", id.clone()),
                    ("result", value),
                ])),
                Err(err) => send(&obj(vec![
                    ("jsonrpc", s("2.0")),
                    ("id", id.clone()),
                    (
                        "error",
                        obj(vec![("code", json::n(-32603.0)), ("message", s(err))]),
                    ),
                ])),
            }
            continue;
        }
        server.notification(&method, &params);
        if method == "exit" {
            return if server.shutdown { 0 } else { 1 };
        }
    }
}

struct Server {
    /// uri → 当前缓冲区文本（didOpen/didChange 维护）。
    docs: HashMap<String, String>,
    roots: Vec<PathBuf>,
    /// 客户端指定的语言卡路径（`initializationOptions.langcardPath`）。
    langcard: Option<PathBuf>,
    shutdown: bool,
}

impl Server {
    fn request(&mut self, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "initialize" => {
                if let Some(folders) = params.get("workspaceFolders").and_then(Json::as_arr) {
                    for f in folders {
                        if let Some(uri) = f.string("uri") {
                            if let Some(p) = uri_to_path(&uri) {
                                self.roots.push(p);
                            }
                        }
                    }
                } else if let Some(uri) = params.string("rootUri") {
                    if let Some(p) = uri_to_path(&uri) {
                        self.roots.push(p);
                    }
                }
                if let Some(path) = params
                    .get("initializationOptions")
                    .and_then(|o| o.string("langcardPath"))
                {
                    self.langcard = Some(PathBuf::from(path));
                }
                Ok(obj(vec![
                    (
                        "capabilities",
                        obj(vec![
                            ("textDocumentSync", json::n(1.0)),
                            (
                                "completionProvider",
                                obj(vec![("triggerCharacters", Json::Arr(vec![]))]),
                            ),
                            ("definitionProvider", Json::Bool(true)),
                            ("documentFormattingProvider", Json::Bool(true)),
                        ]),
                    ),
                    (
                        "serverInfo",
                        obj(vec![
                            ("name", s("micc-lsp")),
                            ("version", s(env!("CARGO_PKG_VERSION"))),
                        ]),
                    ),
                ]))
            }
            "shutdown" => {
                self.shutdown = true;
                Ok(Json::Null)
            }
            "textDocument/completion" => self.completion(params),
            "textDocument/definition" => self.definition(params),
            "textDocument/formatting" => self.formatting(params),
            other => Err(format!("不支持的方法：{}", other)),
        }
    }

    fn notification(&mut self, method: &str, params: &Json) {
        match method {
            "initialized" => {}
            "textDocument/didOpen" => {
                if let Some(doc) = params.get("textDocument") {
                    if let (Some(uri), Some(text)) = (doc.string("uri"), doc.string("text")) {
                        self.docs.insert(uri.clone(), text);
                        self.publish(&uri);
                    }
                }
            }
            "textDocument/didChange" => {
                if let Some(doc) = params.get("textDocument") {
                    if let Some(uri) = doc.string("uri") {
                        if let Some(changes) = params.get("contentChanges").and_then(Json::as_arr) {
                            if let Some(last) = changes.last() {
                                if let Some(text) = last.string("text") {
                                    self.docs.insert(uri.clone(), text);
                                    self.publish(&uri);
                                }
                            }
                        }
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(doc) = params.get("textDocument") {
                    if let Some(uri) = doc.string("uri") {
                        if !self.docs.contains_key(&uri) {
                            if let Some(path) = uri_to_path(&uri) {
                                if let Ok(text) = std::fs::read_to_string(&path) {
                                    self.docs.insert(uri.clone(), text);
                                }
                            }
                        }
                        self.publish(&uri);
                    }
                }
            }
            "textDocument/didClose" => {
                if let Some(doc) = params.get("textDocument") {
                    if let Some(uri) = doc.string("uri") {
                        self.docs.remove(&uri);
                        send(&diagnostics_notification(&uri, Vec::new()));
                    }
                }
            }
            _ => {}
        }
    }

    fn text(&self, uri: &str) -> Option<String> {
        if let Some(text) = self.docs.get(uri) {
            return Some(text.clone());
        }
        uri_to_path(uri).and_then(|p| std::fs::read_to_string(p).ok())
    }

    /// 单文件编译，把首个错误发成一条诊断（与扩展原先的行为一致）。
    fn publish(&self, uri: &str) {
        let Some(text) = self.text(uri) else {
            return;
        };
        let display = uri_to_path(uri)
            .map(|p| crate::pack::display_path(&p))
            .unwrap_or_else(|| uri.to_string());
        let mut diags = Vec::new();
        if let Err(e) = crate::compile_source_opts(&text, &[display], false, false, false) {
            let line = e.line.saturating_sub(1) as f64;
            let col = e.col.saturating_sub(1) as f64;
            let mut message = e.msg.clone();
            if let Some(hint) = &e.hint {
                message.push_str("\n提示：");
                message.push_str(hint);
            }
            let start = obj(vec![
                ("line", json::n(line)),
                ("character", json::n(col)),
            ]);
            // 占一个字符宽，避免零宽高亮。
            let end = obj(vec![
                ("line", json::n(line)),
                ("character", json::n(col + 1.0)),
            ]);
            diags.push(obj(vec![
                ("range", obj(vec![("start", start), ("end", end)])),
                ("severity", json::n(1.0)),
                ("source", s("micc")),
                ("code", s(e.code)),
                ("message", s(message)),
            ]));
        }
        send(&diagnostics_notification(uri, diags));
    }

    fn completion(&mut self, params: &Json) -> Result<Json, String> {
        let uri = params
            .get("textDocument")
            .and_then(|d| d.string("uri"))
            .unwrap_or_default();
        let text = self.text(&uri).unwrap_or_default();
        let line = params
            .get("position")
            .and_then(|p| p.get("line"))
            .and_then(Json::as_i64)
            .unwrap_or(0)
            .max(0) as usize;
        let character = params
            .get("position")
            .and_then(|p| p.get("character"))
            .and_then(Json::as_i64)
            .unwrap_or(0)
            .max(0) as usize;

        let mut items = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let push = |items: &mut Vec<Json>, seen: &mut Vec<String>, label: &str, kind: f64, detail: String, doc: Option<String>| {
            if seen.iter().any(|x| x == label) {
                return;
            }
            seen.push(label.to_string());
            let mut fields = vec![
                ("label", s(label)),
                ("kind", json::n(kind)),
                ("detail", s(detail)),
                ("insertText", s(label)),
            ];
            if let Some(value) = doc {
                fields.push((
                    "documentation",
                    obj(vec![("kind", s("markdown")), ("value", s(value))]),
                ));
            }
            items.push(obj(fields));
        };

        let card = uri_to_path(&uri)
            .as_deref()
            .and_then(|p| load_card(p, self.langcard.as_deref()));
        if let Some(card) = card {
            let prefix = line_prefix(&text, line, character);
            if prefix.trim_start().starts_with("导入") {
                for m in &card.modules {
                    let mut doc = format!("模块 `{}`", m.name);
                    for (name, entry_doc) in &m.entries {
                        doc.push_str(&format!("\n- `{}` {}", name, entry_doc));
                    }
                    push(
                        &mut items,
                        &mut seen,
                        &m.name,
                        9.0,
                        "砾原生模块".to_string(),
                        Some(doc),
                    );
                }
                return Ok(completion_result(items));
            }
            let imported = imported_modules(&text);
            for (word, supported) in &card.keywords {
                if !supported {
                    continue;
                }
                push(&mut items, &mut seen, word, 14.0, "关键字".to_string(), None);
            }
            for b in &card.builtins {
                push(
                    &mut items,
                    &mut seen,
                    &b.name,
                    3.0,
                    format!("内建函数（{}–{} 个参数）", b.min_args, b.max_args),
                    if b.doc.is_empty() { None } else { Some(b.doc.clone()) },
                );
            }
            for m in &card.modules {
                if !imported.iter().any(|x| x == &m.name) {
                    continue;
                }
                for (name, entry_doc) in &m.entries {
                    push(
                        &mut items,
                        &mut seen,
                        name,
                        3.0,
                        format!("原生函数 · {}", m.name),
                        if entry_doc.is_empty() {
                            None
                        } else {
                            Some(entry_doc.clone())
                        },
                    );
                }
            }
            return Ok(completion_result(items));
        }

        // 没有语言卡时退回编译期内置清单，保证补全不空。
        for (word, kw) in lexer::keyword_table() {
            if matches!(kw, lexer::Kw::Unsupported(_)) {
                continue;
            }
            push(&mut items, &mut seen, word, 14.0, "关键字".to_string(), None);
        }
        for b in langcard::BUILTIN_DOCS {
            push(
                &mut items,
                &mut seen,
                b.name,
                3.0,
                format!("内建 · {}..{} 个参数", b.min_args, b.max_args),
                Some(b.doc.to_string()),
            );
        }
        for m in stdlib_spec::MODULES {
            push(&mut items, &mut seen, m.name, 9.0, "标准库模块".to_string(), None);
        }
        for m in native_spec::MODULES {
            push(&mut items, &mut seen, m.name, 9.0, "原生模块".to_string(), None);
        }
        Ok(completion_result(items))
    }

    fn formatting(&mut self, params: &Json) -> Result<Json, String> {
        let uri = params
            .get("textDocument")
            .and_then(|d| d.string("uri"))
            .ok_or_else(|| "缺少 textDocument.uri".to_string())?;
        let text = self.text(&uri).ok_or_else(|| "读不了文档".to_string())?;
        let formatted = format::format_source(&text);
        if formatted == text {
            return Ok(Json::Arr(vec![]));
        }
        let (end_line, end_char) = end_position(&text);
        Ok(Json::Arr(vec![obj(vec![
            (
                "range",
                obj(vec![
                    (
                        "start",
                        obj(vec![("line", json::n(0.0)), ("character", json::n(0.0))]),
                    ),
                    (
                        "end",
                        obj(vec![
                            ("line", json::n(end_line)),
                            ("character", json::n(end_char)),
                        ]),
                    ),
                ]),
            ),
            ("newText", s(formatted)),
        ])]))
    }

    /// 跳转：本文件定义 → 依赖包入口 → 语言卡里的原生模块声明。
    fn definition(&mut self, params: &Json) -> Result<Json, String> {
        let uri = params
            .get("textDocument")
            .and_then(|d| d.string("uri"))
            .ok_or_else(|| "缺少 textDocument.uri".to_string())?;
        let Some(pos) = params.get("position") else {
            return Ok(Json::Null);
        };
        let line = pos.get("line").and_then(Json::as_i64).unwrap_or(0).max(0) as usize;
        let character = pos
            .get("character")
            .and_then(Json::as_i64)
            .unwrap_or(0)
            .max(0) as usize;
        let Some(text) = self.text(&uri) else {
            return Ok(Json::Null);
        };
        let Some((word, receiver)) = word_at(&text, line, character) else {
            return Ok(Json::Null);
        };

        let path = uri_to_path(&uri);
        let manifest = path.as_deref().and_then(nearest_manifest);
        let packs = manifest
            .as_deref()
            .and_then(|dir| symbols::pack_files(dir).ok())
            .unwrap_or_default();

        // 本文件定义（未保存缓冲区也走这份文本）
        let local = symbols::source_symbols(&text).unwrap_or_default();
        let local_hit = if let Some(recv) = &receiver {
            let pkg = packs.iter().find(|(name, _, _)| name == recv);
            if let Some((_, _, entry)) = pkg {
                let remote = symbols::file_symbols(Path::new(entry)).unwrap_or_default();
                remote
                    .into_iter()
                    .find(|sym| sym.name == word && matches!(sym.kind, "函数" | "令" | "类"))
                    .map(|sym| (path_to_uri(Path::new(entry)), sym))
            } else {
                local
                    .iter()
                    .find(|sym| sym.name == word && matches!(sym.kind, "方法" | "字段"))
                    .cloned()
                    .map(|sym| (uri.clone(), sym))
            }
        } else {
            local
                .iter()
                .find(|sym| {
                    sym.name == word && matches!(sym.kind, "令" | "函数" | "类" | "方法" | "字段")
                })
                .cloned()
                .map(|sym| (uri.clone(), sym))
        };
        if let Some((target_uri, sym)) = local_hit {
            return Ok(location(&target_uri, sym.line, sym.col));
        }

        if receiver.is_none() {
            // 裸包名 → 依赖入口第 1 行
            if let Some((_, _, entry)) = packs.iter().find(|(name, _, _)| *name == word) {
                return Ok(location(&path_to_uri(Path::new(entry)), 1, 1));
            }
            // `导入 模块` → 语言卡里的模块声明
            if is_import_line(&text, line, &word) {
                if let Some(p) = path.as_deref() {
                    if let Some((file, line_no)) =
                        langcard_module(p, &word, self.langcard.as_deref())
                    {
                        return Ok(location(&path_to_uri(&file), line_no, 1));
                    }
                }
            }
        }
        Ok(Json::Null)
    }
}

/* ---------------- 传输 ---------------- */

fn read_message(reader: &mut BufReader<std::io::StdinLock<'_>>) -> Option<String> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok();
            }
        }
    }
    let mut buf = vec![0u8; length?];
    reader.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn send(value: &Json) {
    let body = json::stringify(value);
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

fn diagnostics_notification(uri: &str, diagnostics: Vec<Json>) -> Json {
    obj(vec![
        ("jsonrpc", s("2.0")),
        ("method", s("textDocument/publishDiagnostics")),
        (
            "params",
            obj(vec![("uri", s(uri)), ("diagnostics", Json::Arr(diagnostics))]),
        ),
    ])
}

/* ---------------- 位置与 URI ---------------- */

fn end_position(text: &str) -> (f64, f64) {
    let mut line = 0usize;
    let mut last = 0usize;
    for c in text.chars() {
        if c == '\n' {
            line += 1;
            last = 0;
        } else {
            last += c.len_utf16();
        }
    }
    (line as f64, last as f64)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 光标处标识符与 `接收者.` 前缀。
fn word_at(text: &str, line: usize, character: usize) -> Option<(String, Option<String>)> {
    let line_text: &str = text.split('\n').nth(line)?;
    let line_text = line_text.strip_suffix('\r').unwrap_or(line_text);
    let chars: Vec<char> = line_text.chars().collect();
    // UTF-16 列 → 字符下标
    let mut utf16 = 0usize;
    let mut at = chars.len();
    for (i, c) in chars.iter().enumerate() {
        if utf16 >= character {
            at = i;
            break;
        }
        utf16 += c.len_utf16();
    }
    let mut start = at.min(chars.len());
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let word: String = chars[start..end].iter().collect();
    let receiver = if start > 0 && chars[start - 1] == '.' {
        let r_end = start - 1;
        let mut r_start = r_end;
        while r_start > 0 && is_word(chars[r_start - 1]) {
            r_start -= 1;
        }
        if r_start == r_end {
            None
        } else {
            Some(chars[r_start..r_end].iter().collect())
        }
    } else {
        None
    };
    Some((word, receiver))
}

fn is_import_line(text: &str, line: usize, word: &str) -> bool {
    let Some(line_text) = text.split('\n').nth(line) else {
        return false;
    };
    let t = line_text.trim();
    t.starts_with("导入") && t[("导入".len())..].trim_start().starts_with(word)
}

fn location(uri: &str, line: u32, col: u32) -> Json {
    let l = line.saturating_sub(1) as f64;
    let c = col.saturating_sub(1) as f64;
    obj(vec![
        ("uri", s(uri)),
        (
            "range",
            obj(vec![
                (
                    "start",
                    obj(vec![("line", json::n(l)), ("character", json::n(c))]),
                ),
                (
                    "end",
                    obj(vec![("line", json::n(l)), ("character", json::n(c + 1.0))]),
                ),
            ]),
        ),
    ])
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let decoded = percent_decode(rest);
    Some(PathBuf::from(decoded))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_string();
    let mut out = String::from("file:///");
    for b in text.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || "/:.-_~".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

struct CardBuiltin {
    name: String,
    min_args: u8,
    max_args: u8,
    doc: String,
}

struct CardModule {
    name: String,
    entries: Vec<(String, String)>,
}

struct Card {
    keywords: Vec<(String, bool)>,
    builtins: Vec<CardBuiltin>,
    modules: Vec<CardModule>,
}

fn completion_result(items: Vec<Json>) -> Json {
    obj(vec![
        ("isIncomplete", Json::Bool(false)),
        ("items", Json::Arr(items)),
    ])
}

/// 从文档所在目录向上找 `spec/langcard.json` 并解析出补全需要的几段。
/// 客户端给了 `langcardPath` 就优先用它。
fn load_card(doc: &Path, explicit: Option<&Path>) -> Option<Card> {
    if let Some(path) = explicit {
        if let Some(card) = parse_card(path) {
            return Some(card);
        }
    }
    let mut dir = doc.parent()?;
    loop {
        let candidate = dir.join("spec").join("langcard.json");
        if candidate.is_file() {
            return parse_card(&candidate);
        }
        dir = dir.parent()?;
    }
}

fn parse_card(path: &Path) -> Option<Card> {
            let text = std::fs::read_to_string(path).ok()?;
            let value = json::parse(&text).ok()?;
            let mut card = Card {
                keywords: Vec::new(),
                builtins: Vec::new(),
                modules: Vec::new(),
            };
            if let Some(items) = value.get("keywords").and_then(Json::as_arr) {
                for item in items {
                    if let Some(word) = item.string("word") {
                        let supported = item
                            .get("supported")
                            .and_then(|v| match v {
                                Json::Bool(b) => Some(*b),
                                _ => None,
                            })
                            .unwrap_or(true);
                        card.keywords.push((word, supported));
                    }
                }
            }
            if let Some(items) = value.get("builtins").and_then(Json::as_arr) {
                for item in items {
                    if let Some(name) = item.string("name") {
                        card.builtins.push(CardBuiltin {
                            name,
                            min_args: item
                                .get("min_args")
                                .and_then(Json::as_i64)
                                .unwrap_or(0) as u8,
                            max_args: item
                                .get("max_args")
                                .and_then(Json::as_i64)
                                .unwrap_or(0) as u8,
                            doc: item.string("doc").unwrap_or_default(),
                        });
                    }
                }
            }
            if let Some(items) = value.get("native_modules").and_then(Json::as_arr) {
                for item in items {
                    let Some(name) = item.string("module") else {
                        continue;
                    };
                    let mut entries = Vec::new();
                    for e in item.get("entries").and_then(Json::as_arr).unwrap_or(&[]) {
                        if let Some(en) = e.string("name") {
                            entries.push((en, e.string("doc").unwrap_or_default()));
                        }
                    }
                    card.modules.push(CardModule { name, entries });
                }
            }
            Some(card)
}

fn line_prefix(text: &str, line: usize, character: usize) -> String {
    let Some(line_text) = text.split('\n').nth(line) else {
        return String::new();
    };
    let line_text = line_text.strip_suffix('\r').unwrap_or(line_text);
    let mut out = String::new();
    let mut utf16 = 0usize;
    for c in line_text.chars() {
        if utf16 >= character {
            break;
        }
        out.push(c);
        utf16 += c.len_utf16();
    }
    out
}

/// 文档里 `导入 模块` 出现的模块名（用于补全该模块的原生函数）。
fn imported_modules(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("导入") else {
            continue;
        };
        let name = rest.trim_start().split_whitespace().next().unwrap_or("");
        if !name.is_empty() && !out.iter().any(|x| x == name) {
            out.push(name.to_string());
        }
    }
    out
}

fn nearest_manifest(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.join(crate::pack_manifest::MANIFEST_NAME).is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// 从文档所在目录向上找 `spec/langcard.json`，再定位 `"module": "<名字>"` 的行号（1 起）。
/// 客户端给了 `langcardPath` 就优先用它。
fn langcard_module(
    doc: &Path,
    module: &str,
    explicit: Option<&Path>,
) -> Option<(PathBuf, u32)> {
    if let Some(path) = explicit {
        if let Some(found) = module_line_in(path, module) {
            return Some((path.to_path_buf(), found));
        }
    }
    let mut dir = doc.parent()?;
    loop {
        let candidate = dir.join("spec").join("langcard.json");
        if candidate.is_file() {
            let line = module_line_in(&candidate, module)?;
            return Some((candidate, line));
        }
        dir = dir.parent()?;
    }
}

fn module_line_in(path: &Path, module: &str) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    let needle = format!("\"module\": \"{}\"", module);
    for (i, line) in text.lines().enumerate() {
        if line.contains(&needle) {
            return Some((i + 1) as u32);
        }
    }
    None
}

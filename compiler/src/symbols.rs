//! 编辑器查询接口：`--symbols` 与 `--pack-files`。
//!
//! 只复用现成的词法/语法分析与 `pack_resolve`，不复制编译器逻辑；
//! 输出是制表符分隔的纯文本，供 VS Code 扩展解析。

use crate::ast::{Stmt, StmtKind};
use crate::lexer;
use crate::pack_resolve;
use crate::parser::Parser;
use std::path::Path;

pub struct Symbol {
    pub kind: &'static str,
    pub name: String,
    pub line: u32,
    pub col: u32,
}

fn push(out: &mut Vec<Symbol>, kind: &'static str, name: &str, line: u32, col: u32) {
    out.push(Symbol {
        kind,
        name: name.to_string(),
        line,
        col,
    });
}

fn walk(stmts: &[Stmt], out: &mut Vec<Symbol>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Let(name, _) => push(out, "令", name, s.line, 1),
            StmtKind::FuncDef { name, body, .. } => {
                push(out, "函数", name, s.line, 1);
                walk(body, out);
            }
            StmtKind::Class(c) => {
                push(out, "类", &c.name, s.line, 1);
                for (field, init) in &c.fields {
                    push(out, "字段", field, init.line, 1);
                }
                for m in &c.methods {
                    push(out, "方法", &m.name, m.line, 1);
                    walk(&m.body, out);
                }
            }
            StmtKind::Import(name) => push(out, "导入", name, s.line, 1),
            StmtKind::Enum(e) => {
                push(out, "枚举", &e.name, s.line, 1);
                for m in &e.members {
                    push(out, "成员", m, s.line, 1);
                }
            }
            StmtKind::Match {
                arms, else_body, ..
            } => {
                for (_, body) in arms {
                    walk(body, out);
                }
                if let Some(body) = else_body {
                    walk(body, out);
                }
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for (_, body) in branches {
                    walk(body, out);
                }
                if let Some(body) = else_body {
                    walk(body, out);
                }
            }
            StmtKind::While(_, body) | StmtKind::Repeat(_, body) => walk(body, out),
            StmtKind::ForEach(_, _, body) => walk(body, out),
            StmtKind::Try {
                body, catch_body, ..
            } => {
                walk(body, out);
                walk(catch_body, out);
            }
            _ => {}
        }
    }
}

/// 解析一个 `.jsh`，列出其中所有可跳转的定义（行/列从 1 起）。
pub fn file_symbols(path: &Path) -> Result<Vec<Symbol>, String> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("读不了 {}：{}", path.display(), e))?;
    let src = src.strip_prefix('\u{feff}').unwrap_or(&src);
    let tokens = lexer::tokenize(src).map_err(|e| e.to_string())?;
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    walk(&program, &mut out);
    Ok(out)
}

/// 包目录 → 该包及其依赖的入口文件（依赖在前，根包最后）。
/// 在线解析失败时退回离线锁文件，与 `micc pack` 的行为一致。
pub fn pack_files(dir: &Path) -> Result<Vec<(String, String, String)>, String> {
    let tree = pack_resolve::resolve_online(dir)
        .or_else(|_| pack_resolve::resolve_offline(dir))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for pkg in tree.deps.iter().chain(std::iter::once(&tree.root)) {
        let entry = pkg.dir.join(&pkg.manifest.entry);
        let abs = entry.canonicalize().unwrap_or(entry);
        let text = abs.to_string_lossy().replace('\\', "/");
        let text = text.strip_prefix("//?/").unwrap_or(&text).to_string();
        out.push((
            pkg.manifest.name.clone(),
            format!("{}", pkg.manifest.version),
            text,
        ));
    }
    Ok(out)
}

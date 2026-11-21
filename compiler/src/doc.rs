//! `micc doc`：从源码生成的接口数据 + 语言卡，产出 Markdown / HTML API 文档。
//!
//! 不编译、不解析依赖；数据口径与 `micc interface` 完全一致：
//! 若包目录已有 `接口.toml` 且与源码生成的清单不同，直接报 E146（漂移），
//! 避免把过期的接口说明发到包站。

use crate::error::{CompileError, Result};
use crate::json::{self, Json};
use crate::pack::{self, InterfaceClass, InterfaceData};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Markdown,
    Html,
}

struct DocOpts {
    dir: PathBuf,
    out: Option<PathBuf>,
    format: Option<Format>,
    card: Option<PathBuf>,
}

pub fn cmd_doc(args: &[String]) -> i32 {
    match do_doc(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

fn e125(msg: impl Into<String>) -> CompileError {
    CompileError::new("E125", 1, 1, msg)
}

fn e146(msg: impl Into<String>) -> CompileError {
    CompileError::new("E146", 1, 1, msg)
}

fn parse_opts(args: &[String]) -> Result<DocOpts> {
    let usage = "用法：micc doc [--目录 <包目录>] [-o 输出.md|输出.html] [--格式 md|html] [--语言卡 <langcard.json>]";
    let mut opts = DocOpts {
        dir: PathBuf::from("."),
        out: None,
        format: None,
        card: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--目录" | "--dir" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--目录 后面要跟包目录"));
                }
                opts.dir = PathBuf::from(&args[i]);
            }
            "-o" | "--输出" | "--out" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("-o 后面要跟输出文件"));
                }
                opts.out = Some(PathBuf::from(&args[i]));
            }
            "--格式" | "--format" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--格式 后面要跟 md 或 html"));
                }
                opts.format = Some(match args[i].as_str() {
                    "md" | "markdown" => Format::Markdown,
                    "html" => Format::Html,
                    other => {
                        return Err(e125(format!("不认识的格式「{}」；只支持 md / html", other)))
                    }
                });
            }
            "--语言卡" | "--langcard" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--语言卡 后面要跟 langcard.json"));
                }
                opts.card = Some(PathBuf::from(&args[i]));
            }
            "-h" | "--help" => return Err(e125(usage)),
            other => return Err(e125(format!("不认识的参数：{}；{}", other, usage))),
        }
        i += 1;
    }
    Ok(opts)
}

fn do_doc(args: &[String]) -> Result<()> {
    let opts = parse_opts(args)?;
    if !opts.dir.is_dir() {
        return Err(e125(format!("包目录不存在：{}", opts.dir.display())));
    }
    let data = pack::build_interface(&opts.dir)?;
    check_drift(&opts.dir, &pack::interface_text(&data))?;
    let card = load_card(&opts.dir, opts.card.as_deref())?;

    let format = opts.format.unwrap_or_else(|| match &opts.out {
        Some(path) if path.extension().and_then(|e| e.to_str()) == Some("html") => Format::Html,
        _ => Format::Markdown,
    });
    let path = opts.out.unwrap_or_else(|| {
        opts.dir.join(if format == Format::Html {
            "文档.html"
        } else {
            "文档.md"
        })
    });
    let text = match format {
        Format::Markdown => markdown_doc(&data, card.as_deref()),
        Format::Html => html_doc(&data, card.as_deref()),
    };
    std::fs::write(&path, text).map_err(|e| e125(format!("写不了 {}：{}", path.display(), e)))?;
    let modules = native_modules(&data, card.as_deref()).len();
    println!(
        "文档：{} {} → {}（函数 {}，类 {}，枚举 {}，原生模块 {}）",
        data.name,
        data.version,
        path.display(),
        data.funcs.len(),
        data.classes.len(),
        data.enums.len(),
        modules
    );
    Ok(())
}

/// `micc publish` 用：返回 HTML 文档字符串，不写文件。
pub(crate) fn render_html(dir: &Path) -> Result<String> {
    let data = pack::build_interface(dir)?;
    check_drift(dir, &pack::interface_text(&data))?;
    let card = load_card(dir, None)?;
    Ok(html_doc(&data, card.as_deref()))
}

/// 包目录已有 `接口.toml` 时，必须与源码重新生成的清单一致。
fn check_drift(dir: &Path, expected: &str) -> Result<()> {
    let path = dir.join("接口.toml");
    if !path.is_file() {
        return Ok(());
    }
    let actual = std::fs::read_to_string(&path)
        .map_err(|e| e125(format!("读不了 {}：{}", path.display(), e)))?;
    if normalize(&actual) != normalize(expected) {
        return Err(e146(format!(
            "接口清单漂移：{} 与当前源码生成的接口不一致",
            path.display()
        ))
        .with_hint("先运行 micc interface --目录 <包目录> 重新生成；确认接口变化后再生成文档"));
    }
    Ok(())
}

fn normalize(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = String::new();
    for line in text.replace("\r\n", "\n").lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/* ---------------- 语言卡 ---------------- */

struct CardEntry {
    name: String,
    min_args: i64,
    max_args: i64,
    doc: String,
}

struct CardModule {
    name: String,
    table: String,
    entries: Vec<CardEntry>,
}

fn load_card(dir: &Path, explicit: Option<&Path>) -> Result<Option<Vec<CardModule>>> {
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(e125(format!("语言卡不存在：{}", path.display())));
        }
        return parse_card(path).map(Some);
    }
    let mut cur = Some(dir);
    while let Some(at) = cur {
        let candidate = at.join("spec").join("langcard.json");
        if candidate.is_file() {
            return parse_card(&candidate).map(Some);
        }
        cur = at.parent();
    }
    Ok(None)
}

fn parse_card(path: &Path) -> Result<Vec<CardModule>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| e125(format!("读不了语言卡 {}：{}", path.display(), e)))?;
    let value = json::parse(&text)
        .map_err(|e| e125(format!("语言卡 {} 不是合法 JSON：{}", path.display(), e)))?;
    let mut modules = Vec::new();
    let Some(items) = value.get("native_modules").and_then(Json::as_arr) else {
        return Ok(modules);
    };
    for item in items {
        let Some(name) = item.string("module") else {
            continue;
        };
        let table = item.string("table").unwrap_or_default();
        let mut entries = Vec::new();
        for entry in item.get("entries").and_then(Json::as_arr).unwrap_or(&[]) {
            let Some(entry_name) = entry.string("name") else {
                continue;
            };
            entries.push(CardEntry {
                name: entry_name,
                min_args: entry.get("min_args").and_then(Json::as_i64).unwrap_or(0),
                max_args: entry.get("max_args").and_then(Json::as_i64).unwrap_or(0),
                doc: entry.string("doc").unwrap_or_default(),
            });
        }
        modules.push(CardModule {
            name,
            table,
            entries,
        });
    }
    Ok(modules)
}

fn native_modules<'a>(data: &InterfaceData, card: Option<&'a [CardModule]>) -> Vec<&'a CardModule> {
    let Some(card) = card else {
        return Vec::new();
    };
    data.native_mods
        .iter()
        .filter_map(|name| card.iter().find(|module| &module.name == name))
        .collect()
}

fn args_text(min: i64, max: i64) -> String {
    if min == max {
        min.to_string()
    } else {
        format!("{}–{}", min, max)
    }
}

/* ---------------- Markdown ---------------- */

fn md_cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn markdown_doc(data: &InterfaceData, card: Option<&[CardModule]>) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", data.name));
    out.push_str(&format!(
        "版本 {}　原生ABI {}　字节码版本 {}\n\n",
        data.version, data.native_abi, data.bytecode_version
    ));

    out.push_str(&format!("## 函数（{}）\n\n", data.funcs.len()));
    if data.funcs.is_empty() {
        out.push_str("（无）\n\n");
    } else {
        out.push_str("| 函数 | 参数个数 |\n| --- | --- |\n");
        for (name, arity) in &data.funcs {
            out.push_str(&format!("| `{}` | {} |\n", md_cell(name), arity));
        }
        out.push('\n');
    }

    out.push_str(&format!("## 类（{}）\n\n", data.classes.len()));
    if data.classes.is_empty() {
        out.push_str("（无）\n\n");
    }
    for class in &data.classes {
        out.push_str(&format!("### `{}`\n\n", class.name));
        if class.fields.is_empty() {
            out.push_str("字段：（无）\n\n");
        } else {
            let fields = class
                .fields
                .iter()
                .map(|f| format!("`{}`", f))
                .collect::<Vec<_>>()
                .join("、");
            out.push_str(&format!("字段：{}\n\n", fields));
        }
        if class.methods.is_empty() {
            out.push_str("方法：（无）\n\n");
        } else {
            out.push_str("| 方法 | 参数个数 |\n| --- | --- |\n");
            for (name, arity) in &class.methods {
                out.push_str(&format!("| `{}` | {} |\n", md_cell(name), arity));
            }
            out.push('\n');
        }
    }

    out.push_str(&format!("## 枚举（{}）\n\n", data.enums.len()));
    if data.enums.is_empty() {
        out.push_str("（无）\n\n");
    }
    for e in &data.enums {
        let members = e
            .members
            .iter()
            .map(|m| format!("`{}`", m))
            .collect::<Vec<_>>()
            .join("、");
        out.push_str(&format!("### `{}`\n\n成员：{}\n\n", e.name, members));
    }

    let modules = native_modules(data, card);
    if !modules.is_empty() {
        out.push_str(&format!("## 原生模块（{}）\n\n", modules.len()));
        for module in modules {
            if module.table.is_empty() {
                out.push_str(&format!("### `{}`\n\n", module.name));
            } else {
                out.push_str(&format!("### `{}`（`{}`）\n\n", module.name, module.table));
            }
            out.push_str("| 函数 | 参数 | 说明 |\n| --- | --- | --- |\n");
            for entry in &module.entries {
                out.push_str(&format!(
                    "| `{}` | {} | {} |\n",
                    md_cell(&entry.name),
                    args_text(entry.min_args, entry.max_args),
                    md_cell(&entry.doc)
                ));
            }
            out.push('\n');
        }
    }
    out
}

/* ---------------- HTML ---------------- */

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_doc(data: &InterfaceData, card: Option<&[CardModule]>) -> String {
    let mut body = String::new();
    body.push_str(&format!("<h1>{}</h1>\n", esc(&data.name)));
    body.push_str(&format!(
        "<p class=\"meta\">版本 {}　原生ABI {}　字节码版本 {}</p>\n",
        esc(&data.version),
        data.native_abi,
        data.bytecode_version
    ));

    body.push_str(&format!("<h2>函数（{}）</h2>\n", data.funcs.len()));
    if data.funcs.is_empty() {
        body.push_str("<p class=\"empty\">（无）</p>\n");
    } else {
        body.push_str("<table><thead><tr><th>函数</th><th>参数个数</th></tr></thead><tbody>\n");
        for (name, arity) in &data.funcs {
            body.push_str(&format!(
                "<tr><td><code>{}</code></td><td>{}</td></tr>\n",
                esc(name),
                arity
            ));
        }
        body.push_str("</tbody></table>\n");
    }

    body.push_str(&format!("<h2>类（{}）</h2>\n", data.classes.len()));
    if data.classes.is_empty() {
        body.push_str("<p class=\"empty\">（无）</p>\n");
    }
    for class in &data.classes {
        body.push_str(&format!("<h3><code>{}</code></h3>\n", esc(&class.name)));
        body.push_str(&fields_html(class));
        if class.methods.is_empty() {
            body.push_str("<p class=\"empty\">方法：（无）</p>\n");
        } else {
            body.push_str("<table><thead><tr><th>方法</th><th>参数个数</th></tr></thead><tbody>\n");
            for (name, arity) in &class.methods {
                body.push_str(&format!(
                    "<tr><td><code>{}</code></td><td>{}</td></tr>\n",
                    esc(name),
                    arity
                ));
            }
            body.push_str("</tbody></table>\n");
        }
    }

    body.push_str(&format!("<h2>枚举（{}）</h2>\n", data.enums.len()));
    if data.enums.is_empty() {
        body.push_str("<p class=\"empty\">（无）</p>\n");
    }
    for e in &data.enums {
        let members = e
            .members
            .iter()
            .map(|m| format!("<code>{}</code>", esc(m)))
            .collect::<Vec<_>>()
            .join("、");
        body.push_str(&format!(
            "<h3><code>{}</code></h3>\n<p class=\"members\">{}</p>\n",
            esc(&e.name),
            members
        ));
    }

    let modules = native_modules(data, card);
    if !modules.is_empty() {
        body.push_str(&format!("<h2>原生模块（{}）</h2>\n", modules.len()));
        for module in modules {
            let title = if module.table.is_empty() {
                format!("<h3><code>{}</code></h3>", esc(&module.name))
            } else {
                format!(
                    "<h3><code>{}</code> <span class=\"tag\">{}</span></h3>",
                    esc(&module.name),
                    esc(&module.table)
                )
            };
            body.push_str(&title);
            body.push_str(
                "<table><thead><tr><th>函数</th><th>参数</th><th>说明</th></tr></thead><tbody>\n",
            );
            for entry in &module.entries {
                body.push_str(&format!(
                    "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>\n",
                    esc(&entry.name),
                    args_text(entry.min_args, entry.max_args),
                    esc(&entry.doc)
                ));
            }
            body.push_str("</tbody></table>\n");
        }
    }

    format!(
        "<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<meta name=\"generator\" content=\"micc doc\">\n<title>{} {}</title>\n\
<style>\n\
:root {{ color-scheme: light dark; --bg:#fbfaf7; --fg:#1d1b18; --muted:#6b6560; --line:#e4ded5; --accent:#8a4b19; --card:#fff; }}\n\
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#161513; --fg:#eae4da; --muted:#a0988e; --line:#34302b; --accent:#e0a26b; --card:#1e1c19; }} }}\n\
* {{ box-sizing: border-box; }}\n\
body {{ margin:0; background:var(--bg); color:var(--fg); font:15px/1.65 \"Segoe UI\",\"Microsoft YaHei\",system-ui,sans-serif; }}\n\
main {{ max-width: 920px; margin: 0 auto; padding: 40px 20px 64px; }}\n\
h1 {{ font-size: 28px; margin: 0 0 4px; letter-spacing: .04em; }}\n\
.meta, .empty, .members {{ color: var(--muted); }}\n\
h2 {{ font-size: 20px; margin: 34px 0 12px; border-bottom: 1px solid var(--line); padding-bottom: 6px; }}\n\
h3 {{ font-size: 17px; margin: 22px 0 8px; }}\n\
code {{ font-family: ui-monospace,Consolas,monospace; font-size: 13px; }}\n\
table {{ width:100%; border-collapse: collapse; background:var(--card); border:1px solid var(--line); border-radius:10px; overflow:hidden; }}\n\
th, td {{ text-align:left; padding:9px 12px; border-bottom:1px solid var(--line); vertical-align:top; }}\n\
th {{ font-size:13px; color:var(--muted); font-weight:600; }}\n\
tr:last-child td {{ border-bottom:0; }}\n\
tr:hover td {{ background: rgba(138,75,25,.05); }}\n\
.tag {{ display:inline-block; border:1px solid var(--line); border-radius:999px; padding:1px 8px; color:var(--muted); font-size:12px; }}\n\
.rule {{ color: var(--muted); }}\n\
</style>\n</head>\n<body>\n<main>\n{}\n<footer class=\"meta\">由 <code>micc doc</code> 生成</footer>\n</main>\n</body>\n</html>\n",
        esc(&data.name),
        esc(&data.version),
        body
    )
}

fn fields_html(class: &InterfaceClass) -> String {
    if class.fields.is_empty() {
        return "<p class=\"empty\">字段：（无）</p>\n".to_string();
    }
    let list = class
        .fields
        .iter()
        .map(|f| format!("<code>{}</code>", esc(f)))
        .collect::<Vec<_>>()
        .join("、");
    format!("<p>字段：{}</p>\n", list)
}

//! `砾包.toml` 的解析与校验（H4 最小可用切分）。
//!
//! 只实现包规范需要的一小撮 TOML 子集：`[表]`、`键 = 值`，值支持字符串、
//! 十进制整数、字符串数组与单行内联表。为了写法统一，键名允许中文
//! （比标准 TOML 宽松，标准 TOML 的裸键只允许 ASCII）。

use crate::error::{CompileError, Result};
use std::path::{Path, PathBuf};

pub const MANIFEST_NAME: &str = "砾包.toml";
pub const DEFAULT_ENTRY: &str = "主.jsh";
pub const DEFAULT_BYTECODE_LIMIT: usize = 65535;
pub const DEFAULT_HEAP_BUDGET: usize = 6144;

/// 语义化版本：只接受 `主.次.修订` 三段十进制。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.trim().split('.');
        let a = it.next()?.parse().ok()?;
        let b = it.next()?.parse().ok()?;
        let c = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        Some(Version(a, b, c))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// 版本约束：精确 / `^` / `*`。
#[derive(Clone, Debug)]
pub enum Constraint {
    Any,
    Exact(Version),
    Caret(Version),
}

impl Constraint {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s == "*" {
            return Some(Constraint::Any);
        }
        let (caret, rest) = match s.strip_prefix('^') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let v = Version::parse(rest)?;
        Some(if caret {
            Constraint::Caret(v)
        } else {
            Constraint::Exact(v)
        })
    }

    pub fn allows(&self, v: Version) -> bool {
        match self {
            Constraint::Any => true,
            Constraint::Exact(x) => *x == v,
            // `^1.2.3`：主版本相同且不小于；`^0.2.3`：次版本相同且不小于。
            Constraint::Caret(x) => {
                if x.0 > 0 {
                    v.0 == x.0 && v >= *x
                } else {
                    v.0 == 0 && v.1 == x.1 && v >= *x
                }
            }
        }
    }

    pub fn text(&self) -> String {
        match self {
            Constraint::Any => "*".to_string(),
            Constraint::Exact(v) => v.to_string(),
            Constraint::Caret(v) => format!("^{}", v),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DepSpec {
    /// 相对当前包目录的路径（也允许绝对路径）；`None` 表示从静态索引解析。
    pub path: Option<String>,
    pub constraint: Constraint,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub name: String,
    pub version: Version,
    pub entry: String,
    pub native_mods: Vec<String>,
    pub native_abi: u8,
    pub bytecode_limit: usize,
    pub heap_budget: usize,
    /// 二进制包：预编译产物的相对路径（`.jmb`）。`None` = 源码包。
    pub product: Option<String>,
    /// `None` = 顶层名字全部导出。
    pub exports: Option<Vec<String>>,
    /// `[导出] 类型`：可跨包的类/枚举名；`None` = 不导出任何类型。
    pub export_types: Option<Vec<String>>,
    pub deps: Vec<(String, DepSpec)>,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Manifest> {
        let path = dir.join(MANIFEST_NAME);
        let text = read_text(&path)?;
        parse_manifest(&path, &text)
    }

    /// 校验清单声明的 `原生ABI` 与当前编译器一致（E129）。
    /// `install` 与 `pack` 共用同一口径与文案。
    pub fn check_native_abi(&self) -> Result<()> {
        let cur = crate::native_spec::NATIVE_ABI_VERSION;
        if self.native_abi != cur {
            return Err(CompileError::new(
                "E129",
                1,
                1,
                format!(
                    "包「{}」要求 Native ABI v{}，当前编译器是 v{}；ABI 不一致，请用 v{} 重新打包（原生ABI = {}）",
                    self.name, self.native_abi, cur, cur, cur
                ),
            ));
        }
        Ok(())
    }
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| {
        CompileError::new(
            "E125",
            1,
            1,
            format!("读不了包规范 {}：{}", path.display(), e),
        )
    })?;
    let mut text = String::from_utf8(bytes).map_err(|_| {
        CompileError::new(
            "E125",
            1,
            1,
            format!("{} 不是 UTF-8 文本", path.display()),
        )
    })?;
    if text.starts_with('\u{feff}') {
        // BOM 在 UTF-8 里是 3 字节，按字节切避开字符边界 panic。
        text.drain(..'\u{feff}'.len_utf8());
    }
    Ok(text)
}

/// 去掉引号外的 `#` 注释。
pub(crate) fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, ch) in line.char_indices() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                } else if ch == '#' {
                    return &line[..i];
                }
            }
        }
    }
    line
}

/// 解析带引号的字符串（允许 `\"` `\\` `\n` `\t`）。
fn parse_quoted(s: &str) -> Option<(String, usize)> {
    let mut chars = s.char_indices();
    let (_, q) = chars.next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let mut out = String::new();
    while let Some((i, ch)) = chars.next() {
        if ch == q {
            return Some((out, i + ch.len_utf8()));
        }
        if ch == '\\' {
            let (_, esc) = chars.next()?;
            out.push(match esc {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\'' => '\'',
                '\\' => '\\',
                other => other,
            });
        } else {
            out.push(ch);
        }
    }
    None
}

pub(crate) enum Val {
    Str(String),
    Int(i64),
    Arr(Vec<String>),
    Table(Vec<(String, String)>),
}

fn parse_value(s: &str) -> Option<Val> {
    let s = s.trim();
    if s.starts_with('"') || s.starts_with('\'') {
        let (v, used) = parse_quoted(s)?;
        return if s[used..].trim().is_empty() {
            Some(Val::Str(v))
        } else {
            None
        };
    }
    if let Some(rest) = s.strip_prefix('[') {
        let body = rest.strip_suffix(']')?;
        if body.trim().is_empty() {
            return Some(Val::Arr(Vec::new()));
        }
        let mut out = Vec::new();
        let mut buf = String::new();
        let mut quote: Option<char> = None;
        for ch in body.chars() {
            match quote {
                Some(q) => {
                    buf.push(ch);
                    if ch == q {
                        quote = None;
                    }
                }
                None => {
                    if ch == '"' || ch == '\'' {
                        quote = Some(ch);
                        buf.push(ch);
                    } else if ch == ',' {
                        out.push(parse_value(&buf)?.as_str()?);
                        buf.clear();
                    } else {
                        buf.push(ch);
                    }
                }
            }
        }
        if !buf.trim().is_empty() {
            out.push(parse_value(&buf)?.as_str()?);
        }
        return Some(Val::Arr(out));
    }
    if let Some(rest) = s.strip_prefix('{') {
        let body = rest.strip_suffix('}')?;
        if body.trim().is_empty() {
            return Some(Val::Table(Vec::new()));
        }
        let mut out = Vec::new();
        let mut buf = String::new();
        let mut quote: Option<char> = None;
        for ch in body.chars() {
            match quote {
                Some(q) => {
                    buf.push(ch);
                    if ch == q {
                        quote = None;
                    }
                }
                None => {
                    if ch == '"' || ch == '\'' {
                        quote = Some(ch);
                        buf.push(ch);
                    } else if ch == ',' {
                        let (k, v) = parse_field(&buf)?;
                        out.push((k, v.as_str()?));
                        buf.clear();
                    } else {
                        buf.push(ch);
                    }
                }
            }
        }
        if !buf.trim().is_empty() {
            let (k, v) = parse_field(&buf)?;
            out.push((k, v.as_str()?));
        }
        return Some(Val::Table(out));
    }
    s.parse::<i64>().ok().map(Val::Int)
}

impl Val {
    pub(crate) fn as_str(self) -> Option<String> {
        match self {
            Val::Str(s) => Some(s),
            Val::Int(i) => Some(i.to_string()),
            _ => None,
        }
    }
}

/// 解析 `键 = 值`；键允许中文裸键或引号。
pub(crate) fn parse_field(s: &str) -> Option<(String, Val)> {
    let s = s.trim();
    let (key, rest) = if s.starts_with('"') || s.starts_with('\'') {
        let (k, used) = parse_quoted(s)?;
        let rest = s[used..].trim_start();
        (k, rest.strip_prefix('=')?.trim())
    } else {
        let i = s.find('=')?;
        (s[..i].trim().to_string(), s[i + 1..].trim())
    };
    Some((key, parse_value(rest)?))
}

fn err(path: &Path, line: usize, msg: impl Into<String>) -> CompileError {
    CompileError::new(
        "E125",
        line as u32,
        1,
        format!("包规范 {}：{}", path.display(), msg.into()),
    )
}

pub fn parse_manifest(path: &Path, text: &str) -> Result<Manifest> {
    let mut section = String::new();
    let mut name: Option<String> = None;
    let mut version: Option<Version> = None;
    let mut entry: Option<String> = None;
    // 有值 = 二进制包（只发预编译产物，不带源码）。
    let mut product: Option<String> = None;
    let mut native_mods: Vec<String> = Vec::new();
    let mut native_abi: Option<u8> = None;
    let mut bytecode_limit: Option<usize> = None;
    let mut heap_budget: Option<usize> = None;
    let mut exports: Option<Vec<String>> = None;
    let mut export_types: Option<Vec<String>> = None;
    let mut deps: Vec<(String, DepSpec)> = Vec::new();

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("[[") {
            return Err(err(path, line_no, "不支持数组表（[[...]]）"));
        }
        if line.starts_with('[') {
            let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
                return Err(err(path, line_no, "表头缺少右方括号"));
            };
            let name = inner.trim();
            if !matches!(name, "包" | "依赖" | "导出") {
                return Err(err(
                    path,
                    line_no,
                    format!("不认识的表「{}」；只支持 [包] / [依赖] / [导出]", name),
                ));
            }
            section = name.to_string();
            continue;
        }
        let Some((key, value)) = parse_field(line) else {
            return Err(err(path, line_no, "这一行不是 键 = 值 的形式"));
        };
        match section.as_str() {
            "包" => match key.as_str() {
                "名字" => name = Some(as_str(path, line_no, &key, value)?),
                "版本" => {
                    let v = as_str(path, line_no, &key, value)?;
                    version = Some(Version::parse(&v).ok_or_else(|| {
                        err(path, line_no, format!("版本「{}」不是 主.次.修订", v))
                    })?)
                }
                "入口" => entry = Some(as_str(path, line_no, &key, value)?),
                "产物" => {
                    let v = as_str(path, line_no, &key, value)?;
                    if !v.ends_with(".jmb") {
                        return Err(err(
                            path,
                            line_no,
                            format!("产物 必须是 .jmb 文件：{}", v),
                        ));
                    }
                    if v.starts_with('/') || v.starts_with('\\') || v.contains("..") {
                        return Err(err(
                            path,
                            line_no,
                            format!("产物 只能是包目录内的相对路径：{}", v),
                        ));
                    }
                    product = Some(v);
                }
                "原生ABI" => {
                    let v = as_str(path, line_no, &key, value)?;
                    native_abi = Some(v.parse::<u8>().map_err(|_| {
                        err(path, line_no, format!("原生ABI 不是 0~255 的整数：{}", v))
                    })?)
                }
                "字节码上限" => {
                    let v = as_str(path, line_no, &key, value)?;
                    bytecode_limit = Some(v.parse::<usize>().map_err(|_| {
                        err(path, line_no, format!("字节码上限不是整数：{}", v))
                    })?)
                }
                "堆预算" => {
                    let v = as_str(path, line_no, &key, value)?;
                    heap_budget = Some(v.parse::<usize>().map_err(|_| {
                        err(path, line_no, format!("堆预算不是整数：{}", v))
                    })?)
                }
                "原生" => {
                    native_mods = as_array(path, line_no, &key, value)?;
                }
                other => {
                    return Err(err(
                        path,
                        line_no,
                        format!("[包] 里不认识的字段「{}」", other),
                    ))
                }
            },
            "依赖" => {
                let Some(fields) = as_table(value) else {
                    return Err(err(
                        path,
                        line_no,
                        format!(
                            "依赖「{}」要写成 {{ 路径 = \"...\", 版本 = \"...\" }}",
                            key
                        ),
                    ));
                };
                let mut dep_path: Option<String> = None;
                let mut dep_ver: Option<Constraint> = None;
                for (k, v) in fields {
                    match k.as_str() {
                        "路径" => dep_path = Some(v),
                        "版本" => {
                            dep_ver = Some(Constraint::parse(&v).ok_or_else(|| {
                                err(
                                    path,
                                    line_no,
                                    format!("版本约束「{}」不被支持；只支持 精确 / ^ / *", v),
                                )
                            })?)
                        }
                        other => {
                            return Err(err(
                                path,
                                line_no,
                                format!("依赖里不认识的字段「{}」", other),
                            ))
                        }
                    }
                }
                // 不写 路径 = 从静态索引解析（H5）；写了就是本地路径依赖。
                if deps.iter().any(|(n, _)| n == &key) {
                    return Err(err(path, line_no, format!("依赖「{}」重复声明", key)));
                }
                deps.push((
                    key.clone(),
                    DepSpec {
                        path: dep_path,
                        constraint: dep_ver.unwrap_or(Constraint::Any),
                    },
                ));
            }
            "导出" => match key.as_str() {
                "名字" => {
                    if exports.is_some() {
                        return Err(err(path, line_no, "[导出] 名字 重复声明"));
                    }
                    exports = Some(as_array(path, line_no, &key, value)?);
                }
                "类型" => {
                    if export_types.is_some() {
                        return Err(err(path, line_no, "[导出] 类型 重复声明"));
                    }
                    export_types = Some(as_array(path, line_no, &key, value)?);
                }
                other => {
                    return Err(err(
                        path,
                        line_no,
                        format!(
                            "[导出] 里只写「名字 = [...]」或「类型 = [...]」，不认识的字段「{}」",
                            other
                        ),
                    ));
                }
            },
            _ => return Err(err(path, line_no, "键值对必须写在 [包] / [依赖] / [导出] 里")),
        }
    }

    let Some(name) = name else {
        return Err(err(path, 1, "缺少 [包] 名字"));
    };
    if name.trim().is_empty() {
        return Err(err(path, 1, "[包] 名字不能为空"));
    }
    for bad in ['.', ':', '/', '\\', '#', ' ', '\t'] {
        if name.contains(bad) {
            return Err(err(
                path,
                1,
                format!("[包] 名字「{}」不能包含「{}」", name, bad),
            ));
        }
    }
    let Some(version) = version else {
        return Err(err(path, 1, "缺少 [包] 版本"));
    };

    Ok(Manifest {
        name,
        version,
        entry: entry.unwrap_or_else(|| DEFAULT_ENTRY.to_string()),
        native_mods,
        native_abi: native_abi.unwrap_or(crate::native_spec::NATIVE_ABI_VERSION),
        bytecode_limit: bytecode_limit.unwrap_or(DEFAULT_BYTECODE_LIMIT),
        heap_budget: heap_budget.unwrap_or(DEFAULT_HEAP_BUDGET),
        product,
        exports,
        export_types,
        deps,
    })
}

fn as_str(path: &Path, line_no: usize, key: &str, v: Val) -> Result<String> {
    match v {
        Val::Str(s) => Ok(s),
        Val::Int(i) => Ok(i.to_string()),
        _ => Err(err(
            path,
            line_no,
            format!("「{}」这里要写一个值（字符串或整数）", key),
        )),
    }
}

fn as_array(path: &Path, line_no: usize, key: &str, v: Val) -> Result<Vec<String>> {
    match v {
        Val::Arr(a) => Ok(a),
        _ => Err(err(
            path,
            line_no,
            format!("「{}」这里要写字符串数组，例如 [\"a\", \"b\"]", key),
        )),
    }
}

fn as_table(v: Val) -> Option<Vec<(String, String)>> {
    match v {
        Val::Table(t) => Some(t),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 锁文件（` .砾/锁.toml `）
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct LockEntry {
    pub name: String,
    pub version: Version,
    pub source: String,
    pub hash: String,
}

pub fn lock_path(root: &Path) -> PathBuf {
    root.join(".砾").join("锁.toml")
}

pub fn read_lock(root: &Path) -> Result<Option<Vec<LockEntry>>> {
    let path = lock_path(root);
    if !path.is_file() {
        return Ok(None);
    }
    let text = read_text(&path)?;
    parse_lock(&path, &text).map(Some)
}

fn parse_lock(path: &Path, text: &str) -> Result<Vec<LockEntry>> {
    let mut section = String::new();
    let mut version_ok = false;
    let mut entries: Vec<LockEntry> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
                return Err(err(path, line_no, "表头缺少右方括号"));
            };
            section = inner.trim().to_string();
            if section != "包" {
                return Err(err(path, line_no, format!("锁文件里不认识的表「{}」", section)));
            }
            continue;
        }
        let Some((key, value)) = parse_field(line) else {
            return Err(err(path, line_no, "这一行不是 键 = 值 的形式"));
        };
        if section.is_empty() {
            if key == "锁版本" {
                match value {
                    Val::Int(1) => version_ok = true,
                    _ => return Err(err(path, line_no, "只认识 锁版本 = 1")),
                }
            }
            continue;
        }
        let Some(fields) = as_table(value) else {
            return Err(err(path, line_no, "锁里的包要写成内联表"));
        };
        let mut version: Option<Version> = None;
        let mut source = String::new();
        let mut hash = String::new();
        for (k, v) in fields {
            match k.as_str() {
                "版本" => {
                    version = Some(Version::parse(&v).ok_or_else(|| {
                        err(path, line_no, format!("锁里的版本「{}」不是 主.次.修订", v))
                    })?)
                }
                "来源" => source = v,
                "校验" => hash = v,
                other => {
                    return Err(err(
                        path,
                        line_no,
                        format!("锁里不认识的字段「{}」", other),
                    ))
                }
            }
        }
        let Some(version) = version else {
            return Err(err(path, line_no, format!("锁里的包「{}」缺少 版本", key)));
        };
        if hash.is_empty() {
            return Err(err(path, line_no, format!("锁里的包「{}」缺少 校验", key)));
        }
        if entries.iter().any(|e| e.name == key) {
            return Err(err(path, line_no, format!("锁里包「{}」重复", key)));
        }
        entries.push(LockEntry {
            name: key,
            version,
            source,
            hash,
        });
    }
    if !version_ok {
        return Err(err(path, 1, "缺少 锁版本 = 1"));
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub fn write_lock(root: &Path, entries: &[LockEntry]) -> Result<()> {
    let path = lock_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CompileError::new(
                "E125",
                1,
                1,
                format!("建不了目录 {}：{}", parent.display(), e),
            )
        })?;
    }
    let mut s = String::from("# 本文件由 micc install 生成，不要手改\n锁版本 = 1\n\n[包]\n");
    for e in entries {
        s.push_str(&format!(
            "{} = {{ 版本 = \"{}\", 来源 = \"{}\", 校验 = \"{}\" }}\n",
            e.name, e.version, e.source, e.hash
        ));
    }
    std::fs::write(&path, s).map_err(|e| {
        CompileError::new(
            "E125",
            1,
            1,
            format!("写不了锁文件 {}：{}", path.display(), e),
        )
    })
}

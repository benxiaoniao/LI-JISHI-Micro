//! H5 静态包索引（`砾索引.toml`）。
//!
//! 边界先说清：本项目**不跑服务器**，所以这里只做「静态索引」——一个能放在
//! GitHub raw / 本地目录里的文本文件，`micc` 读它、按版本约束挑版本、取内容、
//! 按 H4 的 FNV-1a64 目录哈希校验。上传、鉴权、元数据检索等注册中心能力
//! 不在本轮范围内。
//!
//! 索引格式复用 `砾包.toml` 的 TOML 子集解析器（同一份 `parse_field`），
//! 不引入任何外部依赖，也不新增 TOML/JSON 解析代码。

use crate::error::{CompileError, Result};
use crate::pack_manifest::{parse_field, strip_comment, Constraint, Val, Version};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const INDEX_NAME: &str = "砾索引.toml";

/// 索引里的一条包记录。
#[derive(Clone, Debug)]
pub struct IndexEntry {
    pub name: String,
    pub version: Version,
    /// 本地目录、`file:` 路径，或 `http(s)://` 归档地址。
    pub source: String,
    /// H4 的目录内容哈希：`fnv1a64:xxxxxxxxxxxxxxxx`。
    pub hash: String,
}

#[derive(Debug)]
pub struct Index {
    pub by_name: BTreeMap<String, Vec<IndexEntry>>,
    /// 本地索引所在目录；相对 `来源` 按它解析。HTTP 索引没有基准目录。
    pub base_dir: Option<PathBuf>,
}

fn idx_err(line: usize, msg: impl Into<String>) -> CompileError {
    CompileError::new(
        "E132",
        line as u32,
        1,
        format!("索引 {}：{}", INDEX_NAME, msg.into()),
    )
}

/// 读入索引：支持本地路径、`file:` 前缀与 `http(s)://`。
pub fn load_index(source: &str, temp: &mut TempDirs) -> Result<Index> {
    let source = source.trim();
    if is_http(source) {
        let work = temp.alloc()?;
        let file = work.join("砾索引.toml");
        curl_download(source, &file)?;
        let text = read_index_text(&file)?;
        return parse_index(&text, None);
    }
    let path = strip_file_prefix(source);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|e| idx_err(1, format!("取当前目录失败：{}", e)))?
            .join(path)
    };
    if !path.is_file() {
        return Err(idx_err(
            1,
            format!("读不了 {}：文件不存在（用 --索引 指定索引文件，或放到包目录下）", path.display()),
        ));
    }
    let text = read_index_text(&path)?;
    let base = path.parent().map(|p| p.to_path_buf());
    parse_index(&text, base)
}

fn read_index_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| idx_err(1, format!("读不了 {}：{}", path.display(), e)))?;
    let mut text = String::from_utf8(bytes)
        .map_err(|_| idx_err(1, format!("{} 不是 UTF-8 文本", path.display())))?;
    if text.starts_with('\u{feff}') {
        text.drain(..'\u{feff}'.len_utf8());
    }
    Ok(text)
}

/// 解析索引文本。`base_dir` 只在本地索引时给出。
pub fn parse_index(text: &str, base_dir: Option<PathBuf>) -> Result<Index> {
    let mut section = String::new();
    let mut version_ok = false;
    let mut by_name: BTreeMap<String, Vec<IndexEntry>> = BTreeMap::new();
    let mut seen: Vec<(String, Version)> = Vec::new();

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
                return Err(idx_err(line_no, "表头缺少右方括号"));
            };
            let name = inner.trim();
            if name != "包" {
                return Err(idx_err(
                    line_no,
                    format!("不认识的表「{}」；只支持 [包]", name),
                ));
            }
            section = name.to_string();
            continue;
        }
        let Some((key, value)) = parse_field(line) else {
            return Err(idx_err(line_no, "这一行不是 键 = 值 的形式"));
        };
        if section.is_empty() {
            if key == "索引版本" {
                match value {
                    Val::Int(1) => version_ok = true,
                    _ => return Err(idx_err(line_no, "只认识 索引版本 = 1")),
                }
            }
            continue;
        }
        // [包] 里：`"名字@版本" = { 来源 = "...", 校验 = "fnv1a64:..." }`
        let Some((name, version)) = split_key(&key) else {
            return Err(idx_err(
                line_no,
                format!("条目「{}」要写成 名字@主.次.修订", key),
            ));
        };
        let Some(fields) = as_table(value) else {
            return Err(idx_err(
                line_no,
                format!("条目「{}」要写成 {{ 来源 = \"...\", 校验 = \"...\" }}", key),
            ));
        };
        let mut source = String::new();
        let mut hash = String::new();
        for (k, v) in fields {
            match k.as_str() {
                "来源" => source = v,
                "校验" => hash = v,
                other => {
                    return Err(idx_err(
                        line_no,
                        format!("条目「{}」里不认识的字段「{}」", key, other),
                    ))
                }
            }
        }
        if source.trim().is_empty() {
            return Err(idx_err(line_no, format!("条目「{}」缺少 来源", key)));
        }
        if hash.trim().is_empty() {
            return Err(idx_err(line_no, format!("条目「{}」缺少 校验", key)));
        }
        if !valid_hash(&hash) {
            return Err(idx_err(
                line_no,
                format!(
                    "条目「{}」的校验「{}」不合法；应为 fnv1a64: 加 16 位十六进制",
                    key, hash
                ),
            ));
        }
        hash = hash.to_ascii_lowercase();
        if seen.iter().any(|(n, v)| n == &name && *v == version) {
            return Err(idx_err(
                line_no,
                format!("条目「{}」重复（同一包同一版本只能出现一次）", key),
            ));
        }
        seen.push((name.to_string(), version));
        by_name.entry(name.to_string()).or_default().push(IndexEntry {
            name: name.to_string(),
            version,
            source,
            hash,
        });
    }

    if !version_ok {
        return Err(idx_err(1, "缺少 索引版本 = 1"));
    }
    Ok(Index { by_name, base_dir })
}

fn split_key(key: &str) -> Option<(String, Version)> {
    let (name, ver) = key.rsplit_once('@')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    for bad in ['.', ':', '/', '\\', '#', ' ', '\t'] {
        if name.contains(bad) {
            return None;
        }
    }
    Some((name.to_string(), Version::parse(ver)?))
}

fn valid_hash(hash: &str) -> bool {
    match hash.strip_prefix("fnv1a64:") {
        Some(rest) => rest.len() == 16 && rest.chars().all(|c| c.is_ascii_hexdigit()),
        None => false,
    }
}

fn as_table(v: Val) -> Option<Vec<(String, String)>> {
    match v {
        Val::Table(t) => Some(t),
        _ => None,
    }
}

impl Index {
    /// 按名字与约束挑版本：满足约束的版本里取最高者。
    pub fn select<'a>(&'a self, name: &str, constraint: &Constraint) -> Result<&'a IndexEntry> {
        let Some(list) = self.by_name.get(name) else {
            return Err(CompileError::new(
                "E133",
                1,
                1,
                format!("索引里没有包「{}」", name),
            )
            .with_hint("检查包名拼写；该包可能还没发布进这份索引"));
        };
        let best = list
            .iter()
            .filter(|e| constraint.allows(e.version))
            .max_by_key(|e| e.version);
        match best {
            Some(e) => Ok(e),
            None => {
                let mut available: Vec<String> =
                    list.iter().map(|e| e.version.to_string()).collect();
                available.sort();
                Err(CompileError::new(
                    "E134",
                    1,
                    1,
                    format!(
                        "索引里没有满足「{}」{} 的版本",
                        name,
                        constraint.text()
                    ),
                )
                .with_hint(format!(
                    "索引现有版本：{}；可以放宽 版本 约束，或等索引里发布满足条件的版本",
                    available.join("、")
                )))
            }
        }
    }

    /// 把条目内容准备好，返回一个可读的包目录。
    ///
    /// - 本地 `file:`/相对/绝对路径：直接返回该目录（不复制）。
    /// - `http(s)://`：用系统 `curl` 下载归档，再用系统 `tar` 解包。
    pub fn materialize(&self, entry: &IndexEntry, temp: &mut TempDirs) -> Result<PathBuf> {
        let source = entry.source.trim();
        if is_http(source) {
            let work = temp.alloc()?;
            let archive = work.join("包归档");
            let unpacked = work.join("解包");
            std::fs::create_dir_all(&unpacked)
                .map_err(|e| idx_err(1, format!("建不了临时目录 {}：{}", unpacked.display(), e)))?;
            curl_download(source, &archive)?;
            extract_archive(&archive, &unpacked)?;
            Ok(unwrap_single_dir(unpacked))
        } else {
            let raw = strip_file_prefix(source);
            if raw.is_relative() && self.base_dir.is_none() {
                return Err(CompileError::new(
                    "E135",
                    1,
                    1,
                    format!(
                        "包「{}」{} 的来源是相对路径「{}」，但索引来自 HTTP，没有基准目录",
                        entry.name, entry.version, source
                    ),
                )
                .with_hint("远程索引里的 来源 请写绝对路径或 http(s):// 地址"));
            }
            let dir = local_source_path(source, self.base_dir.as_deref());
            if dir.is_file() {
                // 本地归档（`micc pack --源码` 的产物或任意 zip/tar）：解包后再走目录校验。
                let work = temp.alloc()?;
                let unpacked = work.join("解包");
                std::fs::create_dir_all(&unpacked).map_err(|e| {
                    idx_err(1, format!("建不了临时目录 {}：{}", unpacked.display(), e))
                })?;
                extract_archive(&dir, &unpacked)?;
                return Ok(unwrap_single_dir(unpacked));
            }
            if !dir.is_dir() {
                return Err(CompileError::new(
                    "E135",
                    1,
                    1,
                    format!(
                        "包「{}」{} 的来源 {} 既不是目录也不是归档文件；本地来源只接受目录或 .zip/.tar/.tar.gz",
                        entry.name,
                        entry.version,
                        dir.display()
                    ),
                ));
            }
            Ok(dir)
        }
    }
}

/// 校验目录内容与索引条目一致（名字/版本另由调用方核对清单）。
pub fn verify_entry(entry: &IndexEntry, dir: &Path) -> Result<()> {
    let got = crate::pack_resolve::dir_hash(dir)?;
    if got != entry.hash {
        return Err(CompileError::new(
            "E136",
            1,
            1,
            format!(
                "包「{}」{} 的内容与索引不一致（索引 {}，实际 {}）",
                entry.name, entry.version, entry.hash, got
            ),
        )
        .with_hint("内容可能被改动，或索引里的 校验 填错了；确认来源后重新生成索引"));
    }
    Ok(())
}

fn is_http(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn strip_file_prefix(s: &str) -> PathBuf {
    // `file:///C:/x` → `C:/x`；`file:相对/路径` 保持原样。
    if let Some(rest) = s.strip_prefix("file://") {
        return PathBuf::from(rest.strip_prefix('/').unwrap_or(rest));
    }
    if let Some(rest) = s.strip_prefix("file:") {
        return PathBuf::from(rest);
    }
    PathBuf::from(s)
}

fn local_source_path(source: &str, base_dir: Option<&Path>) -> PathBuf {
    let p = strip_file_prefix(source);
    if p.is_absolute() {
        return p;
    }
    base_dir
        .map(|b| b.join(&p))
        .unwrap_or_else(|| PathBuf::from(&p))
}

fn curl_download(url: &str, out: &Path) -> Result<()> {
    let status = std::process::Command::new("curl")
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(out)
        .arg(url)
        .status()
        .map_err(|e| {
            CompileError::new(
                "E135",
                1,
                1,
                format!("下载 {} 失败：找不到系统 curl（{}）", url, e),
            )
            .with_hint("本机需要可用的 curl；Windows 10/11 自带，其他系统请先确认 curl 在 PATH 里")
        })?;
    if !status.success() {
        return Err(CompileError::new(
            "E135",
            1,
            1,
            format!("下载 {} 失败：curl 退出码 {:?}", url, status.code()),
        ));
    }
    Ok(())
}

/// 是否 ZIP：按文件头「PK」+ 类型判断。
///
/// 不能只看扩展名——HTTP 索引下载的归档落在临时目录里叫 `包归档`，没有 `.zip` 后缀，
/// 按扩展名会漏掉 Linux 上的 unzip 回退。
fn is_zip_archive(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    match std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)) {
        Ok(()) => {
            magic[0] == b'P'
                && magic[1] == b'K'
                && ((magic[2] == 3 && magic[3] == 4)
                    || (magic[2] == 5 && magic[3] == 6)
                    || (magic[2] == 7 && magic[3] == 8))
        }
        Err(_) => false,
    }
}

/// 解包用的临时序号，保证同一进程内并发调用不撞名。
static STAGE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    // Windows 的 tar/unzip 走 ANSI：路径里有中文时 bsdtar 会报
    // `Failed to open '...\???'`。这类情况先把归档与解包目录都放到 ASCII 临时路径，
    // 解完再用 Rust 搬回目标目录（Rust 在 Windows 上走 UTF-16，不受 locale 影响）。
    if needs_ascii_staging(archive, dest) {
        return extract_with_ascii_staging(archive, dest);
    }
    extract_archive_direct(archive, dest)
}

#[cfg(windows)]
fn needs_ascii_staging(archive: &Path, dest: &Path) -> bool {
    !archive.to_string_lossy().is_ascii() || !dest.to_string_lossy().is_ascii()
}

#[cfg(not(windows))]
fn needs_ascii_staging(_archive: &Path, _dest: &Path) -> bool {
    false
}

#[cfg(windows)]
fn extract_with_ascii_staging(archive: &Path, dest: &Path) -> Result<()> {
    let seq = STAGE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let work = std::env::temp_dir().join(format!("li-unpack-{}-{}", std::process::id(), seq));
    let staged = work.join(if is_zip_archive(archive) { "a.zip" } else { "a.tar" });
    let out = work.join("out");
    let result = std::fs::create_dir_all(&out)
        .map_err(|e| idx_err(1, format!("建不了临时目录 {}：{}", out.display(), e)))
        .and_then(|_| {
            std::fs::copy(archive, &staged).map_err(|e| {
                idx_err(1, format!("复制归档 {} 失败：{}", archive.display(), e))
            })
        })
        .and_then(|_| extract_archive_direct(&staged, &out))
        .and_then(|_| move_tree(&out, dest));
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(windows)]
fn move_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)
        .map_err(|e| idx_err(1, format!("建不了目录 {}：{}", to.display(), e)))?;
    let entries = std::fs::read_dir(from)
        .map_err(|e| idx_err(1, format!("读不了目录 {}：{}", from.display(), e)))?;
    for entry in entries {
        let entry = entry.map_err(|e| idx_err(1, format!("读目录项失败：{}", e)))?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            move_tree(&src, &dst)?;
        } else if std::fs::rename(&src, &dst).is_err() {
            std::fs::copy(&src, &dst)
                .map_err(|e| idx_err(1, format!("搬移 {} 失败：{}", src.display(), e)))?;
        }
    }
    Ok(())
}

fn extract_archive_direct(archive: &Path, dest: &Path) -> Result<()> {
    let tar_err = match std::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
    {
        Ok(s) if s.success() => return Ok(()),
        Ok(s) => format!("tar 退出码 {:?}", s.code()),
        Err(e) => format!("找不到系统 tar（{}）", e),
    };

    // Linux 的 GNU tar 不能解 .zip（Windows/macOS 的 bsdtar 可以）；退回 unzip。
    if is_zip_archive(archive) {
        match std::process::Command::new("unzip")
            .args(["-q", "-o"])
            .arg(archive)
            .arg("-d")
            .arg(dest)
            .status()
        {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => {
                return Err(CompileError::new(
                    "E135",
                    1,
                    1,
                    format!(
                        "解包 {} 失败：{}；unzip 退出码 {:?}",
                        archive.display(),
                        tar_err,
                        s.code()
                    ),
                )
                .with_hint("安装 unzip，或把归档改成 .tar.gz"));
            }
            Err(e) => {
                return Err(CompileError::new(
                    "E135",
                    1,
                    1,
                    format!(
                        "解包 {} 失败：{}；找不到系统 unzip（{}）",
                        archive.display(),
                        tar_err,
                        e
                    ),
                )
                .with_hint("Linux 解 .zip 需要 unzip；也可以改用 .tar.gz"));
            }
        }
    }

    Err(CompileError::new(
        "E135",
        1,
        1,
        format!("解包 {} 失败：{}", archive.display(), tar_err),
    )
    .with_hint("HTTP 来源支持 .tar/.tar.gz；Windows/macOS 的 tar 也能解 .zip，Linux 需要 unzip"))
}

/// GitHub 的 release zip 常带一层顶层目录；只有一层且根下没有清单时下钻。
fn unwrap_single_dir(dir: PathBuf) -> PathBuf {
    if dir.join(crate::pack_manifest::MANIFEST_NAME).is_file() {
        return dir;
    }
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return dir;
    };
    let mut children: Vec<PathBuf> = Vec::new();
    for ent in rd.flatten() {
        if ent.path().is_dir() {
            children.push(ent.path());
        }
    }
    if children.len() == 1 && children[0].join(crate::pack_manifest::MANIFEST_NAME).is_file() {
        children.remove(0)
    } else {
        dir
    }
}

/// 临时目录看守：离开作用域统一删除，失败也不留垃圾。
pub struct TempDirs {
    dirs: Vec<PathBuf>,
    seq: u32,
}

impl TempDirs {
    pub fn new() -> Self {
        Self {
            dirs: Vec::new(),
            seq: 0,
        }
    }

    pub fn alloc(&mut self) -> Result<PathBuf> {
        self.seq += 1;
        let dir = std::env::temp_dir().join(format!(
            "li-micc-{}-{}",
            std::process::id(),
            self.seq
        ));
        std::fs::create_dir_all(&dir)
            .map_err(|e| idx_err(1, format!("建不了临时目录 {}：{}", dir.display(), e)))?;
        self.dirs.push(dir.clone());
        Ok(dir)
    }
}

impl Drop for TempDirs {
    fn drop(&mut self) {
        for d in &self.dirs {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

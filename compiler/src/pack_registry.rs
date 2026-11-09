//! P2（N7）：`micc search` 与 `micc publish`。
//!
//! 原则与 H5 一致——**不跑服务器**：
//! - `publish` 只做两件事：把源码包打成归档放进「目录式仓库」，再把条目写回静态索引；
//! - `search` 只读索引（本地文件或 URL），打印名字、版本、来源与校验。
//!
//! 索引格式仍是 `砾索引.toml`（`索引版本 = 1` + `[包]`），复用 H5 的解析器与
//! `micc pack --源码` 的归档实现，不新增格式、不引入依赖。

use crate::error::{CompileError, Result};
use crate::pack_index::{self, TempDirs, INDEX_NAME};
use crate::pack_manifest::{Manifest, Version};
use crate::pack_resolve;
use std::path::{Component, Path, PathBuf};

fn usage_err(msg: impl Into<String>) -> CompileError {
    CompileError::new("E125", 1, 1, msg)
}

fn e138(msg: impl Into<String>) -> CompileError {
    CompileError::new("E138", 1, 1, msg)
}

pub fn cmd_search(args: &[String]) -> i32 {
    match do_search(args) {
        Ok(0) => 1,
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

/// 检索：关键词对包名做「全部包含」匹配（忽略大小写）；无关键词列出全部。
fn do_search(args: &[String]) -> Result<usize> {
    let mut index: Option<String> = None;
    let mut keys: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--索引" | "--index" => {
                i += 1;
                if i >= args.len() {
                    return Err(usage_err("--索引 后面要跟索引文件或 URL"));
                }
                index = Some(args[i].clone());
            }
            "-h" | "--help" => {
                return Err(usage_err("micc search [关键词...] [--索引 <索引文件|URL>]"))
            }
            other if other.starts_with('-') => {
                return Err(usage_err(format!("不认识的参数：{}", other)))
            }
            other => keys.push(other.to_lowercase()),
        }
        i += 1;
    }

    let source = index.unwrap_or_else(|| INDEX_NAME.to_string());
    let mut temp = TempDirs::new();
    let idx = pack_index::load_index(&source, &mut temp)?;

    let mut hits: Vec<&pack_index::IndexEntry> = Vec::new();
    for list in idx.by_name.values() {
        for e in list {
            let name = e.name.to_lowercase();
            if keys.iter().all(|k| name.contains(k)) {
                hits.push(e);
            }
        }
    }
    if hits.is_empty() {
        let what = if keys.is_empty() {
            "索引里没有包".to_string()
        } else {
            format!("关键词：{}", keys.join(" "))
        };
        println!("没有匹配的包（{}）", what);
        return Ok(0);
    }

    hits.sort_by(|a, b| a.name.cmp(&b.name).then(b.version.cmp(&a.version)));
    let count = hits.len();
    println!("索引：{}；命中 {} 条", source, count);
    for e in &hits {
        println!("{} {}　{}　{}", e.name, e.version, e.source, e.hash);
    }
    Ok(count)
}

pub fn cmd_publish(args: &[String]) -> i32 {
    match do_publish(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

/// 发布：源码包 → `<仓库>/<名字>/<名字>-<版本>.zip` → 索引条目。
///
/// 索引里的 校验 取自**归档解包后的目录哈希**，与 `install` 校验口径完全一致；
/// 同版本重复发布覆盖旧条目，保持幂等。
fn do_publish(args: &[String]) -> Result<()> {
    let mut dir = PathBuf::from(".");
    let mut repo: Option<PathBuf> = None;
    let mut index: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--目录" | "--dir" => {
                i += 1;
                if i >= args.len() {
                    return Err(usage_err("--目录 后面要跟包目录"));
                }
                dir = PathBuf::from(&args[i]);
            }
            "--仓库" | "--repo" => {
                i += 1;
                if i >= args.len() {
                    return Err(usage_err("--仓库 后面要跟仓库目录"));
                }
                repo = Some(PathBuf::from(&args[i]));
            }
            "--索引" | "--index" => {
                i += 1;
                if i >= args.len() {
                    return Err(usage_err("--索引 后面要跟索引文件"));
                }
                index = Some(PathBuf::from(&args[i]));
            }
            "-h" | "--help" => {
                return Err(usage_err(
                    "micc publish [--目录 <包目录>] [--索引 <索引文件>] [--仓库 <目录>]",
                ))
            }
            other => return Err(usage_err(format!("不认识的参数：{}", other))),
        }
        i += 1;
    }

    if !dir.is_dir() {
        return Err(usage_err(format!("包目录不存在：{}", dir.display())));
    }
    let m = Manifest::load(&dir)?;
    if let Some(product) = &m.product {
        return Err(e138(format!(
            "「{}」是二进制包（[包] 产物 = {}），进索引后 install 无法消费；请发布源码包",
            m.name, product
        ))
        .with_hint("去掉 [包] 里的 产物 再发布；二进制包请用 micc pack --归档 直接分发"));
    }
    if let Some(p) = &index {
        let text = p.to_string_lossy();
        if text.starts_with("http://") || text.starts_with("https://") {
            return Err(e138(format!(
                "发布目标 {} 是 HTTP 索引，只读；发布必须写本地 砾索引.toml",
                text
            ))
            .with_hint("把仓库取到本地发布，再自行上传索引与归档"));
        }
    }

    let (repo, index_path) = match index {
        Some(p) => {
            let base = p
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .map(|d| d.to_path_buf());
            (repo.or(base).unwrap_or_else(|| PathBuf::from(".")), p)
        }
        None => {
            let r = repo.unwrap_or_else(|| PathBuf::from("."));
            let p = r.join(INDEX_NAME);
            (r, p)
        }
    };
    let index_dir = index_path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let mut temp = TempDirs::new();
    let work = temp.alloc()?;
    let tmp_zip = work.join("包.zip");
    crate::pack::build_source_archive(&dir, &m, &tmp_zip)?;
    let bytes = std::fs::read(&tmp_zip)
        .map_err(|e| e138(format!("读不了临时归档 {}：{}", tmp_zip.display(), e)))?;

    // 校验以归档内容为准：先解包到临时目录，再算目录哈希（与 install 同一口径）。
    let entries = crate::zip::read(&bytes).map_err(|e| e138(format!("生成的归档不可读：{}", e)))?;
    let unpack = work.join("解包");
    std::fs::create_dir_all(&unpack)
        .map_err(|e| e138(format!("建不了临时目录 {}：{}", unpack.display(), e)))?;
    for ent in &entries {
        if ent.name.is_empty()
            || ent.name.starts_with('/')
            || ent.name.contains('\\')
            || ent.name.split('/').any(|s| s == "..")
        {
            return Err(e138(format!("归档条目名不合法：{}", ent.name)));
        }
        let p = unpack.join(&ent.name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| e138(format!("建不了目录 {}：{}", parent.display(), e)))?;
        }
        std::fs::write(&p, &ent.data)
            .map_err(|e| e138(format!("写不了 {}：{}", p.display(), e)))?;
    }
    let hash = pack_resolve::dir_hash(&unpack)?;

    let final_zip = repo
        .join(&m.name)
        .join(format!("{}-{}.zip", m.name, m.version));
    if let Some(parent) = final_zip.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| e138(format!("建不了仓库子目录 {}：{}", parent.display(), e)))?;
    }
    std::fs::write(&final_zip, &bytes)
        .map_err(|e| e138(format!("写不了 {}：{}", final_zip.display(), e)))?;

    // 读旧索引（存在才算），保留其它包的条目。
    let mut rows: Vec<(String, Version, String, String)> = Vec::new();
    if index_path.is_file() {
        let text = read_index(&index_path)?;
        let idx = pack_index::parse_index(&text, None)?;
        for list in idx.by_name.values() {
            for e in list {
                rows.push((e.name.clone(), e.version, e.source.clone(), e.hash.clone()));
            }
        }
    }
    rows.retain(|(n, v, _, _)| !(n == &m.name && *v == m.version));
    rows.push((
        m.name.clone(),
        m.version,
        relative_source(&index_dir, &final_zip),
        hash.clone(),
    ));
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    if let Some(parent) = index_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| e138(format!("建不了目录 {}：{}", parent.display(), e)))?;
        }
    }
    std::fs::write(&index_path, render_index(&rows))
        .map_err(|e| e138(format!("写不了 {}：{}", index_path.display(), e)))?;

    println!("发布完成：{} {}", m.name, m.version);
    println!("  归档：{}（{} B）", final_zip.display(), bytes.len());
    println!("  校验：{}", hash);
    println!("  索引：{}（共 {} 条）", index_path.display(), rows.len());
    if let Ok(dir_hash) = pack_resolve::dir_hash(&dir) {
        if dir_hash != hash {
            println!(
                "  注意：包目录还有未归档的文件，索引记录的是归档内容的哈希（目录 {}）",
                dir_hash
            );
        }
    }
    Ok(())
}

fn read_index(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| e138(format!("读不了 {}：{}", path.display(), e)))?;
    let mut text = String::from_utf8(bytes)
        .map_err(|_| e138(format!("{} 不是 UTF-8 文本", path.display())))?;
    if text.starts_with('\u{feff}') {
        text.drain(..'\u{feff}'.len_utf8());
    }
    Ok(text)
}

/// 归档相对索引目录的路径（正斜杠）。跨盘符时退回绝对路径。
fn relative_source(index_dir: &Path, archive: &Path) -> String {
    let base = std::fs::canonicalize(index_dir).unwrap_or_else(|_| index_dir.to_path_buf());
    let target = std::fs::canonicalize(archive).unwrap_or_else(|_| archive.to_path_buf());
    let b: Vec<Component> = base.components().collect();
    let t: Vec<Component> = target.components().collect();
    let common = b.iter().zip(t.iter()).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return target.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..b.len() {
        parts.push("..".to_string());
    }
    for c in &t[common..] {
        parts.push(c.as_os_str().to_string_lossy().to_string());
    }
    parts.join("/")
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn render_index(rows: &[(String, Version, String, String)]) -> String {
    let mut s = String::new();
    s.push_str("# 砾静态索引（`micc publish` 生成；手写同样支持）\n");
    s.push_str("索引版本 = 1\n\n[包]\n");
    for (name, ver, source, hash) in rows {
        s.push_str(&format!(
            "\"{}@{}\" = {{ 来源 = \"{}\", 校验 = \"{}\" }}\n",
            esc(name),
            ver,
            esc(source),
            esc(hash)
        ));
    }
    s
}

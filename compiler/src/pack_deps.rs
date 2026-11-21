//! `micc add / remove / update`：依赖管理命令。
//!
//! 只改两处：`砾包.toml` 的 `[依赖]` 与 `.砾/锁.toml`。选版、下载、哈希校验
//! 复用 `pack_resolve::install`；`--离线` 只走锁与 `.砾/包` 缓存，不访问索引/网络。
//! 编辑失败时回滚清单与锁，保证「冲突不改文件」。

use crate::error::{CompileError, Result};
use crate::pack_index::{load_index, TempDirs, VerifyKey};
use crate::pack_manifest::{
    lock_path, parse_manifest, read_lock, write_lock, Constraint, LockEntry, Manifest,
    MANIFEST_NAME,
};
use crate::pack_resolve::{self, CACHE_DIR, LOCK_DIR};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn cmd_add(args: &[String]) -> i32 {
    finish(do_add(args))
}

pub fn cmd_remove(args: &[String]) -> i32 {
    finish(do_remove(args))
}

pub fn cmd_update(args: &[String]) -> i32 {
    finish(do_update(args))
}

fn finish(result: Result<()>) -> i32 {
    match result {
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

fn e126(msg: impl Into<String>) -> CompileError {
    CompileError::new("E126", 1, 1, msg)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Add,
    Remove,
    Update,
}

struct Opts {
    dir: PathBuf,
    index: Option<String>,
    offline: bool,
    /// `add --路径`：写成本地路径依赖。
    dep_path: Option<String>,
    pubkey: Option<PathBuf>,
    identity: String,
    positional: Vec<String>,
}

fn parse_opts(args: &[String], mode: Mode) -> Result<Opts> {
    let usage = match mode {
        Mode::Add => {
            "用法：micc add <名字>[@约束] [--路径 <目录>] [--目录 <包>] [--索引 <索引|URL>] [--离线]"
        }
        Mode::Remove => "用法：micc remove <名字> [--目录 <包>] [--离线]",
        Mode::Update => "用法：micc update [名字] [--目录 <包>] [--索引 <索引|URL>] [--离线]",
    };
    let mut opts = Opts {
        dir: PathBuf::from("."),
        index: None,
        offline: false,
        dep_path: None,
        pubkey: None,
        identity: "li".to_string(),
        positional: Vec::new(),
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
            "--索引" | "--index" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--索引 后面要跟索引文件或 URL"));
                }
                opts.index = Some(args[i].clone());
            }
            "--路径" | "--path" if mode == Mode::Add => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--路径 后面要跟依赖包目录"));
                }
                opts.dep_path = Some(args[i].clone());
            }
            "--离线" | "--offline" => opts.offline = true,
            "--公钥" | "--pubkey" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--公钥 后面要跟 allowed_signers 文件"));
                }
                opts.pubkey = Some(PathBuf::from(&args[i]));
            }
            "--签名身份" | "--signer" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--签名身份 后面要跟身份名"));
                }
                opts.identity = args[i].clone();
            }
            "-h" | "--help" => return Err(e125(usage)),
            other if other.starts_with('-') => {
                return Err(e125(format!("不认识的参数：{}；{}", other, usage)));
            }
            other => opts.positional.push(other.to_string()),
        }
        i += 1;
    }
    Ok(opts)
}

fn verify_key(opts: &Opts) -> Option<VerifyKey> {
    opts.pubkey.as_ref().map(|path| VerifyKey {
        allowed_signers: path.clone(),
        identity: opts.identity.clone(),
    })
}

fn ensure_manifest(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Err(e125(format!("包目录不存在：{}", dir.display())));
    }
    if !dir.join(MANIFEST_NAME).is_file() {
        return Err(e125(format!("{} 里没有 {}", dir.display(), MANIFEST_NAME)));
    }
    Ok(())
}

fn read_text(dir: &Path) -> Result<(String, bool)> {
    let path = dir.join(MANIFEST_NAME);
    let bytes =
        std::fs::read(&path).map_err(|e| e125(format!("读不了 {}：{}", path.display(), e)))?;
    let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let text = String::from_utf8(bytes)
        .map_err(|_| e125(format!("{} 不是 UTF-8 文本", path.display())))?;
    let body = text.strip_prefix('\u{feff}').unwrap_or(&text).to_string();
    Ok((body, bom))
}

fn write_text(dir: &Path, body: &str, bom: bool) -> Result<()> {
    let path = dir.join(MANIFEST_NAME);
    let mut out = String::new();
    if bom {
        out.push('\u{feff}');
    }
    out.push_str(body);
    std::fs::write(&path, out).map_err(|e| e125(format!("写不了 {}：{}", path.display(), e)))
}

/// `[依赖]` 里一行 `名字 = { ... }`。
struct DepLine {
    path: Option<String>,
    constraint: Constraint,
}

impl DepLine {
    fn body(&self, name: &str) -> String {
        match &self.path {
            Some(path) => format!(
                "{} = {{ 路径 = \"{}\", 版本 = \"{}\" }}",
                name,
                path.replace('\\', "/"),
                self.constraint.text()
            ),
            None => format!("{} = {{ 版本 = \"{}\" }}", name, self.constraint.text()),
        }
    }
}

fn split_spec(text: &str) -> Result<(String, Option<Constraint>)> {
    let text = text.trim();
    if text.is_empty() {
        return Err(e125("依赖名不能为空"));
    }
    if let Some(at) = text.rfind('@') {
        let name = text[..at].trim();
        let rest = text[at + 1..].trim();
        if name.is_empty() || rest.is_empty() {
            return Err(e125(format!(
                "依赖规格「{}」要写成 名字@约束，例如 工具@^1.2.0",
                text
            )));
        }
        let constraint = Constraint::parse(rest).ok_or_else(|| {
            e125(format!(
                "版本约束「{}」不被支持；只支持 精确（1.2.3）/ ^（^1.2.3）/ *",
                rest
            ))
        })?;
        Ok((name.to_string(), Some(constraint)))
    } else {
        Ok((text.to_string(), None))
    }
}

fn leading_ws(line: &str) -> String {
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

fn dep_key(line: &str) -> Option<String> {
    let text = crate::pack_manifest::strip_comment(line).trim();
    if text.is_empty() || text.starts_with('[') {
        return None;
    }
    crate::pack_manifest::parse_field(text).map(|(key, _)| key)
}

/// 行内注释（含 `#`）；没有就返回空串。
fn comment_suffix(line: &str) -> String {
    let before = crate::pack_manifest::strip_comment(line);
    if before.len() == line.len() {
        return String::new();
    }
    let rest = &line[before.len()..];
    if before.ends_with(' ') || before.ends_with('\t') {
        rest.to_string()
    } else {
        format!(" {}", rest)
    }
}

fn section_bounds(lines: &[String], name: &str) -> Option<(usize, usize)> {
    let start = lines
        .iter()
        .position(|line| line.trim() == format!("[{}]", name))?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| {
            let t = line.trim();
            t.starts_with('[') && t.ends_with(']')
        })
        .map(|(index, _)| index)
        .unwrap_or(lines.len());
    Some((start, end))
}

fn upsert_dep(text: &str, name: &str, spec: &DepLine) -> String {
    let trailing = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let body = spec.body(name);
    match section_bounds(&lines, "依赖") {
        Some((start, end)) => {
            let existing = (start + 1..end).find(|&i| dep_key(&lines[i]).as_deref() == Some(name));
            if let Some(index) = existing {
                let indent = leading_ws(&lines[index]);
                let comment = comment_suffix(&lines[index]);
                lines[index] = format!("{}{}{}", indent, body, comment);
            } else {
                let indent = (start + 1..end)
                    .find(|&i| dep_key(&lines[i]).is_some())
                    .map(|i| leading_ws(&lines[i]))
                    .unwrap_or_default();
                let insert_at = (start + 1..end)
                    .rev()
                    .find(|&i| !lines[i].trim().is_empty())
                    .map(|i| i + 1)
                    .unwrap_or(start + 1);
                lines.insert(insert_at, format!("{}{}", indent, body));
            }
        }
        None => {
            if !lines.is_empty() && lines.last().map_or(false, |line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[依赖]".to_string());
            lines.push(body);
        }
    }
    let mut out = lines.join("\n");
    if trailing || text.is_empty() {
        out.push('\n');
    }
    out
}

fn remove_dep(text: &str, name: &str) -> Option<String> {
    let trailing = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let (start, end) = section_bounds(&lines, "依赖")?;
    let index = (start + 1..end).find(|&i| dep_key(&lines[i]).as_deref() == Some(name))?;
    lines.remove(index);
    let mut out = lines.join("\n");
    if trailing {
        out.push('\n');
    }
    Some(out)
}

fn default_constraint(opts: &Opts, name: &str) -> Result<Constraint> {
    let source = match &opts.index {
        Some(source) => source.clone(),
        None => {
            let path = opts.dir.join(crate::pack_index::INDEX_NAME);
            if !path.is_file() {
                return Err(CompileError::new(
                    "E132",
                    1,
                    1,
                    format!(
                        "add 需要 --索引，或把砾索引.toml 放到包目录：{} 不存在",
                        path.display()
                    ),
                ));
            }
            path.to_string_lossy().to_string()
        }
    };
    let mut temp = TempDirs::new();
    let verify = verify_key(opts);
    let index = load_index(&source, &mut temp, verify.as_ref())?;
    let entry = index.select(name, &Constraint::Any)?;
    Ok(Constraint::Caret(entry.version))
}

fn do_add(args: &[String]) -> Result<()> {
    let opts = parse_opts(args, Mode::Add)?;
    if opts.positional.len() != 1 {
        return Err(e125(
            "用法：micc add <名字>[@约束] [--路径 <目录>] [--目录 <包>] [--索引 <索引|URL>] [--离线]",
        ));
    }
    ensure_manifest(&opts.dir)?;
    let (name, given) = split_spec(&opts.positional[0])?;

    let spec = match (&opts.dep_path, given) {
        (Some(rel), constraint) => {
            let dep_dir = opts.dir.join(rel);
            if !dep_dir.is_dir() {
                return Err(e126(format!("路径依赖目录不存在：{}", dep_dir.display())));
            }
            let dep_manifest = Manifest::load(&dep_dir)?;
            if dep_manifest.name != name {
                return Err(e126(format!(
                    "依赖名「{}」与 {} 里的包名「{}」不一致",
                    name,
                    dep_dir.join(MANIFEST_NAME).display(),
                    dep_manifest.name
                )));
            }
            DepLine {
                path: Some(rel.clone()),
                constraint: constraint.unwrap_or(Constraint::Any),
            }
        }
        (None, Some(constraint)) => DepLine {
            path: None,
            constraint,
        },
        (None, None) => {
            if opts.offline {
                return Err(e126(
                    "离线 add 需要显式约束（名字@版本）或 --路径；选最新版要访问索引",
                ));
            }
            DepLine {
                path: None,
                constraint: default_constraint(&opts, &name)?,
            }
        }
    };

    let (text, bom) = read_text(&opts.dir)?;
    let new_text = upsert_dep(&text, &name, &spec);
    let manifest_path = opts.dir.join(MANIFEST_NAME);
    let parsed = parse_manifest(&manifest_path, &new_text)?;
    if !parsed.deps.iter().any(|(n, _)| n == &name) {
        return Err(e125(format!("写入后没有找到依赖「{}」", name)));
    }
    let entries = commit_edit(&opts, &new_text, bom, opts.offline)?;
    println!("已添加依赖：{}", spec.body(&name));
    print_entries(&opts.dir, &entries, opts.offline);
    Ok(())
}

fn do_remove(args: &[String]) -> Result<()> {
    let opts = parse_opts(args, Mode::Remove)?;
    if opts.positional.len() != 1 {
        return Err(e125("用法：micc remove <名字> [--目录 <包>] [--离线]"));
    }
    ensure_manifest(&opts.dir)?;
    let name = opts.positional[0].trim().to_string();
    let (text, bom) = read_text(&opts.dir)?;
    let new_text =
        remove_dep(&text, &name).ok_or_else(|| e125(format!("[依赖] 里没有「{}」", name)))?;
    parse_manifest(&opts.dir.join(MANIFEST_NAME), &new_text)?;
    let entries = commit_edit(&opts, &new_text, bom, opts.offline)?;
    println!("已移除依赖：{}", name);
    print_entries(&opts.dir, &entries, opts.offline);
    Ok(())
}

fn do_update(args: &[String]) -> Result<()> {
    let opts = parse_opts(args, Mode::Update)?;
    if opts.positional.len() > 1 {
        return Err(e125(
            "用法：micc update [名字] [--目录 <包>] [--索引 <索引|URL>] [--离线]",
        ));
    }
    ensure_manifest(&opts.dir)?;
    let manifest = Manifest::load(&opts.dir)?;
    if let Some(name) = opts.positional.first() {
        let in_manifest = manifest.deps.iter().any(|(n, _)| n == name);
        let in_lock = read_lock(&opts.dir)?
            .map(|entries| entries.iter().any(|e| &e.name == name))
            .unwrap_or(false);
        if !in_manifest && !in_lock {
            return Err(e125(format!("依赖树里没有「{}」", name)));
        }
    }

    if opts.offline {
        let entries = apply_offline(&opts.dir)?;
        print_entries(&opts.dir, &entries, true);
        return Ok(());
    }
    let verify = verify_key(&opts);
    let entries = pack_resolve::install(&opts.dir, opts.index.as_deref(), verify.as_ref())?;
    print_entries(&opts.dir, &entries, false);
    Ok(())
}

/// 写清单 → 解析/下载/写锁；任何一步失败都把清单与旧锁原样放回。
fn commit_edit(opts: &Opts, text: &str, bom: bool, offline: bool) -> Result<Vec<LockEntry>> {
    let manifest_path = opts.dir.join(MANIFEST_NAME);
    let original = std::fs::read(&manifest_path)
        .map_err(|e| e125(format!("读不了 {}：{}", manifest_path.display(), e)))?;
    let lock = lock_path(&opts.dir);
    let old_lock = std::fs::read(&lock).ok();

    let outcome = (|| {
        write_text(&opts.dir, text, bom)?;
        if offline {
            apply_offline(&opts.dir)
        } else {
            let verify = verify_key(opts);
            pack_resolve::install(&opts.dir, opts.index.as_deref(), verify.as_ref())
        }
    })();

    if outcome.is_err() {
        let _ = std::fs::write(&manifest_path, &original);
        match old_lock {
            Some(bytes) => {
                let _ = std::fs::write(&lock, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&lock);
            }
        }
    }
    outcome
}

/// 离线：只从清单 + 锁 + `.砾/包` 重建锁，再按 `verify_offline` 校验。
fn apply_offline(dir: &Path) -> Result<Vec<LockEntry>> {
    let entries = reachable_offline(dir)?;
    write_lock(dir, &entries)?;
    pack_resolve::verify_offline(dir)
}

fn sorted(
    deps: &[(String, crate::pack_manifest::DepSpec)],
) -> Vec<(String, crate::pack_manifest::DepSpec)> {
    let mut list = deps.to_vec();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list
}

/// 从根清单出发，只沿锁与缓存（路径依赖读源目录）收集还能到达的包。
fn reachable_offline(dir: &Path) -> Result<Vec<LockEntry>> {
    let root = Manifest::load(dir)?;
    root.check_native_abi()?;
    let lock = read_lock(dir)?.unwrap_or_default();
    let by_name: BTreeMap<String, LockEntry> = lock
        .into_iter()
        .map(|entry| (entry.name.clone(), entry))
        .collect();
    let mut out: BTreeMap<String, LockEntry> = BTreeMap::new();
    let mut visiting: Vec<String> = Vec::new();
    for (name, spec) in sorted(&root.deps) {
        walk_offline(dir, dir, &name, &spec, &by_name, &mut visiting, &mut out)?;
    }
    Ok(out.into_values().collect())
}

#[allow(clippy::too_many_arguments)]
fn walk_offline(
    root: &Path,
    base: &Path,
    name: &str,
    spec: &crate::pack_manifest::DepSpec,
    lock: &BTreeMap<String, LockEntry>,
    visiting: &mut Vec<String>,
    out: &mut BTreeMap<String, LockEntry>,
) -> Result<()> {
    if out.contains_key(name) {
        return Ok(());
    }
    if visiting.iter().any(|n| n == name) {
        let mut chain = visiting.clone();
        chain.push(name.to_string());
        return Err(e126(format!("依赖成环：{}", chain.join(" → "))));
    }
    let parent = if visiting.len() >= 2 {
        visiting[visiting.len() - 2].clone()
    } else {
        "根包".to_string()
    };
    visiting.push(name.to_string());
    let result = (|| {
        if let Some(rel) = &spec.path {
            let dep_dir = base.join(rel);
            let manifest = Manifest::load(&dep_dir)?;
            if manifest.name != name {
                return Err(e126(format!(
                    "依赖名「{}」与 {} 里的包名「{}」不一致",
                    name,
                    dep_dir.join(MANIFEST_NAME).display(),
                    manifest.name
                )));
            }
            if !spec.constraint.allows(manifest.version) {
                return Err(e126(format!(
                    "版本不满足：{} 需要「{}」{}，实际是 {}",
                    parent,
                    name,
                    spec.constraint.text(),
                    manifest.version
                )));
            }
            manifest.check_native_abi()?;
            let entry = LockEntry {
                name: name.to_string(),
                version: manifest.version,
                source: format!("path:{}", rel),
                hash: pack_resolve::dir_hash(&dep_dir)?,
            };
            out.insert(name.to_string(), entry);
            for (sub_name, sub_spec) in sorted(&manifest.deps) {
                walk_offline(root, &dep_dir, &sub_name, &sub_spec, lock, visiting, out)?;
            }
        } else {
            let Some(entry) = lock.get(name) else {
                return Err(e126(format!(
                    "离线模式缺包：锁里没有「{}」；先联网 micc install 一次",
                    name
                )));
            };
            if !spec.constraint.allows(entry.version) {
                return Err(e126(format!(
                    "版本冲突：需要「{}」{}，锁里是 {}",
                    name,
                    spec.constraint.text(),
                    entry.version
                )));
            }
            let cache = root
                .join(LOCK_DIR)
                .join(CACHE_DIR)
                .join(format!("{}-{}", entry.name, entry.version));
            if !cache.is_dir() {
                return Err(e126(format!(
                    "离线模式缺包：锁里要求「{}」{}，但目录 {} 不存在",
                    entry.name,
                    entry.version,
                    cache.display()
                )));
            }
            let manifest = Manifest::load(&cache)?;
            if manifest.name != entry.name || manifest.version != entry.version {
                return Err(e126(format!(
                    "离线模式校验失败：{} 的清单是「{}」{}，锁里是「{}」{}",
                    cache.display(),
                    manifest.name,
                    manifest.version,
                    entry.name,
                    entry.version
                )));
            }
            manifest.check_native_abi()?;
            out.insert(entry.name.clone(), entry.clone());
            for (sub_name, sub_spec) in sorted(&manifest.deps) {
                if sub_spec.path.is_some() {
                    return Err(e126(format!(
                        "索引包「{}」{} 声明了路径依赖「{}」；索引包只能依赖索引包",
                        manifest.name, manifest.version, sub_name
                    )));
                }
                walk_offline(root, &cache, &sub_name, &sub_spec, lock, visiting, out)?;
            }
        }
        Ok(())
    })();
    visiting.pop();
    result
}

fn print_entries(dir: &Path, entries: &[LockEntry], offline: bool) {
    if offline {
        println!("离线校验通过：{} 个包（未访问网络）", entries.len());
    } else {
        println!(
            "已安装 {} 个包到 {}",
            entries.len(),
            dir.join(LOCK_DIR).join(CACHE_DIR).display()
        );
    }
    for entry in entries {
        println!(
            "  {} {}　{}　{}",
            entry.name, entry.version, entry.source, entry.hash
        );
    }
    println!("锁文件：{}", lock_path(dir).display());
}

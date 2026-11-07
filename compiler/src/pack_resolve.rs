//! H4 的依赖解析、锁文件与离线目录（最小可用切分）。
//!
//! 规则（已拍板）：
//! - 只支持源码包（本地路径依赖），不做二进制包与索引；
//! - 按包名去重，一个包名只允许一个版本；菱形依赖只装一份；
//! - 离线模式只读 `.砾/锁.toml` 与 `.砾/包`，不碰网络、不读依赖声明里的路径。

use crate::error::{CompileError, Result};
use crate::pack_manifest::{read_lock, write_lock, Constraint, LockEntry, Manifest};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub const LOCK_DIR: &str = ".砾";
pub const CACHE_DIR: &str = "包";

#[derive(Clone, Debug)]
pub struct Resolved {
    pub manifest: Manifest,
    pub dir: PathBuf,
    /// `path:../工具` 或 `lock`，只用于锁文件与报错。
    pub source: String,
}

#[derive(Debug)]
pub struct Tree {
    pub root: Resolved,
    /// 拓扑序：被依赖的包在前，根包不在里面。
    pub deps: Vec<Resolved>,
}

fn pkg_err(line: u32, msg: impl Into<String>) -> CompileError {
    CompileError::new("E126", line, 1, msg)
}

/// 在线解析：从根包目录出发，按 `砾包.toml` 里的路径递归找依赖。
pub fn resolve_online(root_dir: &Path) -> Result<Tree> {
    let root_manifest = Manifest::load(root_dir)?;
    let root = Resolved {
        manifest: root_manifest,
        dir: root_dir.to_path_buf(),
        source: "root".to_string(),
    };
    let mut resolved: BTreeMap<String, Resolved> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut chain: Vec<String> = vec![root.manifest.name.clone()];
    let mut dirs: Vec<PathBuf> = vec![canonical(root_dir)?];

    for (dep_name, spec) in sorted_deps(&root.manifest.deps) {
        visit_online(
            &root_dir.join(&spec.path),
            &dep_name,
            &spec.constraint,
            format!("path:{}", spec.path),
            &mut chain,
            &mut dirs,
            &mut resolved,
            &mut order,
        )?;
    }

    let deps = order
        .iter()
        .filter_map(|n| resolved.get(n).cloned())
        .collect::<Vec<_>>();
    Ok(Tree { root, deps })
}

fn sorted_deps(
    deps: &[(String, crate::pack_manifest::DepSpec)],
) -> Vec<(String, crate::pack_manifest::DepSpec)> {
    let mut v = deps.to_vec();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[allow(clippy::too_many_arguments)]
fn visit_online(
    dir: &Path,
    dep_name: &str,
    constraint: &Constraint,
    source: String,
    chain: &mut Vec<String>,
    dirs: &mut Vec<PathBuf>,
    resolved: &mut BTreeMap<String, Resolved>,
    order: &mut Vec<String>,
) -> Result<()> {
    let canon = canonical(dir)?;
    if dirs.contains(&canon) {
        let mut path = chain.clone();
        path.push(dep_name.to_string());
        return Err(pkg_err(
            1,
            format!("依赖成环：{}", path.join(" → ")),
        ));
    }

    if let Some(prev) = resolved.get(dep_name) {
        if prev.dir != canon {
            return Err(pkg_err(
                1,
                format!(
                    "同名包「{}」来自两个目录：{} 与 {}；一个包名只允许一份",
                    dep_name,
                    prev.dir.display(),
                    dir.display()
                ),
            ));
        }
        if !constraint.allows(prev.manifest.version) {
            return Err(pkg_err(
                1,
                format!(
                    "版本冲突：{} 需要「{}」{}，但已经解析到 {}（来源链：{}）",
                    chain.last().map(String::as_str).unwrap_or("?"),
                    dep_name,
                    constraint.text(),
                    prev.manifest.version,
                    chain.join(" → ")
                ),
            ));
        }
        return Ok(());
    }

    let manifest = Manifest::load(dir)?;
    if manifest.name != dep_name {
        return Err(pkg_err(
            1,
            format!(
                "依赖名「{}」与 {} 里的包名「{}」不一致",
                dep_name,
                dir.join(crate::pack_manifest::MANIFEST_NAME).display(),
                manifest.name
            ),
        ));
    }
    if !constraint.allows(manifest.version) {
        return Err(pkg_err(
            1,
            format!(
                "版本不满足：{} 需要「{}」{}，实际是 {}（来源链：{}）",
                chain.last().map(String::as_str).unwrap_or("?"),
                dep_name,
                constraint.text(),
                manifest.version,
                chain.join(" → ")
            ),
        ));
    }

    chain.push(dep_name.to_string());
    dirs.push(canon.clone());
    for (sub_name, spec) in sorted_deps(&manifest.deps) {
        visit_online(
            &dir.join(&spec.path),
            &sub_name,
            &spec.constraint,
            format!("path:{}", spec.path),
            chain,
            dirs,
            resolved,
            order,
        )?;
    }
    dirs.pop();
    chain.pop();

    resolved.insert(
        dep_name.to_string(),
        Resolved {
            manifest,
            dir: canon,
            source,
        },
    );
    order.push(dep_name.to_string());
    Ok(())
}

/// 离线解析：只读锁文件与 `.砾/包/<名字>-<版本>`。
pub fn resolve_offline(root_dir: &Path) -> Result<Tree> {
    let lock = read_lock(root_dir)?.ok_or_else(|| {
        pkg_err(
            1,
            format!(
                "离线打包需要锁文件：{}；请先运行 micc install",
                root_dir.join(LOCK_DIR).join("锁.toml").display()
            ),
        )
    })?;
    let mut by_name: BTreeMap<String, LockEntry> = BTreeMap::new();
    for e in lock {
        by_name.insert(e.name.clone(), e);
    }

    let root_manifest = Manifest::load(root_dir)?;
    let mut loaded: BTreeMap<String, Resolved> = BTreeMap::new();
    for (name, spec) in sorted_deps(&root_manifest.deps) {
        load_cached(root_dir, &name, &by_name, &mut loaded)?;
        if let Some(entry) = by_name.get(&name) {
            if !spec.constraint.allows(entry.version) {
                return Err(pkg_err(
                    1,
                    format!(
                        "版本冲突：{} 需要「{}」{}，锁里是 {}",
                        root_manifest.name,
                        name,
                        spec.constraint.text(),
                        entry.version
                    ),
                ));
            }
        }
    }
    // 锁里多出来的包也要校验存在（同一个锁应该还原出同一棵树）。
    for name in by_name.keys() {
        if !loaded.contains_key(name) {
            load_cached(root_dir, name, &by_name, &mut loaded)?;
        }
    }

    // 拓扑排序（从根包的依赖出发），并校验每个包声明的依赖都在锁里。
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (name, _) in sorted_deps(&root_manifest.deps) {
        topo_offline(
            &name,
            &by_name,
            &loaded,
            &mut seen,
            &mut order,
            &mut vec![root_manifest.name.clone()],
        )?;
    }
    for name in by_name.keys() {
        if !seen.contains(name) {
            topo_offline(
                name,
                &by_name,
                &loaded,
                &mut seen,
                &mut order,
                &mut vec![root_manifest.name.clone()],
            )?;
        }
    }

    let deps = order
        .iter()
        .filter_map(|n| loaded.get(n).cloned())
        .collect::<Vec<_>>();
    Ok(Tree {
        root: Resolved {
            manifest: root_manifest,
            dir: root_dir.to_path_buf(),
            source: "root".to_string(),
        },
        deps,
    })
}

fn load_cached(
    root_dir: &Path,
    name: &str,
    by_name: &BTreeMap<String, LockEntry>,
    out: &mut BTreeMap<String, Resolved>,
) -> Result<()> {
    if out.contains_key(name) {
        return Ok(());
    }
    let Some(entry) = by_name.get(name) else {
        return Err(pkg_err(
            1,
            format!("离线模式缺包：锁里没有「{}」", name),
        ));
    };
    let dir = root_dir
        .join(LOCK_DIR)
        .join(CACHE_DIR)
        .join(format!("{}-{}", entry.name, entry.version));
    if !dir.is_dir() {
        return Err(pkg_err(
            1,
            format!(
                "离线模式缺包：锁里要求「{}」{}，但目录 {} 不存在",
                entry.name,
                entry.version,
                dir.display()
            ),
        ));
    }
    let manifest = Manifest::load(&dir)?;
    if manifest.name != entry.name || manifest.version != entry.version {
        return Err(pkg_err(
            1,
            format!(
                "缓存里的包与锁不一致：锁要求「{}」{}，缓存里是「{}」{}",
                entry.name, entry.version, manifest.name, manifest.version
            ),
        ));
    }
    let got = dir_hash(&dir)?;
    if got != entry.hash {
        return Err(pkg_err(
            1,
            format!(
                "离线模式校验失败：包「{}」的内容与锁不一致（锁 {}，实际 {}）",
                entry.name, entry.hash, got
            ),
        ));
    }
    out.insert(
        name.to_string(),
        Resolved {
            manifest,
            dir,
            source: entry.source.clone(),
        },
    );
    Ok(())
}

fn topo_offline(
    name: &str,
    by_name: &BTreeMap<String, LockEntry>,
    loaded: &BTreeMap<String, Resolved>,
    seen: &mut HashSet<String>,
    order: &mut Vec<String>,
    chain: &mut Vec<String>,
) -> Result<()> {
    if seen.contains(name) {
        return Ok(());
    }
    if chain.iter().any(|n| n == name) {
        let mut path = chain.clone();
        path.push(name.to_string());
        return Err(pkg_err(1, format!("依赖成环：{}", path.join(" → "))));
    }
    let Some(pkg) = loaded.get(name) else {
        return Err(pkg_err(
            1,
            format!("离线模式缺包：锁里没有「{}」", name),
        ));
    };
    chain.push(name.to_string());
    for (sub, spec) in sorted_deps(&pkg.manifest.deps) {
        let Some(sub_entry) = by_name.get(&sub) else {
            return Err(pkg_err(
                1,
                format!(
                    "离线模式缺包：「{}」依赖「{}」，但锁里没有（来源链：{}）",
                    name,
                    sub,
                    chain.join(" → ")
                ),
            ));
        };
        if !spec.constraint.allows(sub_entry.version) {
            return Err(pkg_err(
                1,
                format!(
                    "版本冲突：{} 需要「{}」{}，锁里是 {}（来源链：{}）",
                    name,
                    sub,
                    spec.constraint.text(),
                    sub_entry.version,
                    chain.join(" → ")
                ),
            ));
        }
        topo_offline(&sub, by_name, loaded, seen, order, chain)?;
    }
    chain.pop();
    seen.insert(name.to_string());
    order.push(name.to_string());
    Ok(())
}

/// 安装：把依赖源码复制进 `.砾/包/<名字>-<版本>`，并写锁文件。
pub fn install(root_dir: &Path) -> Result<Vec<LockEntry>> {
    let tree = resolve_online(root_dir)?;
    let mut entries = Vec::new();
    let cache_root = root_dir.join(LOCK_DIR).join(CACHE_DIR);
    for dep in &tree.deps {
        let dest = cache_root.join(format!("{}-{}", dep.manifest.name, dep.manifest.version));
        copy_package(&dep.dir, &dest)?;
        entries.push(LockEntry {
            name: dep.manifest.name.clone(),
            version: dep.manifest.version,
            source: dep.source.clone(),
            hash: dir_hash(&dest)?,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    write_lock(root_dir, &entries)?;
    Ok(entries)
}

/// 离线安装：只校验锁与缓存，不复制、不联网。
pub fn verify_offline(root_dir: &Path) -> Result<Vec<LockEntry>> {
    let tree = resolve_offline(root_dir)?;
    let mut entries = Vec::new();
    for dep in &tree.deps {
        entries.push(LockEntry {
            name: dep.manifest.name.clone(),
            version: dep.manifest.version,
            source: dep.source.clone(),
            hash: dir_hash(&dep.dir)?,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn canonical(dir: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(dir).map_err(|e| {
        pkg_err(
            1,
            format!("找不到依赖目录 {}：{}", dir.display(), e),
        )
    })
}

/// 目录内容校验：FNV-1a 64（非加密，只用来发现内容变化）。
pub fn dir_hash(dir: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort();
    let mut h: u64 = 0xcbf29ce484222325;
    for rel in &files {
        for b in rel.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= 0;
        h = h.wrapping_mul(0x100000001b3);
        let path = dir.join(rel);
        let data = std::fs::read(&path).map_err(|e| {
            pkg_err(1, format!("读不了 {}：{}", path.display(), e))
        })?;
        for b in data {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("fnv1a64:{:016x}", h))
}

fn skip_dir(name: &str) -> bool {
    matches!(name, ".砾" | ".git" | "build" | "target" | "node_modules")
}

fn collect_files(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let rd = std::fs::read_dir(dir)
        .map_err(|e| pkg_err(1, format!("读不了目录 {}：{}", dir.display(), e)))?;
    for ent in rd {
        let ent = ent.map_err(|e| pkg_err(1, format!("读不了目录项：{}", e)))?;
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if skip_dir(&name) {
                continue;
            }
            collect_files(base, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

fn copy_package(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| {
            pkg_err(1, format!("清不掉旧缓存 {}：{}", dest.display(), e))
        })?;
    }
    copy_dir(src, dest)
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)
        .map_err(|e| pkg_err(1, format!("建不了目录 {}：{}", dest.display(), e)))?;
    let rd = std::fs::read_dir(src)
        .map_err(|e| pkg_err(1, format!("读不了目录 {}：{}", src.display(), e)))?;
    for ent in rd {
        let ent = ent.map_err(|e| pkg_err(1, format!("读不了目录项：{}", e)))?;
        let name = ent.file_name().to_string_lossy().to_string();
        let from = ent.path();
        let to = dest.join(&name);
        if from.is_dir() {
            if skip_dir(&name) {
                continue;
            }
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|e| pkg_err(1, format!("复制 {} 失败：{}", from.display(), e)))?;
        }
    }
    Ok(())
}

//! H4/H5 的依赖解析、锁文件与离线目录。
//!
//! 规则（已拍板）：
//! - 源码包；依赖可写本地 `路径`，也可不写路径、从静态索引解析（H5）；
//! - 按包名去重，一个包名只允许一个版本；菱形依赖只装一份；
//! - 离线模式只读 `.砾/锁.toml` 与 `.砾/包`，不碰网络、不读依赖声明里的路径；
//! - 在线 `pack` 对索引依赖也只读锁与缓存，真正的下载由 `micc install` 负责。

use crate::error::{CompileError, Result};
use crate::pack_index::{load_index, verify_entry, Index, IndexEntry, TempDirs};
use crate::pack_manifest::{read_lock, write_lock, Constraint, DepSpec, LockEntry, Manifest};
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

/// 在线解析：从根包目录出发，路径依赖按目录递归；索引依赖读锁与缓存。
pub fn resolve_online(root_dir: &Path) -> Result<Tree> {
    let root_manifest = Manifest::load(root_dir)?;
    let root = Resolved {
        manifest: root_manifest,
        dir: root_dir.to_path_buf(),
        source: "root".to_string(),
    };
    let lock = read_lock(root_dir)?;
    let mut resolved: BTreeMap<String, Resolved> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut chain: Vec<String> = vec![root.manifest.name.clone()];
    let mut dirs: Vec<PathBuf> = vec![canonical(root_dir)?];

    for (dep_name, spec) in sorted_deps(&root.manifest.deps) {
        visit_online(
            root_dir,
            root_dir,
            &dep_name,
            &spec,
            &lock,
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

/// 在线遍历：路径依赖按目录走；索引依赖只读锁与缓存（下载由 install 负责）。
#[allow(clippy::too_many_arguments)]
fn visit_online(
    root_dir: &Path,
    base: &Path,
    dep_name: &str,
    spec: &DepSpec,
    lock: &Option<Vec<LockEntry>>,
    chain: &mut Vec<String>,
    dirs: &mut Vec<PathBuf>,
    resolved: &mut BTreeMap<String, Resolved>,
    order: &mut Vec<String>,
) -> Result<()> {
    let Some(rel) = &spec.path else {
        return visit_index_online(
            root_dir,
            dep_name,
            &spec.constraint,
            lock,
            chain,
            resolved,
            order,
        );
    };
    let dir = base.join(rel);
    let canon = canonical(&dir)?;
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
            let msg = if prev.source.starts_with("index:") {
                format!(
                    "同名包「{}」先由索引解析（{}），又声明了路径 {}；一个包名只允许一份",
                    dep_name,
                    prev.source,
                    dir.display()
                )
            } else {
                format!(
                    "同名包「{}」来自两个目录：{} 与 {}；一个包名只允许一份",
                    dep_name,
                    prev.dir.display(),
                    dir.display()
                )
            };
            return Err(pkg_err(1, msg));
        }
        if !spec.constraint.allows(prev.manifest.version) {
            return Err(pkg_err(
                1,
                format!(
                    "版本冲突：{} 需要「{}」{}，但已经解析到 {}（来源链：{}）",
                    chain.last().map(String::as_str).unwrap_or("?"),
                    dep_name,
                    spec.constraint.text(),
                    prev.manifest.version,
                    chain.join(" → ")
                ),
            ));
        }
        return Ok(());
    }

    let manifest = Manifest::load(&canon)?;
    if manifest.name != dep_name {
        return Err(pkg_err(
            1,
            format!(
                "依赖名「{}」与 {} 里的包名「{}」不一致",
                dep_name,
                canon.join(crate::pack_manifest::MANIFEST_NAME).display(),
                manifest.name
            ),
        ));
    }
    if !spec.constraint.allows(manifest.version) {
        return Err(pkg_err(
            1,
            format!(
                "版本不满足：{} 需要「{}」{}，实际是 {}（来源链：{}）",
                chain.last().map(String::as_str).unwrap_or("?"),
                dep_name,
                spec.constraint.text(),
                manifest.version,
                chain.join(" → ")
            ),
        ));
    }

    chain.push(dep_name.to_string());
    dirs.push(canon.clone());
    for (sub_name, sub_spec) in sorted_deps(&manifest.deps) {
        visit_online(
            root_dir,
            &canon,
            &sub_name,
            &sub_spec,
            lock,
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
            source: format!("path:{}", rel),
        },
    );
    order.push(dep_name.to_string());
    Ok(())
}

/// 在线遍历里的索引依赖：锁里必须有它，缓存必须存在且哈希一致。
fn visit_index_online(
    root_dir: &Path,
    dep_name: &str,
    constraint: &Constraint,
    lock: &Option<Vec<LockEntry>>,
    chain: &mut Vec<String>,
    resolved: &mut BTreeMap<String, Resolved>,
    order: &mut Vec<String>,
) -> Result<()> {
    if chain.iter().any(|n| n == dep_name) {
        let mut path = chain.clone();
        path.push(dep_name.to_string());
        return Err(pkg_err(1, format!("依赖成环：{}", path.join(" → "))));
    }
    let Some(entries) = lock else {
        return Err(pkg_err(
            1,
            format!(
                "「{}」来自索引，但还没有锁文件；请先运行 micc install",
                dep_name
            ),
        ));
    };
    let Some(entry) = entries.iter().find(|e| e.name == dep_name) else {
        return Err(pkg_err(
            1,
            format!(
                "「{}」来自索引，但锁里没有；请先运行 micc install",
                dep_name
            ),
        ));
    };
    if !constraint.allows(entry.version) {
        return Err(pkg_err(
            1,
            format!(
                "版本冲突：{} 需要「{}」{}，锁里是 {}（来源链：{}）",
                chain.last().map(String::as_str).unwrap_or("?"),
                dep_name,
                constraint.text(),
                entry.version,
                chain.join(" → ")
            ),
        ));
    }
    if let Some(prev) = resolved.get(dep_name) {
        if prev.manifest.version != entry.version {
            return Err(pkg_err(
                1,
                format!(
                    "同名包「{}」解析到两个版本：{} 与 {}；一个包名只允许一份",
                    dep_name, prev.manifest.version, entry.version
                ),
            ));
        }
        return Ok(());
    }
    let dir = root_dir
        .join(LOCK_DIR)
        .join(CACHE_DIR)
        .join(format!("{}-{}", entry.name, entry.version));
    if !dir.is_dir() {
        return Err(pkg_err(
            1,
            format!(
                "「{}」{} 还没装到 {}；请先运行 micc install",
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
                "校验失败：包「{}」的缓存内容与锁不一致（锁 {}，实际 {}）",
                entry.name, entry.hash, got
            ),
        ));
    }

    chain.push(dep_name.to_string());
    let mut result = Ok(());
    for (sub_name, sub_spec) in sorted_deps(&manifest.deps) {
        if sub_spec.path.is_some() {
            result = Err(pkg_err(
                1,
                format!(
                    "缓存里的包「{}」声明了路径依赖「{}」；索引包只能依赖索引包",
                    dep_name, sub_name
                ),
            ));
            break;
        }
        if let Err(e) = visit_index_online(
            root_dir,
            &sub_name,
            &sub_spec.constraint,
            lock,
            chain,
            resolved,
            order,
        ) {
            result = Err(e);
            break;
        }
    }
    chain.pop();
    result?;

    resolved.insert(
        dep_name.to_string(),
        Resolved {
            manifest,
            dir,
            source: entry.source.clone(),
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

/// 安装：路径依赖从目录复制；索引依赖从静态索引挑版本、取内容、校验哈希。
/// 所有依赖都落到 `.砾/包/<名字>-<版本>`，最后写锁文件。
pub fn install(root_dir: &Path, index_source: Option<&str>) -> Result<Vec<LockEntry>> {
    let root_manifest = Manifest::load(root_dir)?;
    let mut st = Installer {
        root_dir,
        cache_root: root_dir.join(LOCK_DIR).join(CACHE_DIR),
        index_source,
        index: None,
        temp: TempDirs::new(),
        installed: BTreeMap::new(),
        order: Vec::new(),
        chain: vec![root_manifest.name.clone()],
    };
    for (name, spec) in sorted_deps(&root_manifest.deps) {
        st.visit(root_dir, &name, &spec)?;
    }

    let mut entries: Vec<LockEntry> = st
        .order
        .iter()
        .filter_map(|n| st.installed.get(n))
        .map(|p| LockEntry {
            name: p.manifest.name.clone(),
            version: p.manifest.version,
            source: p.source.clone(),
            hash: p.hash.clone(),
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    write_lock(root_dir, &entries)?;
    Ok(entries)
}

struct Installed {
    manifest: Manifest,
    source: String,
    hash: String,
}

struct Installer<'a> {
    root_dir: &'a Path,
    cache_root: PathBuf,
    index_source: Option<&'a str>,
    index: Option<Index>,
    temp: TempDirs,
    installed: BTreeMap<String, Installed>,
    order: Vec<String>,
    chain: Vec<String>,
}

impl<'a> Installer<'a> {
    fn visit(&mut self, base: &Path, dep_name: &str, spec: &DepSpec) -> Result<()> {
        if let Some(prev) = self.installed.get(dep_name) {
            if !spec.constraint.allows(prev.manifest.version) {
                return Err(pkg_err(
                    1,
                    format!(
                        "版本冲突：{} 需要「{}」{}，但已经解析到 {}（来源链：{}）",
                        self.chain.last().map(String::as_str).unwrap_or("?"),
                        dep_name,
                        spec.constraint.text(),
                        prev.manifest.version,
                        self.chain.join(" → ")
                    ),
                ));
            }
            return Ok(());
        }
        if self.chain.iter().any(|n| n == dep_name) {
            let mut path = self.chain.clone();
            path.push(dep_name.to_string());
            return Err(pkg_err(1, format!("依赖成环：{}", path.join(" → "))));
        }

        let (manifest, source, staging, from_index) = match &spec.path {
            Some(rel) => {
                let dir = canonical(&base.join(rel))?;
                let manifest = Manifest::load(&dir)?;
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
                if !spec.constraint.allows(manifest.version) {
                    return Err(pkg_err(
                        1,
                        format!(
                            "版本不满足：{} 需要「{}」{}，实际是 {}（来源链：{}）",
                            self.chain.last().map(String::as_str).unwrap_or("?"),
                            dep_name,
                            spec.constraint.text(),
                            manifest.version,
                            self.chain.join(" → ")
                        ),
                    ));
                }
                (manifest, format!("path:{}", rel), dir, false)
            }
            None => {
                let entry = self.pick(dep_name, &spec.constraint)?;
                let staging = self.prepare(&entry)?;
                let manifest = Manifest::load(&staging)?;
                if manifest.name != entry.name || manifest.version != entry.version {
                    return Err(CompileError::new(
                        "E136",
                        1,
                        1,
                        format!(
                            "索引条目「{}」{} 与包内容的清单不一致：实际是「{}」{}",
                            entry.name, entry.version, manifest.name, manifest.version
                        ),
                    ));
                }
                (
                    manifest,
                    format!("index:{}", entry.source),
                    staging,
                    true,
                )
            }
        };

        let dest = self
            .cache_root
            .join(format!("{}-{}", manifest.name, manifest.version));
        copy_package(&staging, &dest)?;
        let hash = dir_hash(&dest)?;

        self.chain.push(dep_name.to_string());
        let mut result = Ok(());
        for (sub_name, sub_spec) in sorted_deps(&manifest.deps) {
            if from_index && sub_spec.path.is_some() {
                result = Err(pkg_err(
                    1,
                    format!(
                        "索引包「{}」{} 声明了路径依赖「{}」；索引包只能依赖索引包",
                        manifest.name, manifest.version, sub_name
                    ),
                ));
                break;
            }
            if let Err(e) = self.visit(&staging, &sub_name, &sub_spec) {
                result = Err(e);
                break;
            }
        }
        self.chain.pop();
        result?;

        self.installed.insert(
            dep_name.to_string(),
            Installed {
                manifest,
                source,
                hash,
            },
        );
        self.order.push(dep_name.to_string());
        Ok(())
    }

    /// 懒加载索引：优先 `--索引`，否则包目录下的 `砾索引.toml`。
    fn ensure_index(&mut self) -> Result<()> {
        if self.index.is_some() {
            return Ok(());
        }
        let source = match self.index_source {
            Some(s) => s.to_string(),
            None => {
                let p = self.root_dir.join(crate::pack_index::INDEX_NAME);
                if !p.is_file() {
                    return Err(CompileError::new(
                        "E132",
                        1,
                        1,
                        format!(
                            "有依赖要从索引解析，但没给 --索引，且 {} 不存在",
                            p.display()
                        ),
                    )
                    .with_hint(
                        "用 micc install --索引 <索引文件|URL>，或把 砾索引.toml 放到包目录里",
                    ));
                }
                p.to_string_lossy().to_string()
            }
        };
        self.index = Some(load_index(&source, &mut self.temp)?);
        Ok(())
    }

    fn pick(&mut self, name: &str, constraint: &Constraint) -> Result<IndexEntry> {
        self.ensure_index()?;
        let index = self.index.as_ref().expect("索引已加载");
        index.select(name, constraint).cloned()
    }

    fn prepare(&mut self, entry: &IndexEntry) -> Result<PathBuf> {
        let index = self.index.as_ref().expect("索引已加载");
        let dir = index.materialize(entry, &mut self.temp)?;
        verify_entry(entry, &dir)?;
        Ok(dir)
    }
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

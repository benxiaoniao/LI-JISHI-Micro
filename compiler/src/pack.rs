//! H4 最小可用切分：`micc pack` 与 `micc install`。
//!
//! 只做源码包：多个本地包在源码层合并成一个 Program，产出单个 `.jmb`。
//! 不改 JMB 格式、不改 VM，目标侧新增 0 B。

use crate::codegen;
use crate::error::{CompileError, Result};
use crate::lexer;
use crate::native_spec;
use crate::pack_merge::{self, Unit};
use crate::pack_resolve::{self, Resolved, Tree};
use crate::parser::Parser;
use crate::writer;
use std::collections::HashSet;
use std::path::PathBuf;

/// 堆预算只允许用到 80%，留出运行期波动空间。
const HEAP_USE_PERCENT: u64 = 80;

struct Opts {
    dir: PathBuf,
    out: Option<PathBuf>,
    offline: bool,
    strip_debug: bool,
}

fn parse_opts(args: &[String], allow_out: bool) -> Result<Opts> {
    let mut dir = PathBuf::from(".");
    let mut out = None;
    let mut offline = false;
    let mut strip_debug = false;
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
            "-o" | "--输出" | "--out" if allow_out => {
                i += 1;
                if i >= args.len() {
                    return Err(usage_err("-o 后面要跟输出文件"));
                }
                out = Some(PathBuf::from(&args[i]));
            }
            "--离线" | "--offline" => offline = true,
            "--strip-debug" => strip_debug = true,
            "-h" | "--help" => return Err(usage_err("micc pack / micc install")),
            other => return Err(usage_err(format!("不认识的参数：{}", other))),
        }
        i += 1;
    }
    Ok(Opts {
        dir,
        out,
        offline,
        strip_debug,
    })
}

fn usage_err(msg: impl Into<String>) -> CompileError {
    CompileError::new("E125", 1, 1, msg)
}

pub fn cmd_pack(args: &[String]) -> i32 {
    match do_pack(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

pub fn cmd_install(args: &[String]) -> i32 {
    match do_install(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

fn do_pack(args: &[String]) -> Result<()> {
    let opts = parse_opts(args, true)?;
    let tree = load_tree(&opts)?;

    let mut units: Vec<Unit> = Vec::new();
    for dep in &tree.deps {
        let stmts = parse_entry(dep)?;
        let unit = Unit::new(
            dep.manifest.name.clone(),
            false,
            stmts,
            dep.manifest.exports.clone(),
            entry_display(dep),
        );
        pack_merge::check_dependency_top_level(&unit)?;
        if let Some(exports) = &unit.exports {
            for n in exports {
                if !unit.globals.contains(n) {
                    return Err(CompileError::new(
                        "E125",
                        1,
                        1,
                        format!(
                            "包「{}」的 [导出] 列了「{}」，但它的顶层没有这个名字",
                            unit.name, n
                        ),
                    ));
                }
            }
        }
        units.push(unit);
    }
    let root_stmts = parse_entry(&tree.root)?;
    units.push(Unit::new(
        tree.root.manifest.name.clone(),
        true,
        root_stmts,
        None,
        entry_display(&tree.root),
    ));

    let native_union = check_native(&tree)?;

    let merged = pack_merge::merge(&mut units)?;
    let mut imports: HashSet<String> = HashSet::new();
    pack_merge::collect_imports(&merged.stmts, &mut imports);
    for m in &imports {
        if !native_union.contains(m) {
            return Err(CompileError::new(
                "E129",
                1,
                1,
                format!(
                    "源码里 `导入 {}`，但所有包的 [包] 原生 都没声明这张表",
                    m
                ),
            )
            .with_hint(format!(
                "在需要它的包的 砾包.toml 里写：原生 = [\"{}\"]",
                m
            )));
        }
    }

    let prog = codegen::compile(
        &merged.stmts,
        merged.files.clone(),
        &merged.spans,
    )?;

    let class_fields = pack_merge::class_field_counts(&merged.stmts);
    let heap = pack_merge::heap_estimate(&merged.stmts, &class_fields);
    let budget = tree.root.manifest.heap_budget as u64;
    if heap * 100 > budget * HEAP_USE_PERCENT {
        return Err(CompileError::new(
            "E130",
            1,
            1,
            format!(
                "静态堆下界 {} B 超过预算 {} B 的 {}%（{} B）；请减少容器字面量/实例，或调大 [包] 堆预算",
                heap,
                budget,
                HEAP_USE_PERCENT,
                budget * HEAP_USE_PERCENT / 100
            ),
        ));
    }

    let enc = writer::write(&prog, !opts.strip_debug)?;
    // 上限守的是进芯片的体积：调试段在烧录前会被剥掉，不计入。
    let limit = tree.root.manifest.bytecode_limit.min(65535);
    if enc.base_len > limit {
        return Err(CompileError::new(
            "E130",
            1,
            1,
            format!(
                "打包后 .jmb 为 {} B（剥离调试段后），超过 [包] 字节码上限 {} B",
                enc.base_len, limit
            ),
        ));
    }

    let out_path = opts
        .out
        .clone()
        .unwrap_or_else(|| opts.dir.join(format!("{}.jmb", tree.root.manifest.name)));
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                usage_err(format!("建不了目录 {}：{}", parent.display(), e))
            })?;
        }
    }
    std::fs::write(&out_path, &enc.bytes)
        .map_err(|e| usage_err(format!("写不了 {}：{}", out_path.display(), e)))?;

    println!(
        "打包完成：{} {}（{}）",
        tree.root.manifest.name,
        tree.root.manifest.version,
        if opts.offline { "离线" } else { "在线" }
    );
    if tree.deps.is_empty() {
        println!("  依赖：无");
    } else {
        let list = tree
            .deps
            .iter()
            .map(|d| format!("{} {}", d.manifest.name, d.manifest.version))
            .collect::<Vec<_>>()
            .join("、");
        println!("  依赖：{}（共 {} 个，源码级合并）", list, tree.deps.len());
    }
    println!(
        "  名字表：{} / 255　常量：{} / 255　函数：{} / 255　类：{} / 255",
        prog.names.len(),
        prog.consts.len(),
        prog.codes.len(),
        prog.classes.len()
    );
    if enc.debug_len > 0 {
        println!(
            "  字节码：{} B（剥离调试段后 {} B，上限 {} B；调试段另计 {} B）",
            enc.bytes.len(),
            enc.base_len,
            limit,
            enc.debug_len
        );
    } else {
        println!("  字节码：{} B（无调试段，上限 {} B）", enc.bytes.len(), limit);
    }
    println!(
        "  堆下界估计：{} B（预算 {} B，允许 {} B）",
        heap,
        budget,
        budget * HEAP_USE_PERCENT / 100
    );
    if native_union.is_empty() {
        println!("  原生并集：无");
    } else {
        let mut v = native_union.iter().cloned().collect::<Vec<_>>();
        v.sort();
        println!("  原生并集：{}", v.join("、"));
    }
    println!("  输出：{}", out_path.display());
    Ok(())
}

fn load_tree(opts: &Opts) -> Result<Tree> {
    if !opts.dir.is_dir() {
        return Err(usage_err(format!(
            "包目录不存在：{}",
            opts.dir.display()
        )));
    }
    if opts.offline {
        pack_resolve::resolve_offline(&opts.dir)
    } else {
        pack_resolve::resolve_online(&opts.dir)
    }
}

/// 调试表里的源文件名：尽量相对当前目录，统一用 `/`。
fn entry_display(pkg: &Resolved) -> String {
    let path = pkg.dir.join(&pkg.manifest.entry);
    let abs = path.canonicalize().unwrap_or(path);
    let cwd = std::env::current_dir().unwrap_or_default();
    let rel = abs.strip_prefix(&cwd).unwrap_or(&abs);
    rel.to_string_lossy().replace('\\', "/")
}

fn parse_entry(pkg: &Resolved) -> Result<Vec<crate::ast::Stmt>> {
    let path = pkg.dir.join(&pkg.manifest.entry);
    let src = std::fs::read_to_string(&path).map_err(|e| {
        usage_err(format!(
            "读不了包「{}」的入口 {}：{}",
            pkg.manifest.name,
            path.display(),
            e
        ))
    })?;
    let src = src.strip_prefix('\u{feff}').unwrap_or(&src);
    let tokens = lexer::tokenize(src).map_err(|e| in_pkg(e, &pkg.manifest.name))?;
    let mut p = Parser::new(tokens);
    p.parse_program().map_err(|e| in_pkg(e, &pkg.manifest.name))
}

fn in_pkg(mut e: CompileError, pkg: &str) -> CompileError {
    e.msg = format!("包「{}」：{}", pkg, e.msg);
    e
}

/// 校验 Native ABI 与原生模块并集。
fn check_native(tree: &Tree) -> Result<HashSet<String>> {
    let mut union: HashSet<String> = HashSet::new();
    for pkg in tree.deps.iter().chain(std::iter::once(&tree.root)) {
        let name = &pkg.manifest.name;
        if pkg.manifest.native_abi != native_spec::NATIVE_ABI_VERSION {
            return Err(CompileError::new(
                "E129",
                1,
                1,
                format!(
                    "包「{}」要求 Native ABI v{}，当前编译器/运行时是 v{}；ABI 不一致不能打包",
                    name,
                    pkg.manifest.native_abi,
                    native_spec::NATIVE_ABI_VERSION
                ),
            ));
        }
        for m in &pkg.manifest.native_mods {
            if native_spec::module(m).is_none() {
                return Err(CompileError::new(
                    "E129",
                    1,
                    1,
                    format!("包「{}」声明了不存在的原生模块「{}」", name, m),
                )
                .with_hint(format!("v0 可用：{}", native_spec::module_names())));
            }
            union.insert(m.clone());
        }
    }
    Ok(union)
}

fn do_install(args: &[String]) -> Result<()> {
    let opts = parse_opts(args, false)?;
    if !opts.dir.is_dir() {
        return Err(usage_err(format!(
            "包目录不存在：{}",
            opts.dir.display()
        )));
    }
    if opts.offline {
        let entries = pack_resolve::verify_offline(&opts.dir)?;
        println!(
            "离线校验通过：{} 个包（锁文件与缓存一致，未访问网络）",
            entries.len()
        );
        for e in entries {
            println!("  {} {}　{}", e.name, e.version, e.hash);
        }
        return Ok(());
    }

    let entries = pack_resolve::install(&opts.dir)?;
    println!(
        "已安装 {} 个包到 {}",
        entries.len(),
        opts.dir.join(".砾").join("包").display()
    );
    for e in &entries {
        println!("  {} {}　{}", e.name, e.version, e.source);
    }
    println!("锁文件：{}", crate::pack_manifest::lock_path(&opts.dir).display());
    Ok(())
}

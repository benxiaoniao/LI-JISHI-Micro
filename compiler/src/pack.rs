//! H4 最小可用切分：`micc pack` 与 `micc install`。
//!
//! 只做源码包：多个本地包在源码层合并成一个 Program，产出单个 `.jmb`。
//! 不改 JMB 格式、不改 VM，目标侧新增 0 B。

use crate::codegen;
use crate::error::{CompileError, Result};
use crate::lexer;
use crate::native_spec;
use crate::pack_manifest;
use crate::pack_merge::{self, Unit};
use crate::pack_resolve::{self, Resolved, Tree};
use crate::parser::Parser;
use crate::writer;
use crate::zip;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 堆预算只允许用到 80%，留出运行期波动空间。
const HEAP_USE_PERCENT: u64 = 80;
/// 归档里的清单文件名（固定，不放包名，便于 `micc verify` 直接找）。
const LIST_NAME: &str = "清单.toml";
const MANIFEST_NAME_LOCAL: &str = "砾包.toml";

struct Opts {
    dir: PathBuf,
    out: Option<PathBuf>,
    offline: bool,
    strip_debug: bool,
    no_assert: bool,
    index: Option<String>,
    archive: bool,
    /// `--源码`：归档内容换成源码包（`砾包.toml` + `.jsh`），供 `micc install` 消费。
    source_archive: bool,
}

fn parse_opts(args: &[String], allow_out: bool, allow_index: bool) -> Result<Opts> {
    let mut dir = PathBuf::from(".");
    let mut out = None;
    let mut offline = false;
    let mut archive = false;
    let mut source_archive = false;
    let mut strip_debug = false;
    let mut no_assert = false;
    let mut index = None;
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
            "--索引" | "--index" if allow_index => {
                i += 1;
                if i >= args.len() {
                    return Err(usage_err("--索引 后面要跟索引文件或 URL"));
                }
                index = Some(args[i].clone());
            }
            "--离线" | "--offline" => offline = true,
            "--归档" | "--archive" => archive = true,
            "--源码" | "--source-archive" => {
                source_archive = true;
                archive = true;
            }
            "--strip-debug" => strip_debug = true,
            "--no-assert" => no_assert = true,
            "-h" | "--help" => {
                return Err(usage_err("micc pack / micc install / micc hash / micc verify"))
            }
            other => return Err(usage_err(format!("不认识的参数：{}", other))),
        }
        i += 1;
    }
    Ok(Opts {
        dir,
        out,
        offline,
        strip_debug,
        no_assert,
        index,
        archive,
        source_archive,
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

/// `micc hash`：算目录内容哈希，供发布者手写静态索引时填 校验。
pub fn cmd_hash(args: &[String]) -> i32 {
    match do_hash(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

fn do_hash(args: &[String]) -> Result<()> {
    let opts = parse_opts(args, false, false)?;
    if !opts.dir.is_dir() {
        return Err(usage_err(format!("包目录不存在：{}", opts.dir.display())));
    }
    println!("{}", pack_resolve::dir_hash(&opts.dir)?);
    Ok(())
}

fn do_pack(args: &[String]) -> Result<()> {
    let opts = parse_opts(args, true, false)?;
    let tree = load_tree(&opts)?;
    if opts.source_archive {
        return pack_source_archive(&opts, &tree);
    }
    if let Some(product) = tree.root.manifest.product.clone() {
        return pack_binary(&opts, &tree, &product);
    }

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
        opts.no_assert,
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

    let stem = format!(
        "{}-{}",
        tree.root.manifest.name,
        ver_text(&tree.root.manifest.version)
    );
    let out_path = opts.out.clone().unwrap_or_else(|| {
        if opts.archive {
            opts.dir.join(format!("{}.zip", stem))
        } else {
            opts.dir.join(format!("{}.jmb", tree.root.manifest.name))
        }
    });
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                usage_err(format!("建不了目录 {}：{}", parent.display(), e))
            })?;
        }
    }
    if opts.archive {
        let pkg_toml = std::fs::read(opts.dir.join(MANIFEST_NAME_LOCAL)).map_err(|e| {
            usage_err(format!("读不了 {}：{}", MANIFEST_NAME_LOCAL, e))
        })?;
        let product = format!("{}.jmb", tree.root.manifest.name);
        let list = list_text(&tree.root.manifest, &product, &enc.bytes);
        let iface = read_optional(&opts.dir, IFACE_NAME);
        let bytes = archive_bytes(
            &tree.root.manifest,
            &product,
            &enc.bytes,
            &pkg_toml,
            iface.as_deref(),
        );
        std::fs::write(&out_path, bytes)
            .map_err(|e| usage_err(format!("写不了 {}：{}", out_path.display(), e)))?;
        // 清单与产物同时落一份到输出目录：`micc verify <包目录>` 与归档走同一份元数据。
        write_list_beside(&out_path, &list)?;
        let beside = out_path.parent().unwrap_or_else(|| Path::new(".")).join(&product);
        std::fs::write(&beside, &enc.bytes)
            .map_err(|e| usage_err(format!("写不了 {}：{}", beside.display(), e)))?;
    } else {
        std::fs::write(&out_path, &enc.bytes)
            .map_err(|e| usage_err(format!("写不了 {}：{}", out_path.display(), e)))?;
    }

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
    if opts.archive {
        println!(
            "  归档：{}（{} + {}.jmb + {}）",
            out_path.display(),
            MANIFEST_NAME_LOCAL,
            tree.root.manifest.name,
            LIST_NAME
        );
    } else {
        println!("  输出：{}", out_path.display());
    }
    Ok(())
}

/// 版本的 `主.次.修订` 文本（`Version` 本身只有三段整数）。
fn ver_text(v: &pack_manifest::Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// 校验 `micc verify`：包目录或归档。目录模式读三件同名文件，归档模式解 ZIP。
pub fn cmd_verify(args: &[String]) -> i32 {
    match do_verify(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

fn do_verify(args: &[String]) -> Result<()> {
    if args.len() != 1 {
        return Err(usage_err("用法：micc verify <包目录|归档.zip>"));
    }
    let path = Path::new(&args[0]);
    let entries = if path.is_dir() {
        dir_entries(path)?
    } else {
        let bytes = std::fs::read(path)
            .map_err(|e| usage_err(format!("读不了 {}：{}", path.display(), e)))?;
        zip::read(&bytes).map_err(|m| e137(format!("{}：{}", path.display(), m)))?
    };
    println!("{}", verify_entries(&entries)?);
    Ok(())
}

/// 把包目录读成「归档条目」形状，复用同一套校验。
fn dir_entries(dir: &Path) -> Result<Vec<zip::Entry>> {
    let manifest = pack_manifest::Manifest::load(dir)?;
    let mut out = Vec::new();
    let pkg = std::fs::read(dir.join(MANIFEST_NAME_LOCAL))
        .map_err(|e| usage_err(format!("读不了 {}：{}", MANIFEST_NAME_LOCAL, e)))?;
    out.push(zip::Entry {
        name: MANIFEST_NAME_LOCAL.to_string(),
        data: pkg,
    });
    let list = dir.join(LIST_NAME);
    if !list.is_file() {
        return Err(e137(format!(
            "{} 下没有 {}；这不是归档包，先运行 micc pack --归档",
            dir.display(),
            LIST_NAME
        )));
    }
    out.push(zip::Entry {
        name: LIST_NAME.to_string(),
        data: std::fs::read(&list)
            .map_err(|e| usage_err(format!("读不了 {}：{}", list.display(), e)))?,
    });
    let product = manifest
        .product
        .clone()
        .unwrap_or_else(|| format!("{}.jmb", manifest.name));
    let ppath = dir.join(&product);
    if !ppath.is_file() {
        return Err(e137(format!("包里没有产物 {}", ppath.display())));
    }
    out.push(zip::Entry {
        name: product,
        data: std::fs::read(&ppath)
            .map_err(|e| usage_err(format!("读不了 {}：{}", ppath.display(), e)))?,
    });
    if let Some(iface) = read_optional(dir, IFACE_NAME) {
        out.push(zip::Entry {
            name: IFACE_NAME.to_string(),
            data: iface,
        });
    }
    Ok(out)
}

fn verify_entries(entries: &[zip::Entry]) -> Result<String> {
    let find = |n: &str| entries.iter().find(|e| e.name == n).map(|e| e.data.as_slice());
    let toml = find(MANIFEST_NAME_LOCAL).ok_or_else(|| e137("归档里没有 砾包.toml"))?;
    let toml_text = std::str::from_utf8(toml).map_err(|_| e137("砾包.toml 不是 UTF-8 文本"))?;
    let toml_text = toml_text.strip_prefix('\u{feff}').unwrap_or(toml_text);
    let m = pack_manifest::parse_manifest(Path::new(MANIFEST_NAME_LOCAL), toml_text)?;

    let list = match find(LIST_NAME) {
        Some(v) => v,
        None => {
            if entries.iter().any(|e| e.name.ends_with(".jsh")) {
                return Err(e137(
                    "这是源码归档（只有 .jsh，没有 清单.toml）；用 micc install 安装，或解包后直接使用",
                ));
            }
            return Err(e137("归档里没有 清单.toml"));
        }
    };
    let list_text = std::str::from_utf8(list).map_err(|_| e137("清单.toml 不是 UTF-8 文本"))?;
    let list_text = list_text.strip_prefix('\u{feff}').unwrap_or(list_text);
    let kv = kv_parse(list_text)?;

    let list_ver = kv_get(&kv, "清单版本")?;
    if list_ver != "1" {
        return Err(e137(format!("清单版本 只认 1，实测 {}", list_ver)));
    }
    let name = kv_get(&kv, "包")?;
    if name != m.name {
        return Err(e137(format!(
            "清单写的包名是「{}」，砾包.toml 是「{}」",
            name, m.name
        )));
    }
    let version = kv_get(&kv, "版本")?;
    let mver = ver_text(&m.version);
    if version != mver {
        return Err(e137(format!(
            "清单写的版本是「{}」，砾包.toml 是「{}」",
            version, mver
        )));
    }

    let product = kv_get(&kv, "产物")?;
    let expect_product = m
        .product
        .clone()
        .unwrap_or_else(|| format!("{}.jmb", m.name));
    if product != expect_product {
        return Err(e137(format!(
            "清单写的产物是「{}」，按包规范应为「{}」",
            product, expect_product
        )));
    }
    let data = find(&product)
        .ok_or_else(|| e137(format!("归档里没有产物「{}」", product)))?;

    let size = kv_get(&kv, "产物大小")?;
    let size: usize = size
        .parse()
        .map_err(|_| e137(format!("产物大小 不是整数：{}", size)))?;
    if data.len() != size {
        return Err(e137(format!(
            "产物大小不符：清单 {} B，实际 {} B",
            size,
            data.len()
        )));
    }

    let sum = kv_get(&kv, "产物校验")?;
    let actual = fnv1a64(data);
    if sum != actual {
        return Err(e137(format!(
            "产物校验不符：清单 {}，实际 {}（文件被改过或归档损坏）",
            sum, actual
        )));
    }

    let ver_text = kv_get(&kv, "字节码版本")?;
    let ver: u8 = ver_text
        .parse()
        .map_err(|_| e137(format!("字节码版本 不是整数：{}", ver_text)))?;
    let Some(real) = jmb_version(data) else {
        return Err(e137(format!("产物「{}」不是 .jmb（魔数不符）", product)));
    };
    if real != ver || real != writer::VERSION {
        return Err(e137(format!(
            "字节码版本不符：清单 {}，产物 {}，当前编译器 {}",
            ver,
            real,
            writer::VERSION
        )));
    }

    let abi_text = kv_get(&kv, "原生ABI")?;
    let abi: u8 = abi_text
        .parse()
        .map_err(|_| e137(format!("原生ABI 不是整数：{}", abi_text)))?;
    if abi != m.native_abi || abi != native_spec::NATIVE_ABI_VERSION {
        return Err(CompileError::new(
            "E129",
            1,
            1,
            format!(
                "原生ABI 不一致：清单 v{}，砾包 v{}，编译器 v{}",
                abi,
                m.native_abi,
                native_spec::NATIVE_ABI_VERSION
            ),
        ));
    }

    let dbg_text = kv_get(&kv, "调试段")?;
    let expect_dbg = match dbg_text.as_str() {
        "true" | "真" => true,
        "false" | "假" => false,
        other => return Err(e137(format!("调试段 只认 true/false，实测 {}", other))),
    };
    if has_debug(data) != expect_dbg {
        return Err(e137(format!(
            "调试段标记不符：清单 {}，产物 {}",
            expect_dbg,
            has_debug(data)
        )));
    }

    // 接口清单（可选）：导出函数的参数个数必须与产物代码表一致。
    if let Some(iface) = find(IFACE_NAME) {
        let text = std::str::from_utf8(iface).map_err(|_| e137("接口.toml 不是 UTF-8 文本"))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let (ikv, funcs) = parse_iface(text)?;
        let iver = kv_get(&ikv, "接口版本")?;
        if iver != "1" {
            return Err(e137(format!("接口版本 只认 1，实测 {}", iver)));
        }
        let iname = kv_get(&ikv, "包")?;
        if iname != m.name {
            return Err(e137(format!(
                "接口写的包名是「{}」，砾包.toml 是「{}」",
                iname, m.name
            )));
        }
        let iver_text = kv_get(&ikv, "版本")?;
        if iver_text != mver {
            return Err(e137(format!(
                "接口写的版本是「{}」，砾包.toml 是「{}」",
                iver_text, mver
            )));
        }
        let iabi: u8 = kv_get(&ikv, "原生ABI")?
            .parse()
            .map_err(|_| e137("接口的 原生ABI 不是整数"))?;
        if iabi != abi {
            return Err(e137(format!(
                "接口的原生ABI v{} 与清单 v{} 不一致",
                iabi, abi
            )));
        }
        let img = crate::debug::parse(data)
            .map_err(|msg| e137(format!("产物无法解析为 JMB：{}", msg)))?;
        for (name, arity) in &funcs {
            match img.codes.iter().find(|c| c.name == *name) {
                None => {
                    return Err(e137(format!(
                        "接口列了函数「{}」，但产物里没有这个代码对象",
                        name
                    )))
                }
                Some(c) if c.nparams != *arity => {
                    return Err(e137(format!(
                        "函数「{}」参数个数不符：接口 {}，产物 {}",
                        name, arity, c.nparams
                    )))
                }
                _ => {}
            }
        }
    }

    let kind = if m.product.is_some() { "二进制包" } else { "源码包" };
    Ok(format!(
        "校验通过：{} {}（{}，产物 {} {} B，{}，调试段：{}）",
        m.name,
        mver,
        kind,
        product,
        data.len(),
        actual,
        if expect_dbg { "有" } else { "无" }
    ))
}

fn e137(msg: impl Into<String>) -> CompileError {
    CompileError::new("E137", 1, 1, msg)
}

fn kv_get(kv: &[(String, String)], key: &str) -> Result<String> {
    kv.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .ok_or_else(|| e137(format!("清单缺字段「{}」", key)))
}

/// 清单的极简解析：只认 `键 = 值`，忽略注释与空行，不允许表头与重复键。
fn kv_parse(text: &str) -> Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = pack_manifest::strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            return Err(e137(format!("清单第 {} 行出现了表头，清单只用 键 = 值", idx + 1)));
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(e137(format!("清单第 {} 行不是 键 = 值", idx + 1)));
        };
        let key = k.trim().to_string();
        let raw_v = v.trim();
        let val = if raw_v.len() >= 2 && raw_v.starts_with('"') && raw_v.ends_with('"') {
            raw_v[1..raw_v.len() - 1].to_string()
        } else {
            raw_v.to_string()
        };
        if key.is_empty() || val.is_empty() {
            return Err(e137(format!("清单第 {} 行的键或值为空", idx + 1)));
        }
        if out.iter().any(|(a, _)| *a == key) {
            return Err(e137(format!("清单字段「{}」重复", key)));
        }
        out.push((key, val));
    }
    Ok(out)
}

/// JMB 头：魔数 + 格式版本。不是 `.jmb` 时返回 `None`。
fn jmb_version(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 6 || &bytes[0..4] != &writer::MAGIC {
        return None;
    }
    Some(bytes[4])
}

/// `p5.bit0`：带尾部调试段。
fn has_debug(bytes: &[u8]) -> bool {
    bytes.len() > 5 && bytes[5] & 1 != 0
}

/// FNV-1a 64，与锁文件/索引里的 `校验` 同一种格式（非加密，只用来发现变化）。
fn fnv1a64(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{:016x}", h)
}

fn list_text(m: &pack_manifest::Manifest, product: &str, bytes: &[u8]) -> String {
    format!(
        "# 本文件由 micc pack --归档 生成，不要手改\n\
         清单版本 = 1\n\
         包 = \"{}\"\n\
         版本 = \"{}\"\n\
         字节码版本 = {}\n\
         原生ABI = {}\n\
         产物 = \"{}\"\n\
         产物大小 = {}\n\
         产物校验 = \"{}\"\n\
         调试段 = {}\n",
        m.name,
        ver_text(&m.version),
        jmb_version(bytes).unwrap_or(0),
        m.native_abi,
        product,
        bytes.len(),
        fnv1a64(bytes),
        has_debug(bytes)
    )
}

fn archive_bytes(
    m: &pack_manifest::Manifest,
    product: &str,
    bytes: &[u8],
    pkg_toml: &[u8],
    iface: Option<&[u8]>,
) -> Vec<u8> {
    let mut entries = vec![
        zip::Entry {
            name: MANIFEST_NAME_LOCAL.to_string(),
            data: pkg_toml.to_vec(),
        },
        zip::Entry {
            name: product.to_string(),
            data: bytes.to_vec(),
        },
        zip::Entry {
            name: LIST_NAME.to_string(),
            data: list_text(m, product, bytes).into_bytes(),
        },
    ];
    if let Some(data) = iface {
        entries.push(zip::Entry {
            name: IFACE_NAME.to_string(),
            data: data.to_vec(),
        });
    }
    zip::write(&entries)
}

/// 目录里的可选文件：不存在返回 `None`。
fn read_optional(dir: &Path, name: &str) -> Option<Vec<u8>> {
    let p = dir.join(name);
    if p.is_file() {
        std::fs::read(p).ok()
    } else {
        None
    }
}

/// 源码归档（`pack --源码`）：只收 `砾包.toml` 与本包的 `.jsh`，供 `micc install` 解包后按目录哈希校验。
/// 不含 `.jmb`、`清单.toml`、`.zip` 等生成物，保证归档内容与 `micc hash --目录` 的口径一致。
///
/// 这是归档格式的唯一实现，`pack --源码` 与 `micc publish` 都调它。返回归档里的 `.jsh` 名（已排序）。
pub fn build_source_archive(
    dir: &Path,
    m: &pack_manifest::Manifest,
    out_path: &Path,
) -> Result<Vec<String>> {
    if m.product.is_some() {
        return Err(usage_err(format!(
            "「{}」是二进制包，没有源码可归档；用 pack --归档 分发产物",
            m.name
        )));
    }
    let mut names: Vec<String> = Vec::new();
    let rd = std::fs::read_dir(dir)
        .map_err(|e| usage_err(format!("读不了目录 {}：{}", dir.display(), e)))?;
    for ent in rd.flatten() {
        let p = ent.path();
        if p.is_file() && p.extension().map(|x| x == "jsh").unwrap_or(false) {
            if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                names.push(n.to_string());
            }
        }
    }
    names.sort();
    if !names.iter().any(|n| n == &m.entry) {
        return Err(usage_err(format!(
            "入口 {} 不在包目录里；源码归档必须先有入口源码",
            m.entry
        )));
    }

    let pkg_toml = std::fs::read(dir.join(MANIFEST_NAME_LOCAL))
        .map_err(|e| usage_err(format!("读不了 {}：{}", MANIFEST_NAME_LOCAL, e)))?;
    let mut entries = vec![zip::Entry {
        name: MANIFEST_NAME_LOCAL.to_string(),
        data: pkg_toml,
    }];
    for n in &names {
        let data = std::fs::read(dir.join(n))
            .map_err(|e| usage_err(format!("读不了 {}：{}", n, e)))?;
        entries.push(zip::Entry {
            name: n.clone(),
            data,
        });
    }

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| usage_err(format!("建不了目录 {}：{}", parent.display(), e)))?;
        }
    }
    std::fs::write(out_path, zip::write(&entries))
        .map_err(|e| usage_err(format!("写不了 {}：{}", out_path.display(), e)))?;
    Ok(names)
}

fn pack_source_archive(opts: &Opts, tree: &Tree) -> Result<()> {
    let m = &tree.root.manifest;
    let out_path = opts.out.clone().unwrap_or_else(|| {
        opts.dir.join(format!(
            "{}-{}-源码.zip",
            m.name,
            ver_text(&m.version)
        ))
    });
    let names = build_source_archive(&opts.dir, m, &out_path)?;
    println!(
        "源码归档完成：{} {} → {}（{} + {} 个 .jsh）",
        m.name,
        ver_text(&m.version),
        out_path.display(),
        MANIFEST_NAME_LOCAL,
        names.len()
    );
    println!("  提示：索引里的 校验 用 micc hash --目录 对本包目录取，解包后须一致");
    Ok(())
}

/// 二进制包（`[包] 产物 = "x.jmb"`）：没有源码，只校验产物并可选归档。
fn pack_binary(opts: &Opts, tree: &Tree, product: &str) -> Result<()> {
    let m = &tree.root.manifest;
    if !m.deps.is_empty() {
        return Err(usage_err(format!(
            "二进制包「{}」不能再声明依赖；跨包组合请用源码包",
            m.name
        )));
    }
    if m.native_abi != native_spec::NATIVE_ABI_VERSION {
        return Err(CompileError::new(
            "E129",
            1,
            1,
            format!(
                "包「{}」要求 Native ABI v{}，当前编译器是 v{}",
                m.name,
                m.native_abi,
                native_spec::NATIVE_ABI_VERSION
            ),
        ));
    }
    let path = opts.dir.join(product);
    let bytes = std::fs::read(&path)
        .map_err(|e| usage_err(format!("读不了产物 {}：{}", path.display(), e)))?;
    let Some(real) = jmb_version(&bytes) else {
        return Err(e137(format!("产物 {} 不是 .jmb（魔数不符）", path.display())));
    };
    if real != writer::VERSION {
        return Err(e137(format!(
            "产物 {} 的字节码版本是 v{}，当前编译器只认 v{}",
            path.display(),
            real,
            writer::VERSION
        )));
    }
    if !opts.archive {
        return Err(usage_err(format!(
            "「{}」是二进制包（[包] 产物 = {}），没有源码可编译；加 --归档 产出分发件，或用 micc verify 校验",
            m.name, product
        )));
    }
    let pkg_toml = std::fs::read(opts.dir.join(MANIFEST_NAME_LOCAL))
        .map_err(|e| usage_err(format!("读不了 {}：{}", MANIFEST_NAME_LOCAL, e)))?;
    let out_path = opts.out.clone().unwrap_or_else(|| {
        opts.dir
            .join(format!("{}-{}.zip", m.name, ver_text(&m.version)))
    });
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| usage_err(format!("建不了目录 {}：{}", parent.display(), e)))?;
        }
    }
    let iface = read_optional(&opts.dir, IFACE_NAME);
    let data = archive_bytes(m, product, &bytes, &pkg_toml, iface.as_deref());
    std::fs::write(&out_path, data)
        .map_err(|e| usage_err(format!("写不了 {}：{}", out_path.display(), e)))?;
    write_list_beside(&out_path, &list_text(m, product, &bytes))?;
    println!("归档完成：{} {}（二进制包）", m.name, ver_text(&m.version));
    println!("  产物：{} {} B", product, bytes.len());
    println!("  归档：{}", out_path.display());
    Ok(())
}

/// 清单同时写一份到归档所在目录，供 `micc verify <包目录>` 使用。
fn write_list_beside(archive: &Path, text: &str) -> Result<()> {
    let dir = archive.parent().unwrap_or_else(|| Path::new("."));
    let p = dir.join(LIST_NAME);
    std::fs::write(&p, text)
        .map_err(|e| usage_err(format!("写不了 {}：{}", p.display(), e)))
}

/// 接口清单的文件名（与 `清单.toml` 并列，可随包分发）。
const IFACE_NAME: &str = "接口.toml";

/// `micc interface`：把源码包的导出函数写成接口清单（名字 = 参数个数）。
/// 只读入口源码的 AST，不编译、不解析依赖；产物侧的真实参数个数由 `micc verify` 对照 JMB 代码表。
pub fn cmd_interface(args: &[String]) -> i32 {
    match do_interface(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

fn do_interface(args: &[String]) -> Result<()> {
    let mut dir = PathBuf::from(".");
    let mut out: Option<PathBuf> = None;
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
            "-o" | "--输出" | "--out" => {
                i += 1;
                if i >= args.len() {
                    return Err(usage_err("-o 后面要跟输出文件"));
                }
                out = Some(PathBuf::from(&args[i]));
            }
            other => return Err(usage_err(format!("不认识的参数：{}", other))),
        }
        i += 1;
    }
    if !dir.is_dir() {
        return Err(usage_err(format!("包目录不存在：{}", dir.display())));
    }
    let m = pack_manifest::Manifest::load(&dir)?;
    if m.product.is_some() {
        return Err(usage_err(format!(
            "「{}」是二进制包（没有源码），接口清单应从源码包生成",
            m.name
        )));
    }
    let entry = dir.join(&m.entry);
    let src = std::fs::read_to_string(&entry)
        .map_err(|e| usage_err(format!("读不了入口 {}：{}", entry.display(), e)))?;
    let src = src.strip_prefix('\u{feff}').unwrap_or(&src);
    let tokens = lexer::tokenize(src).map_err(|e| in_pkg(e, &m.name))?;
    let stmts = Parser::new(tokens)
        .parse_program()
        .map_err(|e| in_pkg(e, &m.name))?;

    let mut funcs: Vec<(String, usize)> = Vec::new();
    for s in &stmts {
        if let crate::ast::StmtKind::FuncDef { name, params, .. } = &s.kind {
            funcs.push((name.clone(), params.len()));
        }
    }
    // 有 [导出] 就只列导出的那些；导出里的非函数名字（例如 `令 基 = 1`）不进接口。
    if let Some(exports) = &m.exports {
        funcs.retain(|(n, _)| exports.contains(n));
    }
    funcs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut text = String::new();
    text.push_str("# 本文件由 micc interface 生成，不要手改\n");
    text.push_str("接口版本 = 1\n");
    text.push_str(&format!("包 = \"{}\"\n", m.name));
    text.push_str(&format!("版本 = \"{}\"\n", ver_text(&m.version)));
    text.push_str(&format!("原生ABI = {}\n", m.native_abi));
    text.push_str(&format!("字节码版本 = {}\n", writer::VERSION));
    text.push_str("\n[函数]\n");
    for (n, k) in &funcs {
        text.push_str(&format!("{} = {}\n", n, k));
    }

    let path = out.unwrap_or_else(|| dir.join(IFACE_NAME));
    std::fs::write(&path, text)
        .map_err(|e| usage_err(format!("写不了 {}：{}", path.display(), e)))?;
    println!(
        "接口清单：{} {} → {}（函数 {} 个）",
        m.name,
        ver_text(&m.version),
        path.display(),
        funcs.len()
    );
    Ok(())
}

/// 解析接口清单：头部键值 + `[函数]` 段（名字 = 参数个数）。
fn parse_iface(text: &str) -> Result<(Vec<(String, String)>, Vec<(String, u8)>)> {
    let mut kv: Vec<(String, String)> = Vec::new();
    let mut funcs: Vec<(String, u8)> = Vec::new();
    let mut section = "";
    for (idx, raw) in text.lines().enumerate() {
        let line = pack_manifest::strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
                return Err(e137(format!("接口第 {} 行的表头缺少右方括号", idx + 1)));
            };
            match inner.trim() {
                "函数" => section = "函数",
                other => {
                    return Err(e137(format!(
                        "接口里不认识的表「{}」；只支持 [函数]",
                        other
                    )))
                }
            }
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(e137(format!("接口第 {} 行不是 键 = 值", idx + 1)));
        };
        let key = k.trim().to_string();
        let raw_v = v.trim();
        let val = if raw_v.len() >= 2 && raw_v.starts_with('"') && raw_v.ends_with('"') {
            raw_v[1..raw_v.len() - 1].to_string()
        } else {
            raw_v.to_string()
        };
        if key.is_empty() || val.is_empty() {
            return Err(e137(format!("接口第 {} 行的键或值为空", idx + 1)));
        }
        if section == "函数" {
            let arity: u8 = val
                .parse()
                .map_err(|_| e137(format!("函数「{}」的参数个数不是 0~255：{}", key, val)))?;
            if funcs.iter().any(|(n, _)| *n == key) {
                return Err(e137(format!("接口里函数「{}」重复", key)));
            }
            funcs.push((key, arity));
        } else {
            if kv.iter().any(|(a, _)| *a == key) {
                return Err(e137(format!("接口字段「{}」重复", key)));
            }
            kv.push((key, val));
        }
    }
    Ok((kv, funcs))
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
pub(crate) fn display_path(path: &std::path::Path) -> String {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    let rel = cwd
        .as_ref()
        .and_then(|c| abs.strip_prefix(c).ok())
        .unwrap_or(&abs);
    let text = rel.to_string_lossy().replace('\\', "/");
    text.strip_prefix("//?/").unwrap_or(&text).to_string()
}

fn entry_display(pkg: &Resolved) -> String {
    display_path(&pkg.dir.join(&pkg.manifest.entry))
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
    let opts = parse_opts(args, false, true)?;
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

    let entries = pack_resolve::install(&opts.dir, opts.index.as_deref())?;
    println!(
        "已安装 {} 个包到 {}",
        entries.len(),
        opts.dir.join(".砾").join("包").display()
    );
    for e in &entries {
        println!("  {} {}　{}　{}", e.name, e.version, e.source, e.hash);
    }
    println!("锁文件：{}", crate::pack_manifest::lock_path(&opts.dir).display());
    Ok(())
}

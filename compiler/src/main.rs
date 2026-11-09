mod ast;
mod codegen;
mod debug;
mod error;
mod langcard;
mod lexer;
mod native_spec;
mod opcodes;
mod pack;
mod pack_index;
mod pack_manifest;
mod pack_merge;
mod pack_registry;
mod pack_resolve;
mod parser;
mod symbols;
mod vm_spec;
mod writer;
mod zip;

use std::path::PathBuf;
use std::process::exit;

fn usage() {
    eprintln!("用法：micc <输入.jsh> [-o 输出.jmb] [--dump] [--strip-debug]");
    eprintln!("      micc pack [--目录 <包目录>] [-o 输出] [--离线] [--strip-debug] [--归档]");
    eprintln!("      micc install [--目录 <包目录>] [--索引 <索引文件|URL>] [--离线]");
    eprintln!("      micc hash [--目录 <包目录>]                # 算目录内容哈希（填索引的 校验）");
    eprintln!("      micc verify <包目录|归档.zip>              # 校验归档/二进制包（清单 + 产物 + 哈希）");
    eprintln!("      micc interface --目录 <包目录> [-o 接口.toml]  # 生成导出函数接口清单");
    eprintln!("      micc search [关键词...] [--索引 <索引文件|URL>]  # 检索静态索引");
    eprintln!("      micc publish [--目录 <包目录>] [--索引 <索引文件>] [--仓库 <目录>]  # 发布源码包并写索引");
    eprintln!("      micc --line-of <x.jmb> <代码下标> <偏移>   # 指令位置 → 源文件:行");
    eprintln!("      micc --lines-of <x.jmb> <文件|*> <行号>    # 源码行 → 指令范围");
    eprintln!("      micc --locals-of <x.jmb> <代码下标>        # 代码对象 → 局部槽符号");
    eprintln!("      micc --line-table <x.jmb>                 # 全量行表：代码\t偏移\t文件\t行");
    eprintln!("      micc --debug-dump <x.jmb>                  # 打印调试表");
    eprintln!("      micc --symbols <x.jsh>                     # 列出定义：种类 名字 行 列");
    eprintln!("      micc --pack-files <包目录>                 # 包与依赖的入口文件（给编辑器跳转）");
    eprintln!("      micc --emit-blob <x.jmb> [-o jm_blob.c]     # 生成内嵌字节数组（固件用）");
    eprintln!("      micc --emit-header        # 导出 jm_opcodes.h");
    eprintln!("      micc --emit-native-header # 导出运行时原生表头 runtime/include/jm_native_table.h");
    eprintln!("      micc --check-native       # CI 校验原生表头没漂移（不一致则退出码 1）");
    eprintln!("      micc --langcard           # 生成 docs/语言卡.md 与 spec/langcard.json");
    eprintln!("      micc --check-langcard     # CI 校验语言卡没漂移（不一致则退出码 1）");
}

fn read_jmb(path: &str) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("读不了文件 {}：{}", path, e);
            None
        }
    }
}

/// `--line-of`：正向查询，指令位置 → 源文件与行号。
fn cmd_line_of(args: &[String]) -> i32 {
    if args.len() != 3 {
        usage();
        return 2;
    }
    let Some(bytes) = read_jmb(&args[0]) else {
        return 2;
    };
    let image = match debug::parse(&bytes) {
        Ok(im) => im,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    let (Ok(code), Ok(off)) = (args[1].parse::<usize>(), args[2].parse::<u16>()) else {
        eprintln!("代码下标与偏移必须是整数");
        return 2;
    };
    let Some(ce) = image.codes.get(code) else {
        eprintln!("代码下标 {} 超出范围（共 {} 个）", code, image.codes.len());
        return 1;
    };
    if off >= ce.len {
        eprintln!(
            "偏移 {} 超出代码 {} 的长度 {}",
            off, code, ce.len
        );
        return 1;
    }
    match image.line_at(code, off) {
        Some((file, line)) => {
            println!("{}:{}", file, line);
            0
        }
        None => {
            eprintln!("没有可用调试信息（旧文件、已剥离或调试段损坏）");
            1
        }
    }
}

/// `--lines-of`：反查，源码行 → 各代码对象里的指令范围。
fn cmd_lines_of(args: &[String]) -> i32 {
    if args.len() != 3 {
        usage();
        return 2;
    }
    let Some(bytes) = read_jmb(&args[0]) else {
        return 2;
    };
    let image = match debug::parse(&bytes) {
        Ok(im) => im,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    if image.debug.is_none() {
        eprintln!("没有可用调试信息（旧文件、已剥离或调试段损坏）");
        return 1;
    }
    let file_id = if args[1] == "*" {
        None
    } else {
        match image.file_id(&args[1]) {
            Some(f) => Some(f),
            None => {
                eprintln!("调试表里没有文件「{}」", args[1]);
                return 1;
            }
        }
    };
    let Ok(line) = args[2].parse::<u16>() else {
        eprintln!("行号必须是整数");
        return 2;
    };
    let spans = debug::spans_of_line(&image, file_id, line);
    if spans.is_empty() {
        println!("（没有该行的指令）");
        return 0;
    }
    for s in spans {
        let name = image
            .codes
            .get(s.code)
            .map(|c| c.name.as_str())
            .unwrap_or("?");
        println!(
            "代码 {} 「{}」 偏移 {}..{}",
            s.code, name, s.start, s.end
        );
    }
    0
}

/// `--locals-of`：代码对象 → 局部槽符号（`槽号<TAB>名字`）。
fn cmd_locals_of(args: &[String]) -> i32 {
    if args.len() != 2 {
        usage();
        return 2;
    }
    let Some(bytes) = read_jmb(&args[0]) else {
        return 2;
    };
    let image = match debug::parse(&bytes) {
        Ok(im) => im,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    let Ok(code) = args[1].parse::<usize>() else {
        eprintln!("代码下标必须是整数");
        return 2;
    };
    if image.codes.get(code).is_none() {
        eprintln!("代码下标 {} 超出范围（共 {} 个）", code, image.codes.len());
        return 1;
    }
    match image.locals_at(code) {
        Some(list) => {
            for (slot, name) in list {
                println!("{}\t{}", slot, name);
            }
            0
        }
        None => {
            eprintln!("该代码对象没有局部符号（旧文件、已剥离或未带符号区）");
            1
        }
    }
}

/// `--line-table`：整份行表，供宿主一次预取（避免每指令起进程）。
fn cmd_line_table(args: &[String]) -> i32 {
    if args.len() != 1 {
        usage();
        return 2;
    }
    let Some(bytes) = read_jmb(&args[0]) else {
        return 2;
    };
    let image = match debug::parse(&bytes) {
        Ok(im) => im,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    let Some(d) = image.debug.as_ref() else {
        eprintln!("没有可用调试信息（旧文件、已剥离或调试段损坏）");
        return 1;
    };
    for (ci, c) in d.codes.iter().enumerate() {
        for e in &c.entries {
            println!("{}\t{}\t{}\t{}", ci, e.off, e.file_id, e.line);
        }
    }
    0
}

fn cmd_debug_dump(args: &[String]) -> i32 {
    if args.len() != 1 {
        usage();
        return 2;
    }
    let Some(bytes) = read_jmb(&args[0]) else {
        return 2;
    };
    match debug::parse(&bytes) {
        Ok(im) => {
            print!("{}", debug::dump(&im));
            0
        }
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

/// `--emit-blob`：把 `.jmb` 生成 C 数组源文件（固件内嵌用，build-firmware.ps1 调用）。
fn cmd_emit_blob(args: &[String]) -> i32 {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("-o 后面要跟输出文件");
                    return 2;
                }
                output = Some(PathBuf::from(&args[i]));
            }
            other => {
                if input.is_none() {
                    input = Some(PathBuf::from(other));
                } else {
                    eprintln!("多余的参数：{}", other);
                    return 2;
                }
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("用法：micc --emit-blob <文件.jmb> [-o jm_blob.c]");
        return 2;
    };
    let bytes = match std::fs::read(&input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("读不了文件 {}：{}", input.display(), e);
            return 2;
        }
    };
    let out = output.unwrap_or_else(|| input.with_file_name("jm_blob.c"));
    if let Err(e) = std::fs::write(&out, writer::emit_blob(&bytes)) {
        eprintln!("写不了文件 {}：{}", out.display(), e);
        return 2;
    }
    println!("已生成 {}（内嵌 {} 字节）", out.display(), bytes.len());
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        exit(2);
    }
    if args[0] == "--emit-header" {
        print!("{}", opcodes::emit_c_header());
        return;
    }
    if args[0] == "--emit-native-header" {
        match native_spec::render_c_header() {
            Ok(s) => print!("{}", s),
            Err(e) => {
                eprintln!("{}", e);
                exit(1);
            }
        }
        return;
    }
    if args[0] == "--check-native" {
        if let Err(e) = native_spec::check() {
            eprintln!("{}", e);
            exit(1);
        }
        return;
    }
    if args[0] == "--langcard" {
        if let Err(e) = langcard::write_files() {
            eprintln!("{}", e);
            exit(1);
        }
        return;
    }
    if args[0] == "--check-langcard" {
        if let Err(e) = langcard::check() {
            eprintln!("{}", e);
            exit(1);
        }
        return;
    }
    if args[0] == "--line-of" {
        exit(cmd_line_of(&args[1..]));
    }
    if args[0] == "--lines-of" {
        exit(cmd_lines_of(&args[1..]));
    }
    if args[0] == "--locals-of" {
        exit(cmd_locals_of(&args[1..]));
    }
    if args[0] == "--line-table" {
        exit(cmd_line_table(&args[1..]));
    }
    if args[0] == "--debug-dump" {
        exit(cmd_debug_dump(&args[1..]));
    }
    if args[0] == "--symbols" {
        if args.len() != 2 {
            usage();
            exit(2);
        }
        match symbols::file_symbols(std::path::Path::new(&args[1])) {
            Ok(list) => {
                for s in list {
                    println!("{}\t{}\t{}\t{}", s.kind, s.name, s.line, s.col);
                }
            }
            Err(e) => {
                eprintln!("{}", e);
                exit(1);
            }
        }
        return;
    }
    if args[0] == "--pack-files" {
        if args.len() != 2 {
            usage();
            exit(2);
        }
        match symbols::pack_files(std::path::Path::new(&args[1])) {
            Ok(list) => {
                for (name, version, entry) in list {
                    println!("{}\t{}\t{}", name, version, entry);
                }
            }
            Err(e) => {
                eprintln!("{}", e);
                exit(1);
            }
        }
        return;
    }
    if args[0] == "pack" {
        exit(pack::cmd_pack(&args[1..]));
    }
    if args[0] == "install" {
        exit(pack::cmd_install(&args[1..]));
    }
    if args[0] == "hash" {
        exit(pack::cmd_hash(&args[1..]));
    }
    if args[0] == "verify" {
        exit(pack::cmd_verify(&args[1..]));
    }
    if args[0] == "interface" {
        exit(pack::cmd_interface(&args[1..]));
    }
    if args[0] == "search" {
        exit(pack_registry::cmd_search(&args[1..]));
    }
    if args[0] == "publish" {
        exit(pack_registry::cmd_publish(&args[1..]));
    }
    if args[0] == "--emit-blob" {
        exit(cmd_emit_blob(&args[1..]));
    }

    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut dump = false;
    let mut strip_debug = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                i += 1;
                if i >= args.len() {
                    usage();
                    exit(2);
                }
                output = Some(PathBuf::from(&args[i]));
            }
            "--dump" => dump = true,
            "--strip-debug" => strip_debug = true,
            "-h" | "--help" => {
                usage();
                return;
            }
            other => {
                if input.is_none() {
                    input = Some(PathBuf::from(other));
                } else {
                    eprintln!("多余的参数：{}", other);
                    exit(2);
                }
            }
        }
        i += 1;
    }

    let Some(input) = input else {
        usage();
        exit(2);
    };

    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("读不了文件 {}：{}", input.display(), e);
            exit(2);
        }
    };

    let file_name = pack::display_path(&input);
    match compile_source_opts(&src, &[file_name], !strip_debug) {
        Ok((enc, dump_text)) => {
            if dump {
                print!("{}", dump_text);
            }
            let out_path = output.unwrap_or_else(|| input.with_extension("jmb"));
            if let Err(e) = std::fs::write(&out_path, &enc.bytes) {
                eprintln!("写不了文件 {}：{}", out_path.display(), e);
                exit(2);
            }
            if !dump {
                if enc.debug_len > 0 {
                    println!(
                        "已生成 {}（{} 字节；剥离调试段后 {} 字节，调试段 {} 字节）",
                        out_path.display(),
                        enc.bytes.len(),
                        enc.base_len,
                        enc.debug_len
                    );
                } else {
                    println!("已生成 {}（{} 字节，无调试段）", out_path.display(), enc.bytes.len());
                }
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    }
}

/// 单文件编译：`files` 是调试表的源文件表（下标即 `file_id`）。
pub(crate) fn compile_source_opts(
    src: &str,
    files: &[String],
    with_debug: bool,
) -> Result<(writer::Encoded, String), error::CompileError> {
    let tokens = lexer::tokenize(src)?;
    let mut p = parser::Parser::new(tokens);
    let stmts = p.parse_program()?;
    let spans = if stmts.is_empty() {
        Vec::new()
    } else {
        vec![(0usize, stmts.len(), 0u8)]
    };
    let prog = codegen::compile(&stmts, files.to_vec(), &spans)?;
    let text = writer::disassemble(&prog);
    let enc = writer::write(&prog, with_debug)?;
    Ok((enc, text))
}

pub(crate) fn compile_source(src: &str) -> Result<(Vec<u8>, String), error::CompileError> {
    let files = vec!["<内置示例>".to_string()];
    let (enc, text) = compile_source_opts(src, &files, true)?;
    Ok((enc.bytes, text))
}

mod ast;
mod codegen;
mod error;
mod langcard;
mod lexer;
mod opcodes;
mod parser;
mod writer;

use std::path::PathBuf;
use std::process::exit;

fn usage() {
    eprintln!("用法：micc <输入.jsh> [-o 输出.jmb] [--dump]");
    eprintln!("      micc --emit-header        # 导出 jm_opcodes.h");
    eprintln!("      micc --langcard           # 生成 docs/语言卡.md 与 spec/langcard.json");
    eprintln!("      micc --check-langcard     # CI 校验语言卡没漂移（不一致则退出码 1）");
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

    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut dump = false;
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

    match compile_source(&src) {
        Ok((bytes, dump_text)) => {
            if dump {
                print!("{}", dump_text);
            }
            let out_path = output.unwrap_or_else(|| input.with_extension("jmb"));
            if let Err(e) = std::fs::write(&out_path, &bytes) {
                eprintln!("写不了文件 {}：{}", out_path.display(), e);
                exit(2);
            }
            if !dump {
                println!(
                    "已生成 {}（{} 字节）",
                    out_path.display(),
                    bytes.len()
                );
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    }
}

pub(crate) fn compile_source(src: &str) -> Result<(Vec<u8>, String), error::CompileError> {
    let tokens = lexer::tokenize(src)?;
    let mut p = parser::Parser::new(tokens);
    let stmts = p.parse_program()?;
    let prog = codegen::compile(&stmts)?;
    let text = writer::disassemble(&prog);
    let bytes = writer::write(&prog)?;
    Ok((bytes, text))
}

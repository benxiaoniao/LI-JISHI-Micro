use crate::codegen::{ConstVal, Program};
use crate::error::{CompileError, Result};
use crate::opcodes::*;

const HEADER_LEN: usize = 12;
const CONST_ENTRY: usize = 8;
const NAME_ENTRY: usize = 2;
/// JMB v2：代码表每项 10 字节（新增 `ncap`）。（版本 2 起不再兼容 v1。）
const CODE_ENTRY: usize = 10;
/// 捕获表每项 2 字节：`(父帧槽, 子帧槽)`。
const CAP_ENTRY: usize = 2;

pub fn write(prog: &Program) -> Result<Vec<u8>> {
    let nconsts = prog.consts.len();
    let nnames = prog.names.len();
    let ncodes = prog.codes.len();
    if nconsts > 255 || nnames > 255 || ncodes > 255 {
        return Err(CompileError::new(
            "E105",
            1,
            1,
            "常量 / 名字 / 函数数量超过 255",
        ));
    }

    let const_start = HEADER_LEN;
    let name_start = const_start + CONST_ENTRY * nconsts;
    let code_start = name_start + NAME_ENTRY * nnames;
    let ncap_total: usize = prog.codes.iter().map(|c| c.caps.len()).sum();
    let cap_start = code_start + CODE_ENTRY * ncodes;
    let pool_start = cap_start + CAP_ENTRY * ncap_total;

    // 字符串池：名字在前，字符串常量在后
    let mut pool: Vec<u8> = Vec::new();
    let mut name_offsets: Vec<u16> = Vec::new();
    for name in &prog.names {
        name_offsets.push((pool_start + pool.len()) as u16);
        pool.extend_from_slice(name.as_bytes());
        pool.push(0);
    }
    let mut str_offsets: Vec<Option<u32>> = Vec::new();
    for c in &prog.consts {
        if let ConstVal::Str(s) = c {
            str_offsets.push(Some((pool_start + pool.len()) as u32));
            pool.extend_from_slice(s.as_bytes());
        } else {
            str_offsets.push(None);
        }
    }

    let code_bytes_start = pool_start + pool.len();
    let mut code_offsets: Vec<(u16, u16)> = Vec::new();
    let mut cursor = code_bytes_start;
    for c in &prog.codes {
        if c.code.len() > 65535 {
            return Err(CompileError::new(
                "E111",
                1,
                1,
                "单个函数的指令流超过 64KB",
            ));
        }
        code_offsets.push((cursor as u16, c.code.len() as u16));
        cursor += c.code.len();
    }
    let total = cursor;
    if total > 65535 {
        return Err(CompileError::new(
            "E111",
            1,
            1,
            ".jmb 超过 64KB（u16 偏移的上限）",
        ));
    }

    let mut out = Vec::with_capacity(total);
    // 头
    out.extend_from_slice(b"JMB1");
    out.push(2); // version：JMB v2，代码表带捕获表
    out.push(0); // flags
    out.push(nconsts as u8);
    out.push(nnames as u8);
    out.push(ncodes as u8);
    out.push(prog.entry);
    out.extend_from_slice(&[0, 0]);

    // 常量表
    for (i, c) in prog.consts.iter().enumerate() {
        match c {
            ConstVal::Int(v) => {
                out.push(K_INT);
                out.push(0);
                out.extend_from_slice(&[0, 0]);
                out.extend_from_slice(&(*v as u32).to_le_bytes());
            }
            ConstVal::Dec(v) => {
                out.push(K_DEC);
                out.push(0);
                out.extend_from_slice(&[0, 0]);
                out.extend_from_slice(&(*v as u32).to_le_bytes());
            }
            ConstVal::Str(s) => {
                out.push(K_STR);
                out.push(0);
                out.extend_from_slice(&(s.len() as u16).to_le_bytes());
                out.extend_from_slice(&(str_offsets[i].unwrap()).to_le_bytes());            }
            ConstVal::Nil => {
                out.push(K_NIL);
                out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0]);
            }
            ConstVal::True => {
                out.push(K_TRUE);
                out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0]);
            }
            ConstVal::False => {
                out.push(K_FALSE);
                out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0]);
            }
        }
    }

    // 名字表
    for off in name_offsets {
        out.extend_from_slice(&off.to_le_bytes());
    }

    // 代码表
    for (i, c) in prog.codes.iter().enumerate() {
        let (off, len) = code_offsets[i];
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.push(c.name_idx);
        out.push(c.nparams);
        out.push(c.nlocals);
        out.push(0);
        out.push(c.caps.len() as u8);
        out.push(0);
    }

    // 捕获表：按代码对象顺序排列，每项 (父帧槽, 子帧槽)
    for c in &prog.codes {
        for (pslot, cslot) in &c.caps {
            out.push(*pslot);
            out.push(*cslot);
        }
    }

    // 字符串池
    out.extend_from_slice(&pool);

    // 指令流
    for c in &prog.codes {
        out.extend_from_slice(&c.code);
    }

    debug_assert_eq!(out.len(), total);
    Ok(out)
}

/// 反汇编（调试用）
pub fn disassemble(prog: &Program) -> String {
    let mut s = String::new();
    for (ci, c) in prog.codes.iter().enumerate() {
        s.push_str(&format!(
            "== 代码 {} 「{}」 参数 {} 局部 {} 捕获 {}\n",
            ci,
            prog.names.get(c.name_idx as usize).map(|x| x.as_str()).unwrap_or("?"),
            c.nparams,
            c.nlocals,
            c.caps.len()
        ));
        let mut ip = 0usize;
        while ip < c.code.len() {
            let op = c.code[ip];
            let start = ip;
            ip += 1;
            s.push_str(&format!("  {:04}  {:<22}", start, op_name(op)));
            match operand(op) {
                Operand::None => {}
                Operand::U8 => {
                    let a = c.code[ip];
                    ip += 1;
                    match op {
                        OP_BINOP => s.push_str(binop_name(a)),
                        OP_CMP => s.push_str(cmp_name(a)),
                        OP_CONST => {
                            s.push_str(&format!("#{}", a));
                        }
                        _ => s.push_str(&format!("{}", a)),
                    }
                }
                Operand::U16 => {
                    let a = u16::from_le_bytes([c.code[ip], c.code[ip + 1]]);
                    ip += 2;
                    s.push_str(&format!("-> {}", a));
                }
            }
            s.push('\n');
        }
    }
    s
}

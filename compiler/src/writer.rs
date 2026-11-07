use crate::codegen::{ConstVal, Program};
use crate::error::{CompileError, Result};
use crate::opcodes::*;

/// `.jmb` 魔数；`--check-langcard` 也从这里取，避免语言卡手写漂移。
pub const MAGIC: [u8; 4] = *b"JMB1";
/// 当前字节码格式版本（JMB v4：类表）。
pub const VERSION: u8 = 4;

const HEADER_LEN: usize = 12;
const CONST_ENTRY: usize = 8;
const NAME_ENTRY: usize = 2;
/// JMB v2：代码表每项 10 字节（新增 `ncap`）。v3 表结构与 v2 相同，仅版本号递增。
const CODE_ENTRY: usize = 10;
/// 捕获表每项 2 字节：`(父帧槽, 子帧槽)`。
const CAP_ENTRY: usize = 2;
/// JMB v4：类记录每项 8 字节。
const CLASS_REC: usize = 8;

pub fn write(prog: &Program) -> Result<Vec<u8>> {
    let nconsts = prog.consts.len();
    let nnames = prog.names.len();
    let ncodes = prog.codes.len();
    let nclasses = prog.classes.len();
    if nconsts > 255 || nnames > 255 || ncodes > 255 || nclasses > 255 {
        return Err(CompileError::new(
            "E105",
            1,
            1,
            "常量 / 名字 / 函数 / 类数量超过 255",
        ));
    }

    let const_start = HEADER_LEN;
    let name_start = const_start + CONST_ENTRY * nconsts;
    let code_start = name_start + NAME_ENTRY * nnames;
    let ncap_total: usize = prog.codes.iter().map(|c| c.caps.len()).sum();
    let cap_start = code_start + CODE_ENTRY * ncodes;
    let class_start = cap_start + CAP_ENTRY * ncap_total;
    let total_fields: usize = prog.classes.iter().map(|c| c.fields.len()).sum();
    let total_methods: usize = prog.classes.iter().map(|c| c.methods.len()).sum();
    let fields_start = class_start + CLASS_REC * nclasses;
    let methods_start = fields_start + total_fields;
    let pool_start = methods_start + 2 * total_methods;

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
    out.extend_from_slice(&MAGIC);
    out.push(VERSION); // version：JMB v4（类表）
    out.push(0); // flags
    out.push(nconsts as u8);
    out.push(nnames as u8);
    out.push(ncodes as u8);
    out.push(prog.entry);
    out.push(nclasses as u8); // p10：类数量
    out.push(0); // p11：保留

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

    // 类表：定长记录 → 字段名数组 → 方法(name,code)数组
    let mut rel_fields = CLASS_REC * nclasses;
    let mut rel_methods = CLASS_REC * nclasses + total_fields;
    for c in &prog.classes {
        out.push(c.name_idx);
        out.push(c.fields.len() as u8);
        out.push(c.methods.len() as u8);
        out.push(c.init_code);
        out.extend_from_slice(&(rel_fields as u16).to_le_bytes());
        out.extend_from_slice(&(rel_methods as u16).to_le_bytes());
        rel_fields += c.fields.len();
        rel_methods += 2 * c.methods.len();
    }
    for c in &prog.classes {
        for &f in &c.fields {
            out.push(f);
        }
    }
    for c in &prog.classes {
        for &(name_idx, code_idx) in &c.methods {
            out.push(name_idx);
            out.push(code_idx);
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
                Operand::U8U8 => {
                    let a = c.code[ip];
                    let b = c.code[ip + 1];
                    ip += 2;
                    s.push_str(&format!("{}, {}", a, b));
                }
            }
            s.push('\n');
        }
    }
    s
}

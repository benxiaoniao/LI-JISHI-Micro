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

/// 把 `.jmb` 字节生成 `jm_blob.c`（C 数组）。
///
/// 格式与 `tools/build-firmware.ps1` 的内联生成**逐字节一致**：每行 12 字节、4 空格缩进、
/// 十六进制大写两位、最后一个字节后不加逗号；固定符号名 `jm_app_jmb` / `jm_app_jmb_len`。
pub fn emit_blob(bytes: &[u8]) -> String {
    let mut s = String::new();
    s.push_str("#include <stdint.h>\n");
    s.push_str("const unsigned char jm_app_jmb[] = {\n");
    for (i, b) in bytes.iter().enumerate() {
        if i % 12 == 0 {
            s.push_str("    ");
        }
        s.push_str(&format!("0x{:02X}", b));
        if i + 1 < bytes.len() {
            s.push(',');
        }
        if i % 12 == 11 {
            s.push('\n');
        }
    }
    s.push_str("\n};\n");
    s.push_str(&format!("const uint32_t jm_app_jmb_len = {}u;\n", bytes.len()));
    s
}

/// 编码结果：`bytes` 是最终文件（可能含调试段），
/// `base_len` 是剥离调试段后的长度（体积上限按它算），`debug_len` 是调试段字节数。
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub base_len: usize,
    pub debug_len: usize,
}

fn put_var(v: u16, out: &mut Vec<u8>) {
    if v < 255 {
        out.push(v as u8);
    } else {
        out.push(255);
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// 有符号增量 → zigzag 无符号（-1→1，+1→2）。
fn zigzag(d: i32) -> u16 {
    (((d << 1) ^ (d >> 31)) & 0xffff) as u16
}

fn push_name(pool: &mut Vec<u8>, name: &str) -> Result<u16> {
    let off = pool.len();
    if off > 65535 {
        return Err(CompileError::new(
            "E131",
            1,
            1,
            "调试段的文件名池超过 64KB",
        ));
    }
    pool.extend_from_slice(name.as_bytes());
    pool.push(0);
    Ok(off as u16)
}

/// 生成尾部可选调试段（不含 6 B 尾索引）。`p5.bit0` 由调用方置位。
fn encode_debug(prog: &Program) -> Result<Option<Vec<u8>>> {
    let files: Vec<&str> = if prog.files.is_empty() {
        vec!["<未知>"]
    } else {
        prog.files.iter().map(|s| s.as_str()).collect()
    };
    if files.len() > 255 || prog.codes.len() > 255 {
        return Err(CompileError::new(
            "E131",
            1,
            1,
            "调试段的文件或代码对象超过 255 个",
        ));
    }

    let mut pool = Vec::new();
    let mut file_offs = Vec::new();
    for f in &files {
        file_offs.push(push_name(&mut pool, f)?);
    }

    // 先编码行记录（偏移相对代码对象起点），再回填代码目录。
    // 多文件时每条记录前置一个文件标签：0 = 沿用上一条，k = file_id + 1。
    let multi_file = files.len() > 1;
    let mut record_blobs: Vec<Vec<u8>> = Vec::new();
    for c in &prog.codes {
        let mut blob = Vec::new();
        let mut prev_off = 0u32;
        let mut prev_line = 0u32;
        let mut prev_file = 0u8;
        for (i, m) in c.marks.iter().enumerate() {
            if multi_file {
                if m.file_id as usize >= files.len() {
                    return Err(CompileError::new(
                        "E131",
                        1,
                        1,
                        "调试标记引用了不存在的源文件（编译器内部错误）",
                    ));
                }
                if i == 0 || m.file_id != prev_file {
                    blob.push(m.file_id + 1);
                } else {
                    blob.push(0);
                }
            }
            if i == 0 {
                put_var(m.off.min(65535) as u16, &mut blob);
                put_var(m.line.min(65535) as u16, &mut blob);
            } else {
                if m.off < prev_off {
                    return Err(CompileError::new(
                        "E131",
                        1,
                        1,
                        "调试标记的偏移倒退（编译器内部错误）",
                    ));
                }
                put_var((m.off - prev_off).min(65535) as u16, &mut blob);
                let d = m.line as i32 - prev_line as i32;
                put_var(zigzag(d), &mut blob);
            }
            prev_off = m.off;
            prev_line = m.line;
            prev_file = m.file_id;
        }
        record_blobs.push(blob);
    }

    let header_len = 16usize;
    let file_table_len = 2 * files.len();
    let code_dir_len = 6 * prog.codes.len();
    let str_off = header_len + file_table_len + code_dir_len;

    // 符号区（可选，flags.bit2）：目录（ncodes × u16，相对 sym_off）→ 各代码块
    // （`u8 n` + n × `{u8 槽号, u16 名字偏移}`）→ 名字池。没有任何具名局部时整段省略。
    let mut sym_pool: Vec<u8> = Vec::new();
    let mut code_syms: Vec<Vec<(u8, u16)>> = Vec::new();
    for c in &prog.codes {
        let mut v = Vec::new();
        for (slot, name) in &c.locals {
            v.push((*slot, push_name(&mut sym_pool, name)?));
        }
        code_syms.push(v);
    }
    let has_syms = code_syms.iter().any(|v| !v.is_empty());
    let mut sym_dir: Vec<u16> = Vec::new();
    let mut sym_blocks: Vec<Vec<u8>> = Vec::new();
    if has_syms {
        let dir_len = 2 * prog.codes.len();
        let blocks_total: usize = code_syms
            .iter()
            .map(|v| if v.is_empty() { 0 } else { 1 + 3 * v.len() })
            .sum();
        let pool_base = dir_len + blocks_total;
        let mut cursor = dir_len;
        for v in &code_syms {
            if v.is_empty() {
                sym_dir.push(0);
                continue;
            }
            sym_dir.push(cursor as u16);
            let mut block = Vec::new();
            block.push(v.len() as u8);
            for (slot, pool_off) in v {
                block.push(*slot);
                block.extend_from_slice(&((pool_base + *pool_off as usize) as u16).to_le_bytes());
            }
            cursor += block.len();
            sym_blocks.push(block);
        }
    }

    let mut record_off = str_off + pool.len();
    let records_total: usize = record_blobs.iter().map(|b| b.len()).sum();
    let sym_off = if has_syms {
        str_off + pool.len() + records_total
    } else {
        0
    };

    let mut section = Vec::new();
    section.extend_from_slice(b"DBG1");
    section.push(1); // 版本
    let mut flags = if multi_file { 2 } else { 0 }; // bit1 = 每条记录带文件标签
    if has_syms {
        flags |= 4; // bit2 = 有符号区
    }
    section.push(flags);
    section.push(files.len() as u8);
    section.push(prog.codes.len() as u8);
    section.extend_from_slice(&(header_len as u16).to_le_bytes());
    section.extend_from_slice(&((header_len + file_table_len) as u16).to_le_bytes());
    section.extend_from_slice(&(str_off as u16).to_le_bytes());
    section.extend_from_slice(&(sym_off as u16).to_le_bytes()); // 保留位 → 符号区偏移（0 = 无）

    for off in &file_offs {
        section.extend_from_slice(&off.to_le_bytes());
    }

    for (c, blob) in prog.codes.iter().zip(record_blobs.iter()) {
        section.extend_from_slice(&(record_off as u16).to_le_bytes());
        section.extend_from_slice(&(c.marks.len() as u16).to_le_bytes());
        section.push(c.file_id);
        section.push(0); // 保留
        record_off += blob.len();
    }

    section.extend_from_slice(&pool);
    for blob in &record_blobs {
        section.extend_from_slice(blob);
    }

    if has_syms {
        for off in &sym_dir {
            section.extend_from_slice(&off.to_le_bytes());
        }
        for block in &sym_blocks {
            section.extend_from_slice(block);
        }
        section.extend_from_slice(&sym_pool);
    }

    if section.len() > 65535 {
        return Err(CompileError::new(
            "E131",
            1,
            1,
            "调试段超过 64KB；可用 --strip-debug 剥离后再试",
        ));
    }
    Ok(Some(section))
}

pub fn write(prog: &Program, with_debug: bool) -> Result<Encoded> {
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
            ".jmb 超过 64KB（u16 偏移的上限；调试段不计入）",
        ));
    }

    let debug = if with_debug { encode_debug(prog)? } else { None };
    let mut out = Vec::with_capacity(total + debug.as_ref().map(|d| d.len() + 6).unwrap_or(0));
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

    let debug_len = match debug {
        Some(mut d) => {
            out[5] |= 1; // p5.bit0：带尾部调试段
            d.extend_from_slice(&(d.len() as u16).to_le_bytes());
            d.extend_from_slice(b"DBG1");
            let n = d.len();
            out.extend_from_slice(&d);
            n
        }
        None => 0,
    };

    Ok(Encoded {
        bytes: out,
        base_len: total,
        debug_len,
    })
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

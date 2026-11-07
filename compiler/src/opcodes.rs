//! 操作码定义 —— 编译器与 C 虚拟机共用的唯一事实来源。
//! `micc --emit-header` 会把本文件的内容导出成 `runtime/include/jm_opcodes.h`。

pub const OP_HALT: u8 = 0;
pub const OP_CONST: u8 = 1;
pub const OP_LOAD_LOCAL: u8 = 2;
pub const OP_STORE_LOCAL: u8 = 3;
pub const OP_LOAD_GLOBAL: u8 = 4;
pub const OP_STORE_GLOBAL: u8 = 5;
pub const OP_POP: u8 = 6;
pub const OP_DUP: u8 = 7;
pub const OP_SWAP: u8 = 8;
pub const OP_BINOP: u8 = 9;
pub const OP_NEG: u8 = 10;
pub const OP_NOT: u8 = 11;
pub const OP_CMP: u8 = 12;
pub const OP_JUMP: u8 = 13;
pub const OP_JUMP_IF_FALSE: u8 = 14;
pub const OP_JUMP_IF_TRUE: u8 = 15;
pub const OP_JUMP_IF_FALSE_OR_POP: u8 = 16;
pub const OP_JUMP_IF_TRUE_OR_POP: u8 = 17;
pub const OP_CALL: u8 = 18;
pub const OP_RETURN: u8 = 19;
pub const OP_BUILD_LIST: u8 = 20;
pub const OP_BUILD_DICT: u8 = 21;
pub const OP_GET_ITEM: u8 = 22;
pub const OP_SET_ITEM: u8 = 23;
pub const OP_GET_ITER: u8 = 24;
pub const OP_FOR_ITER: u8 = 25;
pub const OP_MAKE_FUNCTION: u8 = 26;
pub const OP_IMPORT: u8 = 27;
/// 用当前帧的若干槽创建闭包对象；立即数是代码对象下标，捕获表在代码表里。
pub const OP_MAKE_CLOSURE: u8 = 28;
pub const OP_TRY: u8 = 29;
pub const OP_TRY_END: u8 = 30;
pub const OP_THROW: u8 = 31;
/// JM14 类：新建实例（U8 类下标）
pub const OP_NEW_INSTANCE: u8 = 32;
/// 读字段（U8 字段名下标）
pub const OP_GET_FIELD: u8 = 33;
/// 写字段（U8 字段名下标）
pub const OP_SET_FIELD: u8 = 34;
/// 调方法（U8 方法名下标, U8 实参数）
pub const OP_CALL_METHOD: u8 = 35;
/// 调初始化（U8 类下标, U8 实参数）
pub const OP_CALL_INIT: u8 = 36;
/// 方法帧返回（无操作数）
pub const OP_RETURN_METHOD: u8 = 37;

/// 运算子码
pub const BIN_ADD: u8 = 0;
pub const BIN_SUB: u8 = 1;
pub const BIN_MUL: u8 = 2;
pub const BIN_DIV: u8 = 3;
pub const BIN_FLOORDIV: u8 = 4;
pub const BIN_MOD: u8 = 5;
pub const BIN_POW: u8 = 6;

/// 比较子码
pub const CMP_EQ: u8 = 0;
pub const CMP_NE: u8 = 1;
pub const CMP_LT: u8 = 2;
pub const CMP_GT: u8 = 3;
pub const CMP_LE: u8 = 4;
pub const CMP_GE: u8 = 5;

/// 常量种类
pub const K_INT: u8 = 0;
pub const K_STR: u8 = 1;
pub const K_NIL: u8 = 2;
pub const K_TRUE: u8 = 3;
pub const K_FALSE: u8 = 4;
/// 小数常量：4 字节 int32 为 Q12 定点原始值（真实值 × 4096）。
pub const K_DEC: u8 = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    None,
    U8,
    U16,
    /// 两个连续 U8：用于 `CALL_METHOD` / `CALL_INIT`（名字/类下标 + 实参数）。
    U8U8,
}

/// 形态表：决定一条指令占几个字节。
pub fn operand(op: u8) -> Operand {
    match op {
        OP_HALT | OP_POP | OP_DUP | OP_SWAP | OP_NEG | OP_NOT | OP_RETURN | OP_GET_ITEM
        | OP_SET_ITEM | OP_GET_ITER | OP_TRY_END | OP_THROW | OP_RETURN_METHOD => Operand::None,
        OP_JUMP | OP_JUMP_IF_FALSE | OP_JUMP_IF_TRUE | OP_JUMP_IF_FALSE_OR_POP
        | OP_JUMP_IF_TRUE_OR_POP | OP_FOR_ITER | OP_TRY => Operand::U16,
        OP_CALL_METHOD | OP_CALL_INIT => Operand::U8U8,
        _ => Operand::U8,
    }
}

pub fn op_name(op: u8) -> &'static str {
    match op {
        OP_HALT => "HALT",
        OP_CONST => "CONST",
        OP_LOAD_LOCAL => "LOAD_LOCAL",
        OP_STORE_LOCAL => "STORE_LOCAL",
        OP_LOAD_GLOBAL => "LOAD_GLOBAL",
        OP_STORE_GLOBAL => "STORE_GLOBAL",
        OP_POP => "POP",
        OP_DUP => "DUP",
        OP_SWAP => "SWAP",
        OP_BINOP => "BINOP",
        OP_NEG => "NEG",
        OP_NOT => "NOT",
        OP_CMP => "CMP",
        OP_JUMP => "JUMP",
        OP_JUMP_IF_FALSE => "JUMP_IF_FALSE",
        OP_JUMP_IF_TRUE => "JUMP_IF_TRUE",
        OP_JUMP_IF_FALSE_OR_POP => "JUMP_IF_FALSE_OR_POP",
        OP_JUMP_IF_TRUE_OR_POP => "JUMP_IF_TRUE_OR_POP",
        OP_CALL => "CALL",
        OP_RETURN => "RETURN",
        OP_BUILD_LIST => "BUILD_LIST",
        OP_BUILD_DICT => "BUILD_DICT",
        OP_GET_ITEM => "GET_ITEM",
        OP_SET_ITEM => "SET_ITEM",
        OP_GET_ITER => "GET_ITER",
        OP_FOR_ITER => "FOR_ITER",
        OP_MAKE_FUNCTION => "MAKE_FUNCTION",
        OP_IMPORT => "IMPORT",
        OP_MAKE_CLOSURE => "MAKE_CLOSURE",
        OP_TRY => "TRY",
        OP_TRY_END => "TRY_END",
        OP_THROW => "THROW",
        OP_NEW_INSTANCE => "NEW_INSTANCE",
        OP_GET_FIELD => "GET_FIELD",
        OP_SET_FIELD => "SET_FIELD",
        OP_CALL_METHOD => "CALL_METHOD",
        OP_CALL_INIT => "CALL_INIT",
        OP_RETURN_METHOD => "RETURN_METHOD",
        _ => "?",
    }
}

pub fn binop_name(sub: u8) -> &'static str {
    match sub {
        BIN_ADD => "+",
        BIN_SUB => "-",
        BIN_MUL => "*",
        BIN_DIV => "/",
        BIN_FLOORDIV => "//",
        BIN_MOD => "%",
        BIN_POW => "**",
        _ => "?",
    }
}

pub fn cmp_name(sub: u8) -> &'static str {
    match sub {
        CMP_EQ => "==",
        CMP_NE => "!=",
        CMP_LT => "<",
        CMP_GT => ">",
        CMP_LE => "<=",
        CMP_GE => ">=",
        _ => "?",
    }
}

/// 导出 C 头文件（运行时要与编译器逐条对齐）。
pub fn emit_c_header() -> String {
    let mut s = String::new();
    s.push_str("/* 由 micc --emit-header 生成，请勿手改。 */\n");
    s.push_str("#ifndef JM_OPCODES_H\n#define JM_OPCODES_H\n\n");
    let ops: [(u8, &str); 38] = [
        (OP_HALT, "OP_HALT"),
        (OP_CONST, "OP_CONST"),
        (OP_LOAD_LOCAL, "OP_LOAD_LOCAL"),
        (OP_STORE_LOCAL, "OP_STORE_LOCAL"),
        (OP_LOAD_GLOBAL, "OP_LOAD_GLOBAL"),
        (OP_STORE_GLOBAL, "OP_STORE_GLOBAL"),
        (OP_POP, "OP_POP"),
        (OP_DUP, "OP_DUP"),
        (OP_SWAP, "OP_SWAP"),
        (OP_BINOP, "OP_BINOP"),
        (OP_NEG, "OP_NEG"),
        (OP_NOT, "OP_NOT"),
        (OP_CMP, "OP_CMP"),
        (OP_JUMP, "OP_JUMP"),
        (OP_JUMP_IF_FALSE, "OP_JUMP_IF_FALSE"),
        (OP_JUMP_IF_TRUE, "OP_JUMP_IF_TRUE"),
        (OP_JUMP_IF_FALSE_OR_POP, "OP_JUMP_IF_FALSE_OR_POP"),
        (OP_JUMP_IF_TRUE_OR_POP, "OP_JUMP_IF_TRUE_OR_POP"),
        (OP_CALL, "OP_CALL"),
        (OP_RETURN, "OP_RETURN"),
        (OP_BUILD_LIST, "OP_BUILD_LIST"),
        (OP_BUILD_DICT, "OP_BUILD_DICT"),
        (OP_GET_ITEM, "OP_GET_ITEM"),
        (OP_SET_ITEM, "OP_SET_ITEM"),
        (OP_GET_ITER, "OP_GET_ITER"),
        (OP_FOR_ITER, "OP_FOR_ITER"),
        (OP_MAKE_FUNCTION, "OP_MAKE_FUNCTION"),
        (OP_IMPORT, "OP_IMPORT"),
        (OP_MAKE_CLOSURE, "OP_MAKE_CLOSURE"),
        (OP_TRY, "OP_TRY"),
        (OP_TRY_END, "OP_TRY_END"),
        (OP_THROW, "OP_THROW"),
        (OP_NEW_INSTANCE, "OP_NEW_INSTANCE"),
        (OP_GET_FIELD, "OP_GET_FIELD"),
        (OP_SET_FIELD, "OP_SET_FIELD"),
        (OP_CALL_METHOD, "OP_CALL_METHOD"),
        (OP_CALL_INIT, "OP_CALL_INIT"),
        (OP_RETURN_METHOD, "OP_RETURN_METHOD"),
    ];
    for (v, n) in ops {
        s.push_str(&format!("#define {n} {v}\n"));
    }
    s.push_str("\n");
    s.push_str(&format!("#define BIN_ADD {BIN_ADD}\n"));
    s.push_str(&format!("#define BIN_SUB {BIN_SUB}\n"));
    s.push_str(&format!("#define BIN_MUL {BIN_MUL}\n"));
    s.push_str(&format!("#define BIN_DIV {BIN_DIV}\n"));
    s.push_str(&format!("#define BIN_FLOORDIV {BIN_FLOORDIV}\n"));
    s.push_str(&format!("#define BIN_MOD {BIN_MOD}\n"));
    s.push_str(&format!("#define BIN_POW {BIN_POW}\n\n"));
    s.push_str(&format!("#define CMP_EQ {CMP_EQ}\n"));
    s.push_str(&format!("#define CMP_NE {CMP_NE}\n"));
    s.push_str(&format!("#define CMP_LT {CMP_LT}\n"));
    s.push_str(&format!("#define CMP_GT {CMP_GT}\n"));
    s.push_str(&format!("#define CMP_LE {CMP_LE}\n"));
    s.push_str(&format!("#define CMP_GE {CMP_GE}\n\n"));
    s.push_str(&format!("#define K_INT {K_INT}\n"));
    s.push_str(&format!("#define K_STR {K_STR}\n"));
    s.push_str(&format!("#define K_NIL {K_NIL}\n"));
    s.push_str(&format!("#define K_TRUE {K_TRUE}\n"));
    s.push_str(&format!("#define K_FALSE {K_FALSE}\n"));
    s.push_str(&format!("#define K_DEC {K_DEC}\n\n"));
    s.push_str("#endif\n");
    s
}

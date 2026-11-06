use crate::ast::*;
use crate::error::{CompileError, Result};
use crate::opcodes::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum ConstVal {
    Int(i32),
    Str(String),
    Nil,
    True,
    False,
}

#[derive(Debug)]
pub struct CodeObj {
    pub name_idx: u8,
    pub nparams: u8,
    pub nlocals: u8,
    pub code: Vec<u8>,
}

#[derive(Debug)]
pub struct Program {
    pub consts: Vec<ConstVal>,
    pub names: Vec<String>,
    pub codes: Vec<CodeObj>,
    pub entry: u8,
}

struct Builder {
    consts: Vec<ConstVal>,
    names: Vec<String>,
    name_map: HashMap<String, u8>,
    codes: Vec<CodeObj>,
}

impl Builder {
    fn new() -> Self {
        Self {
            consts: Vec::new(),
            names: Vec::new(),
            name_map: HashMap::new(),
            codes: Vec::new(),
        }
    }

    fn intern_name(&mut self, name: &str, line: u32, col: u32) -> Result<u8> {
        if let Some(&i) = self.name_map.get(name) {
            return Ok(i);
        }
        let i = self.names.len();
        if i >= 255 {
            return Err(CompileError::new(
                "E105",
                line,
                col,
                "名字超过 255 个（Micro Core v0 的上限）",
            ));
        }
        self.names.push(name.to_string());
        self.name_map.insert(name.to_string(), i as u8);
        Ok(i as u8)
    }

    fn const_int(&mut self, v: i64, line: u32, col: u32) -> Result<u8> {
        if !(-(1i64 << 28)..=(1i64 << 28) - 1).contains(&v) {
            return Err(CompileError::new(
                "E110",
                line,
                col,
                "整数超出 29 位范围（-268435456 .. 268435455）",
            ));
        }
        let v = v as i32;
        for (i, c) in self.consts.iter().enumerate() {
            if let ConstVal::Int(x) = c {
                if *x == v {
                    return Ok(i as u8);
                }
            }
        }
        self.push_const(ConstVal::Int(v), line, col)
    }

    fn const_str(&mut self, s: &str, line: u32, col: u32) -> Result<u8> {
        for (i, c) in self.consts.iter().enumerate() {
            if let ConstVal::Str(x) = c {
                if x == s {
                    return Ok(i as u8);
                }
            }
        }
        self.push_const(ConstVal::Str(s.to_string()), line, col)
    }

    fn const_nil(&mut self, line: u32, col: u32) -> Result<u8> {
        for (i, c) in self.consts.iter().enumerate() {
            if matches!(c, ConstVal::Nil) {
                return Ok(i as u8);
            }
        }
        self.push_const(ConstVal::Nil, line, col)
    }

    fn const_bool(&mut self, v: bool, line: u32, col: u32) -> Result<u8> {
        for (i, c) in self.consts.iter().enumerate() {
            match c {
                ConstVal::True if v => return Ok(i as u8),
                ConstVal::False if !v => return Ok(i as u8),
                _ => {}
            }
        }
        self.push_const(if v { ConstVal::True } else { ConstVal::False }, line, col)
    }

    fn push_const(&mut self, c: ConstVal, line: u32, col: u32) -> Result<u8> {
        let i = self.consts.len();
        if i >= 255 {
            return Err(CompileError::new(
                "E105",
                line,
                col,
                "常量超过 255 个（Micro Core v0 的上限）",
            ));
        }
        self.consts.push(c);
        Ok(i as u8)
    }
}

struct LoopCtx {
    is_foreach: bool,
    continue_target: usize,
    breaks: Vec<usize>,
}

struct FnState {
    code: Vec<u8>,
    slots: HashMap<String, u8>,
    parent_names: Vec<String>,
    nlocals: u8,
    temps: Vec<u8>,
    loops: Vec<LoopCtx>,
    is_module: bool,
}

impl FnState {
    fn emit(&mut self, op: u8) {
        self.code.push(op);
    }
    fn emit_u8(&mut self, op: u8, a: u8) {
        self.code.push(op);
        self.code.push(a);
    }
    fn here(&self) -> usize {
        self.code.len()
    }
    fn jump(&mut self, op: u8) -> usize {
        let at = self.code.len() + 1;
        self.code.push(op);
        self.code.push(0);
        self.code.push(0);
        at
    }
    fn patch(&mut self, at: usize, target: usize) {
        self.code[at] = (target & 0xff) as u8;
        self.code[at + 1] = ((target >> 8) & 0xff) as u8;
    }
    fn alloc_temp(&mut self, line: u32, col: u32) -> Result<u8> {
        if let Some(t) = self.temps.pop() {
            return Ok(t);
        }
        if self.nlocals >= 255 {
            return Err(CompileError::new("E104", line, col, "局部变量超过 255 个"));
        }
        let t = self.nlocals;
        self.nlocals += 1;
        Ok(t)
    }
    fn free_temp(&mut self, t: u8) {
        self.temps.push(t);
    }
}

enum Res {
    Local(u8),
    Global(u8),
}

fn resolve(fs: &FnState, b: &mut Builder, name: &str, line: u32, col: u32) -> Result<Res> {
    if !fs.is_module {
        if let Some(&slot) = fs.slots.get(name) {
            return Ok(Res::Local(slot));
        }
        if fs.parent_names.iter().any(|n| n == name) {
            return Err(CompileError::new(
                "E101",
                line,
                col,
                format!("闭包暂不支持：内层函数读到了外层的局部变量「{}」", name),
            )
            .with_hint("v0 只有模块级函数；需要共享数据就放模块级「令」"));
        }
    }
    Ok(Res::Global(b.intern_name(name, line, col)?))
}

fn store_name(fs: &mut FnState, b: &mut Builder, name: &str, line: u32, col: u32) -> Result<()> {
    match resolve(fs, b, name, line, col)? {
        Res::Local(s) => fs.emit_u8(OP_STORE_LOCAL, s),
        Res::Global(g) => fs.emit_u8(OP_STORE_GLOBAL, g),
    }
    Ok(())
}

fn load_name(fs: &mut FnState, b: &mut Builder, name: &str, line: u32, col: u32) -> Result<()> {
    match resolve(fs, b, name, line, col)? {
        Res::Local(s) => fs.emit_u8(OP_LOAD_LOCAL, s),
        Res::Global(g) => fs.emit_u8(OP_LOAD_GLOBAL, g),
    }
    Ok(())
}

fn collect_decls(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Let(n, _) => out.push(n.clone()),
            StmtKind::ForEach(n, _, _) => out.push(n.clone()),
            StmtKind::FuncDef { name, .. } => out.push(name.clone()),
            StmtKind::If { branches, else_body } => {
                for (_, body) in branches {
                    collect_decls(body, out);
                }
                if let Some(b) = else_body {
                    collect_decls(b, out);
                }
            }
            StmtKind::While(_, body) | StmtKind::Repeat(_, body) => collect_decls(body, out),
            _ => {}
        }
    }
}

pub fn compile(stmts: &[Stmt]) -> Result<Program> {
    let mut b = Builder::new();
    let module_name = b.intern_name("<模块>", 1, 1)?;
    b.codes.push(CodeObj {
        name_idx: module_name,
        nparams: 0,
        nlocals: 0,
        code: Vec::new(),
    });

    let mut fs = FnState {
        code: Vec::new(),
        slots: HashMap::new(),
        parent_names: Vec::new(),
        nlocals: 0,
        temps: Vec::new(),
        loops: Vec::new(),
        is_module: true,
    };

    compile_stmts(&mut fs, &mut b, stmts)?;
    fs.emit(OP_HALT);

    b.codes[0].nlocals = fs.nlocals;
    b.codes[0].code = fs.code;

    Ok(Program {
        consts: b.consts,
        names: b.names,
        codes: b.codes,
        entry: 0,
    })
}

fn compile_function(
    b: &mut Builder,
    name: &str,
    params: &[String],
    body: &[Stmt],
    parent_names: &[String],
    line: u32,
    col: u32,
) -> Result<u8> {
    let idx = b.codes.len();
    if idx >= 255 {
        return Err(CompileError::new("E106", line, col, "函数超过 255 个"));
    }
    let idx = idx as u8;
    let name_idx = b.intern_name(name, line, col)?;
    b.codes.push(CodeObj {
        name_idx,
        nparams: params.len() as u8,
        nlocals: params.len() as u8,
        code: Vec::new(),
    });

    let mut fs = FnState {
        code: Vec::new(),
        slots: HashMap::new(),
        parent_names: parent_names.to_vec(),
        nlocals: params.len() as u8,
        temps: Vec::new(),
        loops: Vec::new(),
        is_module: false,
    };

    for (i, p) in params.iter().enumerate() {
        if fs.slots.insert(p.clone(), i as u8).is_some() {
            return Err(CompileError::new("E109", line, col, "参数重名"));
        }
    }
    let mut decls = Vec::new();
    collect_decls(body, &mut decls);
    for d in decls {
        if fs.slots.contains_key(&d) {
            continue;
        }
        if fs.nlocals >= 255 {
            return Err(CompileError::new("E104", line, col, "局部变量超过 255 个"));
        }
        let slot = fs.nlocals;
        fs.nlocals += 1;
        fs.slots.insert(d, slot);
    }

    compile_stmts(&mut fs, b, body)?;

    // 没有显式返回就返回「空」
    let nil = b.const_nil(line, col)?;
    fs.emit_u8(OP_CONST, nil);
    fs.emit(OP_RETURN);

    b.codes[idx as usize].nlocals = fs.nlocals;
    b.codes[idx as usize].code = fs.code;
    Ok(idx)
}

fn compile_stmts(fs: &mut FnState, b: &mut Builder, stmts: &[Stmt]) -> Result<()> {
    for s in stmts {
        compile_stmt(fs, b, s)?;
    }
    Ok(())
}

fn compile_stmt(fs: &mut FnState, b: &mut Builder, s: &Stmt) -> Result<()> {
    match &s.kind {
        StmtKind::Let(name, e) => {
            compile_expr(fs, b, e)?;
            store_name(fs, b, name, s.line, e.col)?;
        }
        StmtKind::Assign(name, e) => {
            compile_expr(fs, b, e)?;
            store_name(fs, b, name, s.line, e.col)?;
        }
        StmtKind::AssignIndex(obj, idx, val) => {
            compile_expr(fs, b, obj)?;
            compile_expr(fs, b, idx)?;
            compile_expr(fs, b, val)?;
            fs.emit(OP_SET_ITEM);
        }
        StmtKind::Expr(e) => {
            compile_expr(fs, b, e)?;
            fs.emit(OP_POP);
        }
        StmtKind::If {
            branches,
            else_body,
        } => {
            compile_if(fs, b, branches, else_body.as_deref(), 0)?;
        }
        StmtKind::While(cond, body) => {
            let start = fs.here();
            compile_expr(fs, b, cond)?;
            let jf = fs.jump(OP_JUMP_IF_FALSE);
            fs.loops.push(LoopCtx {
                is_foreach: false,
                continue_target: start,
                breaks: Vec::new(),
            });
            compile_stmts(fs, b, body)?;
            let ctx = fs.loops.pop().unwrap();
            let j = fs.jump(OP_JUMP);
            fs.patch(j, start);
            let end = fs.here();
            fs.patch(jf, end);
            for p in ctx.breaks {
                fs.patch(p, end);
            }
        }
        StmtKind::Repeat(count, body) => {
            compile_expr(fs, b, count)?;
            let t = fs.alloc_temp(s.line, 1)?;
            fs.emit_u8(OP_STORE_LOCAL, t);
            let start = fs.here();
            fs.emit_u8(OP_LOAD_LOCAL, t);
            let zero = b.const_int(0, s.line, 1)?;
            fs.emit_u8(OP_CONST, zero);
            fs.emit_u8(OP_CMP, CMP_GT);
            let jf = fs.jump(OP_JUMP_IF_FALSE);
            fs.loops.push(LoopCtx {
                is_foreach: false,
                continue_target: start,
                breaks: Vec::new(),
            });
            compile_stmts(fs, b, body)?;
            let ctx = fs.loops.pop().unwrap();
            fs.emit_u8(OP_LOAD_LOCAL, t);
            let one = b.const_int(1, s.line, 1)?;
            fs.emit_u8(OP_CONST, one);
            fs.emit_u8(OP_BINOP, BIN_SUB);
            fs.emit_u8(OP_STORE_LOCAL, t);
            let j = fs.jump(OP_JUMP);
            fs.patch(j, start);
            let end = fs.here();
            fs.patch(jf, end);
            for p in ctx.breaks {
                fs.patch(p, end);
            }
            fs.free_temp(t);
        }
        StmtKind::ForEach(name, iterable, body) => {
            compile_expr(fs, b, iterable)?;
            fs.emit(OP_GET_ITER);
            let start = fs.here();
            let jf = fs.jump(OP_FOR_ITER);
            store_name(fs, b, name, s.line, 1)?;
            fs.loops.push(LoopCtx {
                is_foreach: true,
                continue_target: start,
                breaks: Vec::new(),
            });
            compile_stmts(fs, b, body)?;
            let ctx = fs.loops.pop().unwrap();
            let j = fs.jump(OP_JUMP);
            fs.patch(j, start);
            let end = fs.here();
            fs.patch(jf, end);
            for p in ctx.breaks {
                fs.patch(p, end);
            }
        }
        StmtKind::Break => {
            let Some(ctx) = fs.loops.last() else {
                return Err(CompileError::new("E109", s.line, 1, "「中断」只能写在循环里"));
            };
            let is_foreach = ctx.is_foreach;
            if is_foreach {
                fs.emit(OP_POP); // 丢掉迭代器
            }
            let p = fs.jump(OP_JUMP);
            fs.loops.last_mut().unwrap().breaks.push(p);
        }
        StmtKind::Continue => {
            let Some(ctx) = fs.loops.last() else {
                return Err(CompileError::new("E109", s.line, 1, "「继续」只能写在循环里"));
            };
            let target = ctx.continue_target;
            let j = fs.jump(OP_JUMP);
            fs.patch(j, target);
        }
        StmtKind::FuncDef { name, params, body } => {
            let mut parent = fs.parent_names.clone();
            for k in fs.slots.keys() {
                parent.push(k.clone());
            }
            let idx = compile_function(b, name, params, body, &parent, s.line, 1)?;
            fs.emit_u8(OP_MAKE_FUNCTION, idx);
            store_name(fs, b, name, s.line, 1)?;
        }
        StmtKind::Return(e) => {
            match e {
                Some(e) => compile_expr(fs, b, e)?,
                None => {
                    let nil = b.const_nil(s.line, 1)?;
                    fs.emit_u8(OP_CONST, nil);
                }
            }
            fs.emit(OP_RETURN);
        }
    }
    Ok(())
}

fn compile_if(
    fs: &mut FnState,
    b: &mut Builder,
    branches: &[(Expr, Vec<Stmt>)],
    else_body: Option<&[Stmt]>,
    index: usize,
) -> Result<()> {
    if index >= branches.len() {
        if let Some(body) = else_body {
            compile_stmts(fs, b, body)?;
        }
        return Ok(());
    }
    let (cond, body) = &branches[index];
    compile_expr(fs, b, cond)?;
    let jf = fs.jump(OP_JUMP_IF_FALSE);
    compile_stmts(fs, b, body)?;
    let jend = fs.jump(OP_JUMP);
    let next = fs.here();
    fs.patch(jf, next);
    compile_if(fs, b, branches, else_body, index + 1)?;
    let end = fs.here();
    fs.patch(jend, end);
    Ok(())
}

fn binop_code(op: BinOp) -> u8 {
    match op {
        BinOp::Add => BIN_ADD,
        BinOp::Sub => BIN_SUB,
        BinOp::Mul => BIN_MUL,
        BinOp::Div => BIN_DIV,
        BinOp::FloorDiv => BIN_FLOORDIV,
        BinOp::Mod => BIN_MOD,
        BinOp::Pow => BIN_POW,
        _ => unreachable!(),
    }
}

fn cmp_code(op: CmpOp) -> u8 {
    match op {
        CmpOp::Eq => CMP_EQ,
        CmpOp::Ne => CMP_NE,
        CmpOp::Lt => CMP_LT,
        CmpOp::Gt => CMP_GT,
        CmpOp::Le => CMP_LE,
        CmpOp::Ge => CMP_GE,
    }
}

fn compile_expr(fs: &mut FnState, b: &mut Builder, e: &Expr) -> Result<()> {
    match &e.kind {
        ExprKind::Int(v) => {
            let c = b.const_int(*v, e.line, e.col)?;
            fs.emit_u8(OP_CONST, c);
        }
        ExprKind::Str(s) => {
            let c = b.const_str(s, e.line, e.col)?;
            fs.emit_u8(OP_CONST, c);
        }
        ExprKind::Nil => {
            let c = b.const_nil(e.line, e.col)?;
            fs.emit_u8(OP_CONST, c);
        }
        ExprKind::True => {
            let c = b.const_bool(true, e.line, e.col)?;
            fs.emit_u8(OP_CONST, c);
        }
        ExprKind::False => {
            let c = b.const_bool(false, e.line, e.col)?;
            fs.emit_u8(OP_CONST, c);
        }
        ExprKind::Name(name) => load_name(fs, b, name, e.line, e.col)?,
        ExprKind::List(items) => {
            for it in items {
                compile_expr(fs, b, it)?;
            }
            if items.len() > 255 {
                return Err(CompileError::new("E105", e.line, e.col, "列表元素超过 255 个"));
            }
            fs.emit_u8(OP_BUILD_LIST, items.len() as u8);
        }
        ExprKind::Dict(pairs) => {
            for (k, v) in pairs {
                compile_expr(fs, b, k)?;
                compile_expr(fs, b, v)?;
            }
            if pairs.len() > 255 {
                return Err(CompileError::new("E105", e.line, e.col, "字典项超过 255 个"));
            }
            fs.emit_u8(OP_BUILD_DICT, pairs.len() as u8);
        }
        ExprKind::Bin(BinOp::And, l, r) => {
            compile_expr(fs, b, l)?;
            let j = fs.jump(OP_JUMP_IF_FALSE_OR_POP);
            compile_expr(fs, b, r)?;
            let here = fs.here();
            fs.patch(j, here);
        }
        ExprKind::Bin(BinOp::Or, l, r) => {
            compile_expr(fs, b, l)?;
            let j = fs.jump(OP_JUMP_IF_TRUE_OR_POP);
            compile_expr(fs, b, r)?;
            let here = fs.here();
            fs.patch(j, here);
        }
        ExprKind::Bin(op, l, r) => {
            compile_expr(fs, b, l)?;
            compile_expr(fs, b, r)?;
            fs.emit_u8(OP_BINOP, binop_code(*op));
        }
        ExprKind::Unary(UnOp::Neg, x) => {
            compile_expr(fs, b, x)?;
            fs.emit(OP_NEG);
        }
        ExprKind::Unary(UnOp::Not, x) => {
            compile_expr(fs, b, x)?;
            fs.emit(OP_NOT);
        }
        ExprKind::Compare(first, pairs) => {
            compile_expr(fs, b, first)?;
            let t0 = fs.alloc_temp(e.line, e.col)?;
            fs.emit_u8(OP_STORE_LOCAL, t0);
            let mut patches = Vec::new();
            for (i, (op, right)) in pairs.iter().enumerate() {
                compile_expr(fs, b, right)?;
                let t1 = fs.alloc_temp(e.line, e.col)?;
                fs.emit_u8(OP_STORE_LOCAL, t1);
                fs.emit_u8(OP_LOAD_LOCAL, t0);
                fs.emit_u8(OP_LOAD_LOCAL, t1);
                fs.emit_u8(OP_CMP, cmp_code(*op));
                if i + 1 < pairs.len() {
                    patches.push(fs.jump(OP_JUMP_IF_FALSE_OR_POP));
                    fs.emit_u8(OP_LOAD_LOCAL, t1);
                    fs.emit_u8(OP_STORE_LOCAL, t0);
                }
                fs.free_temp(t1);
            }
            let end = fs.here();
            for p in patches {
                fs.patch(p, end);
            }
            fs.free_temp(t0);
        }
        ExprKind::Call(callee, args) => {
            compile_expr(fs, b, callee)?;
            for a in args {
                compile_expr(fs, b, a)?;
            }
            if args.len() > 255 {
                return Err(CompileError::new("E114", e.line, e.col, "实参超过 255 个"));
            }
            fs.emit_u8(OP_CALL, args.len() as u8);
        }
        ExprKind::Index(obj, idx) => {
            compile_expr(fs, b, obj)?;
            compile_expr(fs, b, idx)?;
            fs.emit(OP_GET_ITEM);
        }
    }
    Ok(())
}

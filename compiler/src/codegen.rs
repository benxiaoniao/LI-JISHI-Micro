use crate::ast::*;
use crate::error::{CompileError, Result};
use crate::native_spec;
use crate::opcodes::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub enum ConstVal {
    Int(i32),
    /// 小数：Q12 定点原始值（真实值 × 4096）。
    Dec(i32),
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
    /// 闭包捕获表：`(父帧槽, 子帧槽)`，按出现顺序排列。
    pub caps: Vec<(u8, u8)>,
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

    fn const_dec(&mut self, v: i64, line: u32, col: u32) -> Result<u8> {
        let v = v as i32;
        for (i, c) in self.consts.iter().enumerate() {
            if let ConstVal::Dec(x) = c {
                if *x == v {
                    return Ok(i as u8);
                }
            }
        }
        self.push_const(ConstVal::Dec(v), line, col)
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
    /// 本函数捕获的外层变量：名字 → 本帧槽（只读）。
    captures: HashMap<String, u8>,
    /// 父帧里可见的槽：名字 → 父帧槽。
    parent_slots: HashMap<String, u8>,
    /// 本函数的捕获表，按首次出现顺序追加。
    caps: Vec<(u8, u8)>,
    /// 正在编译的函数名，用于拒绝「捕获自己的名字」。
    self_name: Option<String>,
    nlocals: u8,
    temps: Vec<u8>,
    loops: Vec<LoopCtx>,
    is_module: bool,
    /// 顶层 `导入 模块` 汇总出来的原生函数名。
    imported: HashSet<String>,
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
    /// 闭包捕获槽：读可以，写报 E117。
    Capture(u8),
    Global(u8),
}

fn resolve(fs: &mut FnState, b: &mut Builder, name: &str, line: u32, col: u32) -> Result<Res> {
    if !fs.is_module {
        if let Some(&slot) = fs.slots.get(name) {
            return Ok(Res::Local(slot));
        }
        if let Some(&slot) = fs.captures.get(name) {
            return Ok(Res::Capture(slot));
        }
        if let Some(&pslot) = fs.parent_slots.get(name) {
            return capture_from_parent(fs, name, pslot, line, col);
        }
    }
    Ok(Res::Global(b.intern_name(name, line, col)?))
}

/// 在父帧槽 `pslot` 与子帧之间建立捕获：本帧分配一个新槽并记进捕获表。
fn capture_from_parent(
    fs: &mut FnState,
    name: &str,
    pslot: u8,
    line: u32,
    col: u32,
) -> Result<Res> {
    if fs.self_name.as_deref() == Some(name) {
        return Err(CompileError::new(
            "E117",
            line,
            col,
            format!("函数「{}」不能捕获自己的名字", name),
        )
        .with_hint("要递归请把函数定义在模块顶层，通过全局名字调用"));
    }
    if fs.nlocals >= 255 {
        return Err(CompileError::new(
            "E116",
            line,
            col,
            format!("闭包捕获的变量超过 255 个（卡在「{}」）", name),
        ));
    }
    let slot = fs.nlocals;
    fs.nlocals += 1;
    fs.captures.insert(name.to_string(), slot);
    fs.caps.push((pslot, slot));
    Ok(Res::Capture(slot))
}

fn store_name(fs: &mut FnState, b: &mut Builder, name: &str, line: u32, col: u32) -> Result<()> {
    if native_spec::entry(name).is_some() {
        return Err(CompileError::new(
            "E115",
            line,
            col,
            format!("原生函数名「{}」是保留名，不能声明或赋值", name),
        ));
    }
    match resolve(fs, b, name, line, col)? {
        Res::Local(s) => fs.emit_u8(OP_STORE_LOCAL, s),
        Res::Capture(_) => {
            return Err(CompileError::new(
                "E117",
                line,
                col,
                format!("「{}」是闭包捕获的变量，不能赋值", name),
            )
            .with_hint(
                "闭包按值捕获、只读；要改状态就放模块级「令」，或在本函数里用「令」新建同名局部变量",
            ));
        }
        Res::Global(g) => fs.emit_u8(OP_STORE_GLOBAL, g),
    }
    Ok(())
}

fn load_name(fs: &mut FnState, b: &mut Builder, name: &str, line: u32, col: u32) -> Result<()> {
    match resolve(fs, b, name, line, col)? {
        Res::Local(s) | Res::Capture(s) => fs.emit_u8(OP_LOAD_LOCAL, s),
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

fn note_free(out: &mut Vec<String>, bound: &HashSet<String>, name: &str) {
    if !bound.contains(name) && !out.iter().any(|n| n == name) {
        out.push(name.to_string());
    }
}

fn collect_free_expr(e: &Expr, bound: &HashSet<String>, out: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Name(n) => note_free(out, bound, n),
        ExprKind::List(items) => {
            for it in items {
                collect_free_expr(it, bound, out);
            }
        }
        ExprKind::Dict(pairs) => {
            for (k, v) in pairs {
                collect_free_expr(k, bound, out);
                collect_free_expr(v, bound, out);
            }
        }
        ExprKind::Bin(_, l, r) => {
            collect_free_expr(l, bound, out);
            collect_free_expr(r, bound, out);
        }
        ExprKind::Unary(_, x) => collect_free_expr(x, bound, out),
        ExprKind::Compare(first, pairs) => {
            collect_free_expr(first, bound, out);
            for (_, x) in pairs {
                collect_free_expr(x, bound, out);
            }
        }
        ExprKind::Call(callee, args) => {
            collect_free_expr(callee, bound, out);
            for a in args {
                collect_free_expr(a, bound, out);
            }
        }
        ExprKind::Index(obj, idx) => {
            collect_free_expr(obj, bound, out);
            collect_free_expr(idx, bound, out);
        }
        _ => {}
    }
}

/// 收集函数体里的自由名字（不含本函数参数与声明）。
/// 嵌套函数体内的自由名字也会冒泡到本层，供中间层提前建立捕获。
fn collect_free_stmts(stmts: &[Stmt], bound: &HashSet<String>, out: &mut Vec<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Import(_) | StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Let(_, e) => collect_free_expr(e, bound, out),
            StmtKind::Assign(n, e) => {
                collect_free_expr(e, bound, out);
                note_free(out, bound, n);
            }
            StmtKind::AssignIndex(obj, idx, val) => {
                collect_free_expr(obj, bound, out);
                collect_free_expr(idx, bound, out);
                collect_free_expr(val, bound, out);
            }
            StmtKind::Expr(e) => collect_free_expr(e, bound, out),
            StmtKind::Return(Some(e)) => collect_free_expr(e, bound, out),
            StmtKind::Return(None) => {}
            StmtKind::If {
                branches,
                else_body,
            } => {
                for (c, body) in branches {
                    collect_free_expr(c, bound, out);
                    collect_free_stmts(body, bound, out);
                }
                if let Some(body) = else_body {
                    collect_free_stmts(body, bound, out);
                }
            }
            StmtKind::While(c, body) | StmtKind::Repeat(c, body) => {
                collect_free_expr(c, bound, out);
                collect_free_stmts(body, bound, out);
            }
            StmtKind::ForEach(_, iterable, body) => {
                collect_free_expr(iterable, bound, out);
                collect_free_stmts(body, bound, out);
            }
            StmtKind::FuncDef { params, body, .. } => {
                let mut child_bound: HashSet<String> = params.iter().cloned().collect();
                let mut decls = Vec::new();
                collect_decls(body, &mut decls);
                child_bound.extend(decls);
                collect_free_stmts(body, &child_bound, out);
            }
        }
    }
}

pub fn compile(stmts: &[Stmt]) -> Result<Program> {
    // 先汇总顶层导入：`导入 模块` 是纯编译期声明，不生成字节码，
    // 但模块里的原生名字会进入本文件的名字作用域（供静态检查）。
    let mut imported: HashSet<String> = HashSet::new();
    for s in stmts {
        if let StmtKind::Import(m) = &s.kind {
            let Some(spec) = native_spec::module(m) else {
                return Err(CompileError::new(
                    "E112",
                    s.line,
                    1,
                    format!("不认识的导入模块「{}」", m),
                )
                .with_hint(format!("v0 可用：{}", native_spec::module_names())));
            };
            for e in spec.entries {
                imported.insert(e.name.to_string());
            }
        }
    }

    let mut b = Builder::new();
    let module_name = b.intern_name("<模块>", 1, 1)?;
    b.codes.push(CodeObj {
        name_idx: module_name,
        nparams: 0,
        nlocals: 0,
        code: Vec::new(),
        caps: Vec::new(),
    });

    let mut fs = FnState {
        code: Vec::new(),
        slots: HashMap::new(),
        captures: HashMap::new(),
        parent_slots: HashMap::new(),
        caps: Vec::new(),
        self_name: None,
        nlocals: 0,
        temps: Vec::new(),
        loops: Vec::new(),
        is_module: true,
        imported,
    };

    // 顶层导入已校验；其余语句照常编译。嵌套的「导入」由 compile_stmt 拒绝。
    for s in stmts {
        if matches!(s.kind, StmtKind::Import(_)) {
            continue;
        }
        compile_stmt(&mut fs, &mut b, s)?;
    }
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
    parent_slots: HashMap<String, u8>,
    imported: &HashSet<String>,
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
        caps: Vec::new(),
    });

    let mut fs = FnState {
        code: Vec::new(),
        slots: HashMap::new(),
        captures: HashMap::new(),
        parent_slots,
        caps: Vec::new(),
        self_name: Some(name.to_string()),
        nlocals: params.len() as u8,
        temps: Vec::new(),
        loops: Vec::new(),
        is_module: false,
        imported: imported.clone(),
    };

    for (i, p) in params.iter().enumerate() {
        if native_spec::entry(p).is_some() {
            return Err(CompileError::new(
                "E115",
                line,
                col,
                format!("原生函数名「{}」是保留名，不能当参数", p),
            ));
        }
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
    b.codes[idx as usize].caps = fs.caps;
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
        StmtKind::Import(_) => {
            return Err(CompileError::new(
                "E109",
                s.line,
                1,
                "「导入」只能写在模块顶层",
            ));
        }
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
            // 父帧里可见的名字：本帧局部槽 + 本帧已有的捕获槽。
            let mut visible: HashMap<String, u8> = fs.slots.clone();
            for (n, slot) in &fs.captures {
                visible.insert(n.clone(), *slot);
            }
            // 子函数（含更深层）用到、但本帧还没有的祖先变量，先让本帧捕获，
            // 这样多层闭包就能沿捕获槽一层层传下去。
            let mut child_bound: HashSet<String> = params.iter().cloned().collect();
            let mut decls = Vec::new();
            collect_decls(body, &mut decls);
            child_bound.extend(decls);
            let mut free = Vec::new();
            collect_free_stmts(body, &child_bound, &mut free);
            for n in free {
                if visible.contains_key(&n) {
                    continue;
                }
                if fs.parent_slots.contains_key(&n) {
                    if let Res::Capture(slot) = resolve(fs, b, &n, s.line, 1)? {
                        visible.insert(n, slot);
                    }
                }
            }
            let idx = compile_function(b, name, params, body, visible, &fs.imported, s.line, 1)?;
            if b.codes[idx as usize].caps.is_empty() {
                fs.emit_u8(OP_MAKE_FUNCTION, idx);
            } else {
                fs.emit_u8(OP_MAKE_CLOSURE, idx);
            }
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
        ExprKind::Dec(v) => {
            let c = b.const_dec(*v, e.line, e.col)?;
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
        ExprKind::Name(name) => {
            if let Some((m, _)) = native_spec::entry(name) {
                if fs.imported.contains(name) {
                    return Err(CompileError::new(
                        "E115",
                        e.line,
                        e.col,
                        format!("原生函数「{}」只能直接调用，不能当值使用", name),
                    ));
                }
                return Err(CompileError::new(
                    "E113",
                    e.line,
                    e.col,
                    format!("原生函数「{}」属于模块「{}」，还没有导入", name, m.name),
                )
                .with_hint(format!("先写：导入 {}", m.name)));
            }
            load_name(fs, b, name, e.line, e.col)?;
        }
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
            match &callee.kind {
                ExprKind::Name(name) => {
                    if let Some((m, ent)) = native_spec::entry(name) {
                        if !fs.imported.contains(name) {
                            return Err(CompileError::new(
                                "E113",
                                callee.line,
                                callee.col,
                                format!(
                                    "原生函数「{}」属于模块「{}」，还没有导入",
                                    name, m.name
                                ),
                            )
                            .with_hint(format!("先写：导入 {}", m.name)));
                        }
                        let n = args.len();
                        if n < ent.min_args as usize || n > ent.max_args as usize {
                            let want = if ent.min_args == ent.max_args {
                                format!("{} 个", ent.min_args)
                            } else {
                                format!("{}–{} 个", ent.min_args, ent.max_args)
                            };
                            return Err(CompileError::new(
                                "E115",
                                callee.line,
                                callee.col,
                                format!(
                                    "原生函数「{}」需要 {}参数，这里给了 {} 个",
                                    name, want, n
                                ),
                            ));
                        }
                    } else if let Some((m, ent)) = native_spec::suggest(name, &fs.imported) {
                        return Err(CompileError::new(
                            "E115",
                            callee.line,
                            callee.col,
                            format!("模块「{}」里没有「{}」，是不是「{}」？", m.name, name, ent.name),
                        )
                        .with_hint("原生函数名在编译期校验，拼错不会等到运行时 E004"));
                    }
                    // 原生函数和普通全局函数一样按名字装载，运行期再绑定到 native 表。
                    load_name(fs, b, name, callee.line, callee.col)?;
                }
                _ => compile_expr(fs, b, callee)?,
            }
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

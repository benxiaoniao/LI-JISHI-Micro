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

/// 语句级调试标记：指令流偏移 → 源码行号。
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub off: u32,
    pub line: u32,
}

#[derive(Debug)]
pub struct CodeObj {
    pub name_idx: u8,
    pub nparams: u8,
    pub nlocals: u8,
    pub code: Vec<u8>,
    /// 闭包捕获表：`(父帧槽, 子帧槽)`，按出现顺序排列。
    pub caps: Vec<(u8, u8)>,
    /// 本代码对象所属源文件（调试表 `file_id`）。
    pub file_id: u8,
    /// 语句开始处的（偏移, 行号），按偏移严格递增。
    pub marks: Vec<Mark>,
}

#[derive(Debug)]
pub struct ClassObj {
    pub name_idx: u8,
    pub init_code: u8,
    /// 字段名在名字表中的下标，按声明顺序。
    pub fields: Vec<u8>,
    /// `(方法名下标, 代码对象下标)`。
    pub methods: Vec<(u8, u8)>,
}

#[derive(Debug)]
pub struct Program {
    pub consts: Vec<ConstVal>,
    pub names: Vec<String>,
    pub codes: Vec<CodeObj>,
    pub classes: Vec<ClassObj>,
    pub entry: u8,
    /// 源文件表，下标即 `file_id`；写入调试段。
    pub files: Vec<String>,
}

/// 名字表溢出时按 `包名::` 前缀统计占用（单文件编译没有前缀，算「入口包」）。
/// 打包后多个包共用一张名字表，这条提示能直接指出是谁吃掉了名额。
fn name_owner_hint(names: &[String]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for n in names {
        let owner = match n.split_once("::") {
            Some((p, _)) => p,
            None => "<入口包>",
        };
        if let Some(e) = counts.iter_mut().find(|(k, _)| k == owner) {
            e.1 += 1;
        } else {
            counts.push((owner.to_string(), 1));
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top = counts
        .iter()
        .take(3)
        .map(|(k, v)| format!("{} {}", k, v))
        .collect::<Vec<_>>()
        .join("，");
    format!("已用满 255 个，占用最多的是：{}", top)
}

struct Builder {
    consts: Vec<ConstVal>,
    names: Vec<String>,
    name_map: HashMap<String, u8>,
    codes: Vec<CodeObj>,
    classes: Vec<ClassObj>,
    /// 类名 → 类表下标（`新建` 用）。
    class_map: HashMap<String, u8>,
}

impl Builder {
    fn new() -> Self {
        Self {
            consts: Vec::new(),
            names: Vec::new(),
            name_map: HashMap::new(),
            codes: Vec::new(),
            classes: Vec::new(),
            class_map: HashMap::new(),
        }
    }

    fn intern_name(&mut self, name: &str, line: u32, col: u32) -> Result<u8> {
        if let Some(&i) = self.name_map.get(name) {
            return Ok(i);
        }
        let i = self.names.len();
        if i >= 255 {
            return Err(CompileError::new(
                "E124",
                line,
                col,
                format!(
                    "名字表超过 255 个（Micro Core v0 的上限）；{}",
                    name_owner_hint(&self.names)
                ),
            ));
        }
        self.names.push(name.to_string());
        self.name_map.insert(name.to_string(), i as u8);
        Ok(i as u8)
    }

    /// 在类表里找字段名下标。
    fn class_field_idx(&self, class_idx: u8, name: &str) -> Option<u8> {
        let c = self.classes.get(class_idx as usize)?;
        c.fields
            .iter()
            .copied()
            .find(|&ni| self.names.get(ni as usize).map(|s| s == name).unwrap_or(false))
    }

    /// 在类表里找方法名下标。
    fn class_method_idx(&self, class_idx: u8, name: &str) -> Option<u8> {
        let c = self.classes.get(class_idx as usize)?;
        c.methods
            .iter()
            .copied()
            .find(|&(ni, _)| self.names.get(ni as usize).map(|s| s == name).unwrap_or(false))
            .map(|(ni, _)| ni)
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

/// 正在编译的 `尝试` 块：记录进入时的循环层数，
/// 用于在 `返回` / `中断` / `继续` 离开时补发 `TRY_END`。
struct TryCtx {
    loop_depth: usize,
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
    /// 当前语句位置上仍生效的 `尝试` 块（由内向外）。
    try_stack: Vec<TryCtx>,
    is_module: bool,
    /// 顶层 `导入 模块` 汇总出来的原生函数名。
    imported: HashSet<String>,
    /// 当前编译体所属的类（类表下标）；方法内及方法里的闭包会带上。
    current_class: Option<u8>,
    /// 编译字段默认值期间为 true：禁止引用 `本`（E120）。
    self_blocked: bool,
    /// 当前帧是 `初始化`：禁止显式 `返回`（E120）。
    in_init: bool,
    /// 当前语句所属的源文件（调试表 `file_id`）。
    file_id: u8,
    /// 语句级调试标记（偏移 → 行号）。
    marks: Vec<Mark>,
}

impl FnState {
    fn emit(&mut self, op: u8) {
        self.code.push(op);
    }
    fn emit_u8(&mut self, op: u8, a: u8) {
        self.code.push(op);
        self.code.push(a);
    }
    fn emit_u8u8(&mut self, op: u8, a: u8, b: u8) {
        self.code.push(op);
        self.code.push(a);
        self.code.push(b);
    }
    fn here(&self) -> usize {
        self.code.len()
    }
    /// 记一条语句级调试标记。同一偏移只保留最后一次（前面的语句没有生成指令）；
    /// 同一行连续出现时只留最早一条，减小调试段。
    fn mark(&mut self, line: u32) {
        let off = self.code.len() as u32;
        if let Some(last) = self.marks.last_mut() {
            if last.off == off {
                last.line = line;
                return;
            }
            if last.line == line {
                return;
            }
        }
        self.marks.push(Mark { off, line });
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

/// 装载 `本`：方法帧槽 0，或方法内闭包对槽 0 的捕获。
fn load_self(fs: &mut FnState, line: u32, col: u32) -> Result<()> {
    if fs.self_blocked {
        return Err(CompileError::new(
            "E120",
            line,
            col,
            "字段默认值里不能引用「本」",
        ));
    }
    if let Some(&slot) = fs.slots.get("本") {
        fs.emit_u8(OP_LOAD_LOCAL, slot);
        return Ok(());
    }
    if let Some(&slot) = fs.captures.get("本") {
        fs.emit_u8(OP_LOAD_LOCAL, slot);
        return Ok(());
    }
    if let Some(pslot) = fs.parent_slots.get("本").copied() {
        if let Res::Capture(slot) = capture_from_parent(fs, "本", pslot, line, col)? {
            fs.emit_u8(OP_LOAD_LOCAL, slot);
            return Ok(());
        }
    }
    Err(CompileError::new(
        "E120",
        line,
        col,
        "「本」只能在类方法（及其内部闭包）里使用",
    ))
}

/// 取字段名的名字表下标；接收者是 `本` 时顺带做静态检查（E122/E123）。
fn field_name_idx(
    fs: &FnState,
    b: &mut Builder,
    receiver: &Expr,
    name: &str,
    line: u32,
    col: u32,
) -> Result<u8> {
    if matches!(receiver.kind, ExprKind::SelfRef) {
        if let Some(ci) = fs.current_class {
            if b.class_method_idx(ci, name).is_some() {
                return Err(CompileError::new(
                    "E123",
                    line,
                    col,
                    format!("「{}」是方法，不能当值使用（要调用请写 本.{}(...)）", name, name),
                ));
            }
            if let Some(ni) = b.class_field_idx(ci, name) {
                return Ok(ni);
            }
            return Err(CompileError::new(
                "E122",
                line,
                col,
                format!("类里没有字段「{}」", name),
            ));
        }
    }
    b.intern_name(name, line, col)
}

/// 取方法名的名字表下标；接收者是 `本` 时静态检查成员是否存在。
fn method_name_idx(
    fs: &FnState,
    b: &mut Builder,
    receiver: &Expr,
    name: &str,
    line: u32,
    col: u32,
) -> Result<u8> {
    if matches!(receiver.kind, ExprKind::SelfRef) {
        if let Some(ci) = fs.current_class {
            if let Some(ni) = b.class_method_idx(ci, name) {
                return Ok(ni);
            }
            if b.class_field_idx(ci, name).is_some() {
                return Err(CompileError::new(
                    "E123",
                    line,
                    col,
                    format!("「{}」是字段，不是方法，不能调用", name),
                ));
            }
            return Err(CompileError::new(
                "E122",
                line,
                col,
                format!("类里没有方法「{}」", name),
            ));
        }
    }
    b.intern_name(name, line, col)
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
            StmtKind::Try {
                body,
                catch_name,
                catch_body,
            } => {
                if let Some(n) = catch_name {
                    out.push(n.clone());
                }
                collect_decls(body, out);
                collect_decls(catch_body, out);
            }
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
        ExprKind::SelfRef => note_free(out, bound, "本"),
        ExprKind::New(_, args) => {
            for a in args {
                collect_free_expr(a, bound, out);
            }
        }
        ExprKind::Member(obj, _) => collect_free_expr(obj, bound, out),
        ExprKind::MethodCall(obj, _, args) => {
            collect_free_expr(obj, bound, out);
            for a in args {
                collect_free_expr(a, bound, out);
            }
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
            StmtKind::AssignMember(obj, _, val) => {
                collect_free_expr(obj, bound, out);
                collect_free_expr(val, bound, out);
            }
            StmtKind::Class(_) => {}
            StmtKind::Expr(e) => collect_free_expr(e, bound, out),
            StmtKind::Return(Some(e)) => collect_free_expr(e, bound, out),
            StmtKind::Return(None) => {}
            StmtKind::Throw(e) => collect_free_expr(e, bound, out),
            StmtKind::Try {
                body,
                catch_name,
                catch_body,
            } => {
                collect_free_stmts(body, bound, out);
                let mut cbound = bound.clone();
                if let Some(n) = catch_name {
                    cbound.insert(n.clone());
                }
                collect_free_stmts(catch_body, &cbound, out);
            }
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

/// 顶层语句下标 → 源文件（`spans` 是合并时记录的 `[起, 止, file_id]`）。
fn file_at(spans: &[(usize, usize, u8)], i: usize) -> u8 {
    spans
        .iter()
        .find(|(a, b, _)| i >= *a && i < *b)
        .map(|(_, _, f)| *f)
        .unwrap_or(0)
}

pub fn compile(
    stmts: &[Stmt],
    files: Vec<String>,
    spans: &[(usize, usize, u8)],
) -> Result<Program> {
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
        file_id: 0,
        marks: Vec::new(),
    });

    // 类先于模块代码编译：类表要在 `新建` 之前完整，方法体也独立成代码对象。
    compile_classes(&mut b, stmts, &imported, spans)?;

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
        try_stack: Vec::new(),
        is_module: true,
        imported,
        current_class: None,
        self_blocked: false,
        in_init: false,
        file_id: spans.last().map(|s| s.2).unwrap_or(0),
        marks: Vec::new(),
    };
    fs.mark(stmts.first().map(|s| s.line).unwrap_or(1));

    // 顶层导入已校验；其余语句照常编译。嵌套的「导入」由 compile_stmt 拒绝。
    for (si, s) in stmts.iter().enumerate() {
        if matches!(s.kind, StmtKind::Import(_)) {
            continue;
        }
        fs.file_id = file_at(spans, si);
        compile_stmt(&mut fs, &mut b, s)?;
    }
    fs.emit(OP_HALT);

    b.codes[0].nlocals = fs.nlocals;
    b.codes[0].file_id = fs.file_id;
    b.codes[0].marks = fs.marks;
    b.codes[0].code = fs.code;

    Ok(Program {
        consts: b.consts,
        names: b.names,
        codes: b.codes,
        classes: b.classes,
        entry: 0,
        files,
    })
}

fn compile_classes(
    b: &mut Builder,
    stmts: &[Stmt],
    imported: &HashSet<String>,
    spans: &[(usize, usize, u8)],
) -> Result<()> {
    // 顶层名字（类名不能与它们重名）。
    let mut top = HashSet::new();
    for s in stmts {
        match &s.kind {
            StmtKind::Let(n, _) | StmtKind::ForEach(n, ..) => {
                top.insert(n.clone());
            }
            StmtKind::FuncDef { name, .. } => {
                top.insert(name.clone());
            }
            _ => {}
        }
    }

    // 第一遍：分配类下标（类名全局唯一）。
    let mut decls: Vec<(&ClassDecl, u8)> = Vec::new();
    for (si, s) in stmts.iter().enumerate() {
        if let StmtKind::Class(c) = &s.kind {
            if top.contains(&c.name) {
                return Err(CompileError::new(
                    "E119",
                    s.line,
                    1,
                    format!("类名「{}」与顶层函数或变量重名", c.name),
                ));
            }
            if b.class_map.contains_key(&c.name) {
                return Err(CompileError::new(
                    "E119",
                    s.line,
                    1,
                    format!("类「{}」重复定义", c.name),
                ));
            }
            if b.classes.len() >= 255 {
                return Err(CompileError::new("E105", s.line, 1, "类超过 255 个"));
            }
            b.class_map.insert(c.name.clone(), b.classes.len() as u8);
            b.classes.push(ClassObj {
                name_idx: 0,
                init_code: 0,
                fields: Vec::new(),
                methods: Vec::new(),
            });
            decls.push((c, file_at(spans, si)));
        }
    }

    // 第二遍：类名 / 字段名 / 方法名进全局名字表，并做类内唯一性检查。
    for (ci, (c, _)) in decls.iter().enumerate() {
        let name_idx = b.intern_name(&c.name, 1, 1)?;
        b.classes[ci].name_idx = name_idx;
        if c.fields.len() > 255 || c.methods.len() > 255 {
            return Err(CompileError::new("E105", 1, 1, "类的字段或方法超过 255 个"));
        }
        let mut members: HashSet<String> = HashSet::new();
        for (fname, fexpr) in &c.fields {
            if fname == "初始化" {
                return Err(CompileError::new(
                    "E119",
                    fexpr.line,
                    fexpr.col,
                    "「初始化」是构造方法名，不能用作字段名",
                ));
            }
            if !members.insert(fname.clone()) {
                return Err(CompileError::new(
                    "E119",
                    fexpr.line,
                    fexpr.col,
                    format!("类「{}」里重复的成员名「{}」", c.name, fname),
                ));
            }
        }
        for m in &c.methods {
            if !members.insert(m.name.clone()) {
                return Err(CompileError::new(
                    "E119",
                    m.line,
                    1,
                    format!("类「{}」里重复的成员名「{}」", c.name, m.name),
                ));
            }
        }
        let mut fields = Vec::new();
        for (fname, _) in &c.fields {
            fields.push(b.intern_name(fname, 1, 1)?);
        }
        b.classes[ci].fields = fields;
    }

    // 第三遍：编译方法体与合成的初始化代码对象。
    for (ci, (c, fid)) in decls.iter().enumerate() {
        let ci = ci as u8;
        for m in &c.methods {
            if m.name == "初始化" {
                continue; // 构造方法只由合成的初始化代码真正执行
            }
            let midx = b.intern_name(&m.name, m.line, 1)?;
            // 先占位登记名字，后面的方法才能静态看到前面的方法（E122 检查要用）。
            let slot = b.classes[ci as usize].methods.len();
            b.classes[ci as usize].methods.push((midx, 0));
            let code = compile_method(b, ci, midx, &m.params, &m.body, imported, m.line, *fid)?;
            b.classes[ci as usize].methods[slot].1 = code;
        }
        let init_code = compile_class_init(b, c, ci, imported, *fid)?;
        b.classes[ci as usize].init_code = init_code;
    }
    Ok(())
}

/// 方法帧：槽 0 固定是 `本`，声明形参从槽 1 开始。
fn compile_method(
    b: &mut Builder,
    class_idx: u8,
    name_idx: u8,
    params: &[String],
    body: &[Stmt],
    imported: &HashSet<String>,
    line: u32,
    file_id: u8,
) -> Result<u8> {
    let idx = b.codes.len();
    if idx >= 255 {
        return Err(CompileError::new("E106", line, 1, "函数超过 255 个"));
    }
    let idx = idx as u8;
    b.codes.push(CodeObj {
        name_idx,
        nparams: params.len() as u8 + 1,
        nlocals: params.len() as u8 + 1,
        code: Vec::new(),
        caps: Vec::new(),
        file_id,
        marks: Vec::new(),
    });

    let mut fs = FnState {
        code: Vec::new(),
        slots: HashMap::new(),
        captures: HashMap::new(),
        parent_slots: HashMap::new(),
        caps: Vec::new(),
        self_name: None,
        nlocals: params.len() as u8 + 1,
        temps: Vec::new(),
        loops: Vec::new(),
        try_stack: Vec::new(),
        is_module: false,
        imported: imported.clone(),
        current_class: Some(class_idx),
        self_blocked: false,
        in_init: false,
        file_id,
        marks: Vec::new(),
    };
    fs.mark(line);
    fs.slots.insert("本".to_string(), 0);

    for (i, p) in params.iter().enumerate() {
        if native_spec::entry(p).is_some() {
            return Err(CompileError::new(
                "E115",
                line,
                1,
                format!("原生函数名「{}」是保留名，不能当参数", p),
            ));
        }
        if fs.slots.insert(p.clone(), (i + 1) as u8).is_some() {
            return Err(CompileError::new("E109", line, 1, "参数重名"));
        }
    }
    let mut decls = Vec::new();
    collect_decls(body, &mut decls);
    for d in decls {
        if fs.slots.contains_key(&d) {
            continue;
        }
        if fs.nlocals >= 255 {
            return Err(CompileError::new("E104", line, 1, "局部变量超过 255 个"));
        }
        fs.slots.insert(d, fs.nlocals);
        fs.nlocals += 1;
    }

    compile_stmts(&mut fs, b, body)?;

    let nil = b.const_nil(line, 1)?;
    fs.emit_u8(OP_CONST, nil);
    fs.emit(OP_RETURN_METHOD);

    b.codes[idx as usize].nlocals = fs.nlocals;
    b.codes[idx as usize].caps = fs.caps;
    b.codes[idx as usize].marks = fs.marks;
    b.codes[idx as usize].code = fs.code;
    Ok(idx)
}

/// 合成初始化代码：字段默认值 → 用户 `初始化` 体 → 隐式 `返回 本`。
fn compile_class_init(
    b: &mut Builder,
    c: &ClassDecl,
    class_idx: u8,
    imported: &HashSet<String>,
    file_id: u8,
) -> Result<u8> {
    let user = c.methods.iter().find(|m| m.name == "初始化");
    let params: Vec<String> = user.map(|m| m.params.clone()).unwrap_or_default();

    let idx = b.codes.len();
    if idx >= 255 {
        return Err(CompileError::new("E106", 1, 1, "函数超过 255 个"));
    }
    let idx = idx as u8;
    let name_idx = b.classes[class_idx as usize].name_idx;
    b.codes.push(CodeObj {
        name_idx,
        nparams: params.len() as u8 + 1,
        nlocals: params.len() as u8 + 1,
        code: Vec::new(),
        caps: Vec::new(),
        file_id,
        marks: Vec::new(),
    });

    let mut fs = FnState {
        code: Vec::new(),
        slots: HashMap::new(),
        captures: HashMap::new(),
        parent_slots: HashMap::new(),
        caps: Vec::new(),
        self_name: None,
        nlocals: params.len() as u8 + 1,
        temps: Vec::new(),
        loops: Vec::new(),
        try_stack: Vec::new(),
        is_module: false,
        imported: imported.clone(),
        current_class: Some(class_idx),
        self_blocked: false,
        in_init: true,
        file_id,
        marks: Vec::new(),
    };
    fs.mark(user.map(|m| m.line).unwrap_or(1));
    fs.slots.insert("本".to_string(), 0);

    for (i, p) in params.iter().enumerate() {
        if native_spec::entry(p).is_some() {
            return Err(CompileError::new(
                "E115",
                user.map(|m| m.line).unwrap_or(1),
                1,
                format!("原生函数名「{}」是保留名，不能当参数", p),
            ));
        }
        if fs.slots.insert(p.clone(), (i + 1) as u8).is_some() {
            return Err(CompileError::new(
                "E109",
                user.map(|m| m.line).unwrap_or(1),
                1,
                "参数重名",
            ));
        }
    }
    if let Some(u) = user {
        let mut decls = Vec::new();
        collect_decls(&u.body, &mut decls);
        for d in decls {
            if fs.slots.contains_key(&d) {
                continue;
            }
            if fs.nlocals >= 255 {
                return Err(CompileError::new("E104", u.line, 1, "局部变量超过 255 个"));
            }
            fs.slots.insert(d, fs.nlocals);
            fs.nlocals += 1;
        }
    }

    // 字段默认值：按声明顺序求值，禁止引用 `本`。
    for (i, (_, fexpr)) in c.fields.iter().enumerate() {
        fs.emit_u8(OP_LOAD_LOCAL, 0);
        fs.self_blocked = true;
        compile_expr(&mut fs, b, fexpr)?;
        fs.self_blocked = false;
        let fidx = b.classes[class_idx as usize].fields[i];
        fs.emit_u8(OP_SET_FIELD, fidx);
    }

    if let Some(u) = user {
        compile_stmts(&mut fs, b, &u.body)?;
    }

    fs.emit_u8(OP_LOAD_LOCAL, 0);
    fs.emit(OP_RETURN_METHOD);

    b.codes[idx as usize].nlocals = fs.nlocals;
    b.codes[idx as usize].caps = fs.caps;
    b.codes[idx as usize].marks = fs.marks;
    b.codes[idx as usize].code = fs.code;
    Ok(idx)
}

#[allow(clippy::too_many_arguments)]
fn compile_function(
    b: &mut Builder,
    name: &str,
    params: &[String],
    body: &[Stmt],
    parent_slots: HashMap<String, u8>,
    imported: &HashSet<String>,
    current_class: Option<u8>,
    line: u32,
    col: u32,
    file_id: u8,
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
        file_id,
        marks: Vec::new(),
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
        try_stack: Vec::new(),
        is_module: false,
        imported: imported.clone(),
        current_class,
        self_blocked: false,
        in_init: false,
        file_id,
        marks: Vec::new(),
    };
    fs.mark(line);

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
    b.codes[idx as usize].marks = fs.marks;
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
    fs.mark(s.line);
    match &s.kind {
        StmtKind::Import(_) => {
            return Err(CompileError::new(
                "E109",
                s.line,
                1,
                "「导入」只能写在模块顶层",
            ));
        }
        StmtKind::Class(_) => {
            if !fs.is_module {
                return Err(CompileError::new(
                    "E118",
                    s.line,
                    1,
                    "「类」只能写在模块顶层",
                ));
            }
            // 顶层类已由 compile_classes 预编译，这里不生成模块字节码。
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
        StmtKind::AssignMember(obj, name, val) => {
            let ni = field_name_idx(fs, b, obj, name, s.line, obj.col)?;
            compile_expr(fs, b, obj)?;
            compile_expr(fs, b, val)?;
            fs.emit_u8(OP_SET_FIELD, ni);
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
            // 只退出位于本循环体内的 `尝试` 块；包住整个循环的不能弹。
            let loop_depth = fs.loops.len();
            let exits = fs
                .try_stack
                .iter()
                .filter(|c| c.loop_depth >= loop_depth)
                .count();
            for _ in 0..exits {
                fs.emit(OP_TRY_END);
            }
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
            let loop_depth = fs.loops.len();
            let exits = fs
                .try_stack
                .iter()
                .filter(|c| c.loop_depth >= loop_depth)
                .count();
            for _ in 0..exits {
                fs.emit(OP_TRY_END);
            }
            let j = fs.jump(OP_JUMP);
            fs.patch(j, target);
        }
        StmtKind::Throw(e) => {
            compile_expr(fs, b, e)?;
            fs.emit(OP_THROW);
        }
        StmtKind::Try {
            body,
            catch_name,
            catch_body,
        } => {
            let tryjmp = fs.jump(OP_TRY);
            fs.try_stack.push(TryCtx {
                loop_depth: fs.loops.len(),
            });
            compile_stmts(fs, b, body)?;
            fs.try_stack.pop();
            fs.emit(OP_TRY_END);
            // 正常路径跳过捕获块
            let skip = fs.jump(OP_JUMP);
            let handler = fs.here();
            fs.patch(tryjmp, handler);
            match catch_name {
                Some(n) => store_name(fs, b, n, s.line, 1)?,
                None => fs.emit(OP_POP),
            }
            compile_stmts(fs, b, catch_body)?;
            let end = fs.here();
            fs.patch(skip, end);
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
            let idx = compile_function(
                b,
                name,
                params,
                body,
                visible,
                &fs.imported,
                fs.current_class,
                s.line,
                1,
                fs.file_id,
            )?;
            if b.codes[idx as usize].caps.is_empty() {
                fs.emit_u8(OP_MAKE_FUNCTION, idx);
            } else {
                fs.emit_u8(OP_MAKE_CLOSURE, idx);
            }
            store_name(fs, b, name, s.line, 1)?;
        }
        StmtKind::Return(e) => {
            if fs.in_init {
                return Err(CompileError::new(
                    "E120",
                    s.line,
                    e.as_ref().map(|x| x.col).unwrap_or(1),
                    "「初始化」里不能写「返回」；构造结果固定为「本」",
                ));
            }
            for _ in 0..fs.try_stack.len() {
                fs.emit(OP_TRY_END);
            }
            match e {
                Some(e) => compile_expr(fs, b, e)?,
                None => {
                    let nil = b.const_nil(s.line, 1)?;
                    fs.emit_u8(OP_CONST, nil);
                }
            }
            // 方法帧的槽 0 是接收者，必须用 RETURN_METHOD 收尾。
            if fs.slots.contains_key("本") {
                fs.emit(OP_RETURN_METHOD);
            } else {
                fs.emit(OP_RETURN);
            }
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
        ExprKind::SelfRef => {
            load_self(fs, e.line, e.col)?;
        }
        ExprKind::New(cls, args) => {
            let Some(&ci) = b.class_map.get(cls) else {
                return Err(CompileError::new(
                    "E121",
                    e.line,
                    e.col,
                    format!("类「{}」没有定义", cls),
                ));
            };
            fs.emit_u8(OP_NEW_INSTANCE, ci);
            for a in args {
                compile_expr(fs, b, a)?;
            }
            if args.len() > 255 {
                return Err(CompileError::new("E114", e.line, e.col, "实参超过 255 个"));
            }
            fs.emit_u8u8(OP_CALL_INIT, ci, args.len() as u8);
        }
        ExprKind::Member(obj, name) => {
            let ni = field_name_idx(fs, b, obj, name, e.line, e.col)?;
            compile_expr(fs, b, obj)?;
            fs.emit_u8(OP_GET_FIELD, ni);
        }
        ExprKind::MethodCall(obj, name, args) => {
            let ni = method_name_idx(fs, b, obj, name, e.line, e.col)?;
            compile_expr(fs, b, obj)?;
            for a in args {
                compile_expr(fs, b, a)?;
            }
            if args.len() > 255 {
                return Err(CompileError::new("E114", e.line, e.col, "实参超过 255 个"));
            }
            fs.emit_u8u8(OP_CALL_METHOD, ni, args.len() as u8);
        }
        ExprKind::Index(obj, idx) => {
            compile_expr(fs, b, obj)?;
            compile_expr(fs, b, idx)?;
            fs.emit(OP_GET_ITEM);
        }
    }
    Ok(())
}

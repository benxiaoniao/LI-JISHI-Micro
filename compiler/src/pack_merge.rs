//! 源码级合并：多包 AST → 单个 Program。
//!
//! 做法：
//! - 依赖包的顶层名字统一改写成 `包名::名字`，包内引用同步改写；
//! - 跨包引用写成 `包名.名字`，只有 `[导出]` 里的名字可见；
//! - 合并顺序是「依赖在前、根包在后」，模块级语句按这个顺序执行。

use crate::ast::*;
use crate::error::{CompileError, Result};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct DepView {
    pub exports: Option<HashSet<String>>,
    pub classes: HashSet<String>,
}

pub struct Unit {
    pub name: String,
    pub is_root: bool,
    pub stmts: Vec<Stmt>,
    pub globals: HashSet<String>,
    pub classes: HashSet<String>,
    pub exports: Option<HashSet<String>>,
    /// 入口源文件（调试表的显示名）。
    pub file: String,
}

impl Unit {
    pub fn new(
        name: String,
        is_root: bool,
        stmts: Vec<Stmt>,
        exports: Option<Vec<String>>,
        file: String,
    ) -> Unit {
        let mut globals = HashSet::new();
        let mut classes = HashSet::new();
        collect_top(&stmts, &mut globals, &mut classes);
        Unit {
            name,
            is_root,
            stmts,
            globals,
            classes,
            exports: exports.map(|v| v.into_iter().collect()),
            file,
        }
    }

    pub fn view(&self) -> DepView {
        DepView {
            exports: self.exports.clone(),
            classes: self.classes.clone(),
        }
    }
}

fn collect_top(stmts: &[Stmt], globals: &mut HashSet<String>, classes: &mut HashSet<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Let(n, _) | StmtKind::FuncDef { name: n, .. } => {
                globals.insert(n.clone());
            }
            StmtKind::Class(c) => {
                globals.insert(c.name.clone());
                classes.insert(c.name.clone());
            }
            StmtKind::ForEach(n, _, _) => {
                globals.insert(n.clone());
            }
            _ => {}
        }
    }
}

/// 依赖包的顶层只允许声明：`导入` / `令` / `函数` / `类`。
pub fn check_dependency_top_level(unit: &Unit) -> Result<()> {
    for s in &unit.stmts {
        let ok = matches!(
            s.kind,
            StmtKind::Import(_)
                | StmtKind::Let(_, _)
                | StmtKind::FuncDef { .. }
                | StmtKind::Class(_)
        );
        if !ok {
            return Err(CompileError::new(
                "E127",
                s.line,
                1,
                format!(
                    "依赖包「{}」的顶层只允许声明（导入/令/函数/类），不能出现{}；\
                     可执行的初始化请写进根包，或放进导出函数里",
                    unit.name,
                    stmt_label(&s.kind)
                ),
            ));
        }
    }
    Ok(())
}

fn stmt_label(k: &StmtKind) -> &'static str {
    match k {
        StmtKind::Import(_) => "导入",
        StmtKind::Let(_, _) => "令",
        StmtKind::Assign(_, _) => "赋值",
        StmtKind::AssignIndex(_, _, _) | StmtKind::AssignMember(_, _, _) => "赋值",
        StmtKind::Expr(_) => "表达式语句",
        StmtKind::If { .. } => "如果",
        StmtKind::While(_, _) => "当",
        StmtKind::Repeat(_, _) => "循环",
        StmtKind::ForEach(_, _, _) => "遍历",
        StmtKind::Break => "中断",
        StmtKind::Continue => "继续",
        StmtKind::FuncDef { .. } => "函数",
        StmtKind::Class(_) => "类",
        StmtKind::Return(_) => "返回",
        StmtKind::Throw(_) => "抛出",
        StmtKind::Try { .. } => "尝试",
    }
}

struct Renamer<'a> {
    pkg: &'a str,
    is_root: bool,
    globals: &'a HashSet<String>,
    classes: &'a HashSet<String>,
    deps: &'a HashMap<String, DepView>,
    scopes: Vec<HashSet<String>>,
}

impl<'a> Renamer<'a> {
    fn mangle(&self, n: &str) -> String {
        format!("{}::{}", self.pkg, n)
    }

    fn shadowed(&self, n: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(n))
    }

    fn is_dep_pkg(&self, n: &str) -> bool {
        self.deps.contains_key(n) && !self.shadowed(n) && !self.globals.contains(n)
    }

    fn global_ref(&self, n: &str) -> Option<String> {
        if !self.is_root && !self.shadowed(n) && self.globals.contains(n) {
            Some(self.mangle(n))
        } else {
            None
        }
    }

    fn check_export(&self, pkg: &str, name: &str, line: u32) -> Result<()> {
        let view = &self.deps[pkg];
        if view.classes.contains(name) {
            return Err(CompileError::new(
                "E128",
                line,
                1,
                format!(
                    "类「{}.{}」不能跨包当值用；跨包 `新建` 不在本切片范围内",
                    pkg, name
                ),
            ));
        }
        if let Some(exports) = &view.exports {
            if !exports.contains(name) {
                return Err(CompileError::new(
                    "E128",
                    line,
                    1,
                    format!(
                        "包「{}」没有导出「{}」；请在它的 砾包.toml [导出] 里列出这个名字",
                        pkg, name
                    ),
                ));
            }
        }
        Ok(())
    }

    fn stmts(&mut self, list: &mut [Stmt], module: bool) -> Result<()> {
        for s in list {
            self.stmt(s, module)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: &mut Stmt, module: bool) -> Result<()> {
        match &mut s.kind {
            StmtKind::Import(_) | StmtKind::Break | StmtKind::Continue => Ok(()),
            StmtKind::Let(n, e) => {
                self.expr(e)?;
                if module {
                    if let Some(m) = self.global_ref(n) {
                        *n = m;
                    }
                }
                Ok(())
            }
            StmtKind::Assign(n, e) => {
                self.expr(e)?;
                if let Some(m) = self.global_ref(n) {
                    *n = m;
                }
                Ok(())
            }
            StmtKind::AssignIndex(o, i, v) => {
                self.expr(o)?;
                self.expr(i)?;
                self.expr(v)
            }
            StmtKind::AssignMember(o, _f, v) => {
                self.expr(o)?;
                self.expr(v)
            }
            StmtKind::Expr(e) => self.expr(e),
            StmtKind::If {
                branches,
                else_body,
            } => {
                for (cond, body) in branches {
                    self.expr(cond)?;
                    self.stmts(body, false)?;
                }
                if let Some(b) = else_body {
                    self.stmts(b, false)?;
                }
                Ok(())
            }
            StmtKind::While(c, b) => {
                self.expr(c)?;
                self.stmts(b, false)
            }
            StmtKind::Repeat(c, b) => {
                self.expr(c)?;
                self.stmts(b, false)
            }
            StmtKind::ForEach(n, e, b) => {
                self.expr(e)?;
                if module {
                    if let Some(m) = self.global_ref(n) {
                        *n = m;
                    }
                }
                self.stmts(b, false)
            }
            StmtKind::FuncDef { name, params, body } => {
                if module {
                    if let Some(m) = self.global_ref(name) {
                        *name = m;
                    }
                }
                let mut scope = HashSet::new();
                for d in local_decls(body) {
                    scope.insert(d);
                }
                for p in params.iter() {
                    scope.insert(p.clone());
                }
                self.scopes.push(scope);
                let r = self.stmts(body, false);
                self.scopes.pop();
                r
            }
            StmtKind::Class(c) => {
                if module {
                    if let Some(m) = self.global_ref(&c.name) {
                        c.name = m;
                    }
                }
                for (_, e) in c.fields.iter_mut() {
                    self.expr(e)?;
                }
                for m in c.methods.iter_mut() {
                    let mut scope = HashSet::new();
                    for d in local_decls(&m.body) {
                        scope.insert(d);
                    }
                    for p in m.params.iter() {
                        scope.insert(p.clone());
                    }
                    self.scopes.push(scope);
                    let r = self.stmts(&mut m.body, false);
                    self.scopes.pop();
                    r?;
                }
                Ok(())
            }
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(e)?;
                }
                Ok(())
            }
            StmtKind::Throw(e) => self.expr(e),
            StmtKind::Try {
                body,
                catch_name: _,
                catch_body,
            } => {
                self.stmts(body, false)?;
                self.stmts(catch_body, false)
            }
        }
    }

    fn expr(&mut self, e: &mut Expr) -> Result<()> {
        // `包名.名字` / `包名.函数(...)`：跨包引用。
        if let ExprKind::Member(base, field) = &e.kind {
            if let ExprKind::Name(p) = &base.kind {
                if self.is_dep_pkg(p) {
                    let p = p.clone();
                    let field = field.clone();
                    self.check_export(&p, &field, e.line)?;
                    e.kind = ExprKind::Name(format!("{}::{}", p, field));
                    return Ok(());
                }
            }
        }
        if let ExprKind::MethodCall(base, name, args) = &mut e.kind {
            if let ExprKind::Name(p) = &base.kind {
                if self.is_dep_pkg(p) {
                    let p = p.clone();
                    let name = name.clone();
                    self.check_export(&p, &name, e.line)?;
                    let args = std::mem::take(args);
                    let line = e.line;
                    let col = e.col;
                    e.kind = ExprKind::Call(
                        Box::new(Expr {
                            kind: ExprKind::Name(format!("{}::{}", p, name)),
                            line,
                            col,
                        }),
                        args,
                    );
                    return Ok(());
                }
            }
        }

        match &mut e.kind {
            ExprKind::Name(n) => {
                if let Some(m) = self.global_ref(n) {
                    *n = m;
                }
            }
            ExprKind::New(cls, args) => {
                if !self.is_root && self.classes.contains(cls.as_str()) {
                    *cls = self.mangle(cls);
                }
                for a in args {
                    self.expr(a)?;
                }
            }
            ExprKind::List(items) => {
                for it in items {
                    self.expr(it)?;
                }
            }
            ExprKind::Dict(pairs) => {
                for (k, v) in pairs {
                    self.expr(k)?;
                    self.expr(v)?;
                }
            }
            ExprKind::Bin(_, a, b) => {
                self.expr(a)?;
                self.expr(b)?;
            }
            ExprKind::Unary(_, a) => self.expr(a)?,
            ExprKind::Compare(a, rest) => {
                self.expr(a)?;
                for (_, b) in rest {
                    self.expr(b)?;
                }
            }
            ExprKind::Call(f, args) => {
                self.expr(f)?;
                for a in args {
                    self.expr(a)?;
                }
            }
            ExprKind::Index(a, b) => {
                self.expr(a)?;
                self.expr(b)?;
            }
            ExprKind::Member(base, _f) => self.expr(base)?,
            ExprKind::MethodCall(base, _n, args) => {
                self.expr(base)?;
                for a in args {
                    self.expr(a)?;
                }
            }
            ExprKind::Int(_)
            | ExprKind::Dec(_)
            | ExprKind::Str(_)
            | ExprKind::Nil
            | ExprKind::True
            | ExprKind::False
            | ExprKind::SelfRef => {}
        }
        Ok(())
    }
}

fn local_decls(stmts: &[Stmt]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(stmts: &[Stmt], out: &mut Vec<String>) {
        for s in stmts {
            match &s.kind {
                StmtKind::Let(n, _) => out.push(n.clone()),
                StmtKind::ForEach(n, _, b) => {
                    out.push(n.clone());
                    walk(b, out);
                }
                StmtKind::FuncDef { name, .. } => out.push(name.clone()),
                StmtKind::If {
                    branches,
                    else_body,
                } => {
                    for (_, b) in branches {
                        walk(b, out);
                    }
                    if let Some(b) = else_body {
                        walk(b, out);
                    }
                }
                StmtKind::While(_, b) | StmtKind::Repeat(_, b) => walk(b, out),
                StmtKind::Try {
                    body,
                    catch_name,
                    catch_body,
                } => {
                    if let Some(n) = catch_name {
                        out.push(n.clone());
                    }
                    walk(body, out);
                    walk(catch_body, out);
                }
                _ => {}
            }
        }
    }
    walk(stmts, &mut out);
    out
}

/// 合并结果：语句表 + 调试源文件表 + 顶层语句到 `file_id` 的区间。
pub struct Merged {
    pub stmts: Vec<Stmt>,
    pub files: Vec<String>,
    pub spans: Vec<(usize, usize, u8)>,
}

/// 对全部单元做改名，返回按「依赖在前、根包在后」拼好的语句表。
pub fn merge(units: &mut Vec<Unit>) -> Result<Merged> {
    let views: HashMap<String, DepView> = units
        .iter()
        .filter(|u| !u.is_root)
        .map(|u| (u.name.clone(), u.view()))
        .collect();

    for u in units.iter_mut() {
        let mut r = Renamer {
            pkg: if u.is_root { "" } else { &u.name },
            is_root: u.is_root,
            globals: &u.globals,
            classes: &u.classes,
            deps: &views,
            scopes: Vec::new(),
        };
        r.stmts(&mut u.stmts, true)?;
    }

    let mut out = Vec::new();
    let mut files = Vec::new();
    let mut spans = Vec::new();
    for u in units.iter() {
        let start = out.len();
        out.extend(u.stmts.iter().cloned());
        let file_id = files.len() as u8;
        files.push(u.file.clone());
        spans.push((start, out.len(), file_id));
    }
    Ok(Merged {
        stmts: out,
        files,
        spans,
    })
}

/// 静态堆下界估计（字节）：列表/字典字面量 + 新建实例。
/// 只统计字面量出现处，不乘循环次数，因此是下界，不是峰值。
pub fn heap_estimate(stmts: &[Stmt], class_fields: &HashMap<String, usize>) -> u64 {
    fn expr(e: &Expr, cf: &HashMap<String, usize>) -> u64 {
        match &e.kind {
            ExprKind::List(items) => {
                let mut n = 12 + 4 * items.len() as u64;
                for it in items {
                    n += expr(it, cf);
                }
                n
            }
            ExprKind::Dict(pairs) => {
                let mut n = 12 + 8 * pairs.len() as u64;
                for (k, v) in pairs {
                    n += expr(k, cf);
                    n += expr(v, cf);
                }
                n
            }
            ExprKind::New(cls, args) => {
                let fields = cf.get(cls).copied().unwrap_or(0) as u64;
                let mut n = 16 + 4 * fields;
                for a in args {
                    n += expr(a, cf);
                }
                n
            }
            ExprKind::Bin(_, a, b) => expr(a, cf) + expr(b, cf),
            ExprKind::Unary(_, a) => expr(a, cf),
            ExprKind::Compare(a, rest) => {
                let mut n = expr(a, cf);
                for (_, b) in rest {
                    n += expr(b, cf);
                }
                n
            }
            ExprKind::Call(f, args) => {
                let mut n = expr(f, cf);
                for a in args {
                    n += expr(a, cf);
                }
                n
            }
            ExprKind::Index(a, b) => expr(a, cf) + expr(b, cf),
            ExprKind::Member(a, _) => expr(a, cf),
            ExprKind::MethodCall(a, _, args) => {
                let mut n = expr(a, cf);
                for x in args {
                    n += expr(x, cf);
                }
                n
            }
            _ => 0,
        }
    }

    fn walk_stmts(list: &[Stmt], cf: &HashMap<String, usize>) -> u64 {
        let mut n = 0;
        for s in list {
            n += match &s.kind {
                StmtKind::Let(_, e) | StmtKind::Assign(_, e) | StmtKind::Throw(e) => expr(e, cf),
                StmtKind::AssignIndex(a, b, c) => expr(a, cf) + expr(b, cf) + expr(c, cf),
                StmtKind::AssignMember(a, _, b) => expr(a, cf) + expr(b, cf),
                StmtKind::Expr(e) => expr(e, cf),
                StmtKind::If {
                    branches,
                    else_body,
                } => {
                    let mut m = 0;
                    for (c, b) in branches {
                        m += expr(c, cf) + walk_stmts(b, cf);
                    }
                    if let Some(b) = else_body {
                        m += walk_stmts(b, cf);
                    }
                    m
                }
                StmtKind::While(c, b) | StmtKind::Repeat(c, b) => expr(c, cf) + walk_stmts(b, cf),
                StmtKind::ForEach(_, e, b) => expr(e, cf) + walk_stmts(b, cf),
                StmtKind::FuncDef { body, .. } => walk_stmts(body, cf),
                StmtKind::Class(c) => {
                    let mut m = 0;
                    for (_, e) in &c.fields {
                        m += expr(e, cf);
                    }
                    for md in &c.methods {
                        m += walk_stmts(&md.body, cf);
                    }
                    m
                }
                StmtKind::Return(Some(e)) => expr(e, cf),
                StmtKind::Return(None) => 0,
                StmtKind::Try {
                    body, catch_body, ..
                } => walk_stmts(body, cf) + walk_stmts(catch_body, cf),
                _ => 0,
            };
        }
        n
    }

    walk_stmts(stmts, class_fields)
}

/// 合并后的类名 → 字段数（用于新建实例的堆下界）。
pub fn class_field_counts(stmts: &[Stmt]) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for s in stmts {
        if let StmtKind::Class(c) = &s.kind {
            out.insert(c.name.clone(), c.fields.len());
        }
    }
    out
}

/// 收集所有 `导入 模块`，用于和包规范里的「原生」并集对照。
pub fn collect_imports(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Import(m) => {
                out.insert(m.clone());
            }
            StmtKind::If {
                branches,
                else_body,
            } => {
                for (_, b) in branches {
                    collect_imports(b, out);
                }
                if let Some(b) = else_body {
                    collect_imports(b, out);
                }
            }
            StmtKind::While(_, b) | StmtKind::Repeat(_, b) => collect_imports(b, out),
            StmtKind::ForEach(_, _, b) => collect_imports(b, out),
            StmtKind::FuncDef { body, .. } => collect_imports(body, out),
            StmtKind::Class(c) => {
                for m in &c.methods {
                    collect_imports(&m.body, out);
                }
            }
            StmtKind::Try {
                body, catch_body, ..
            } => {
                collect_imports(body, out);
                collect_imports(catch_body, out);
            }
            _ => {}
        }
    }
}

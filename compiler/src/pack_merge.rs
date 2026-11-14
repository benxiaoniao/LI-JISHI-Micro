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
    /// `[导出] 类型`：可跨包的类/枚举名；`None` = 不导出任何类型。
    pub types: Option<HashSet<String>>,
    pub classes: HashSet<String>,
    /// 依赖包定义的枚举名。
    pub enums: HashSet<String>,
    /// 顶层 `私有` 声明，任何情况下都不跨包。
    pub private: HashSet<String>,
}

pub struct Unit {
    pub name: String,
    pub is_root: bool,
    pub stmts: Vec<Stmt>,
    pub globals: HashSet<String>,
    pub classes: HashSet<String>,
    pub enums: HashSet<String>,
    pub private: HashSet<String>,
    pub exports: Option<HashSet<String>>,
    pub export_types: Option<HashSet<String>>,
    /// 入口源文件（调试表的显示名）。
    pub file: String,
}

impl Unit {
    pub fn new(
        name: String,
        is_root: bool,
        stmts: Vec<Stmt>,
        exports: Option<Vec<String>>,
        export_types: Option<Vec<String>>,
        file: String,
    ) -> Unit {
        let mut globals = HashSet::new();
        let mut classes = HashSet::new();
        let mut enums = HashSet::new();
        let mut private = HashSet::new();
        collect_top(&stmts, &mut globals, &mut classes, &mut enums, &mut private);
        Unit {
            name,
            is_root,
            stmts,
            globals,
            classes,
            enums,
            private,
            exports: exports.map(|v| v.into_iter().collect()),
            export_types: export_types.map(|v| v.into_iter().collect()),
            file,
        }
    }

    pub fn view(&self) -> DepView {
        DepView {
            exports: self.exports.clone(),
            types: self.export_types.clone(),
            classes: self.classes.clone(),
            enums: self.enums.clone(),
            private: self.private.clone(),
        }
    }
}

fn collect_top(
    stmts: &[Stmt],
    globals: &mut HashSet<String>,
    classes: &mut HashSet<String>,
    enums: &mut HashSet<String>,
    private: &mut HashSet<String>,
) {
    for s in stmts {
        let name = match &s.kind {
            StmtKind::Let(n, _) | StmtKind::FuncDef { name: n, .. } => Some(n.clone()),
            StmtKind::Class(c) => {
                globals.insert(c.name.clone());
                classes.insert(c.name.clone());
                Some(c.name.clone())
            }
            StmtKind::Enum(e) => {
                // 枚举名按模块级名字处理：依赖包里会随其他顶层名字一起改名。
                globals.insert(e.name.clone());
                enums.insert(e.name.clone());
                Some(e.name.clone())
            }
            StmtKind::ForEach(n, _, _) => Some(n.clone()),
            _ => None,
        };
        if let Some(n) = name {
            globals.insert(n.clone());
            if s.is_private {
                private.insert(n);
            }
        }
    }
}

/// 依赖包的顶层只允许声明：`导入` / `令` / `函数` / `类` / `枚举`。
pub fn check_dependency_top_level(unit: &Unit) -> Result<()> {
    for s in &unit.stmts {
        let ok = matches!(
            s.kind,
            StmtKind::Import(_)
                | StmtKind::Let(_, _)
                | StmtKind::FuncDef { .. }
                | StmtKind::Class(_)
                | StmtKind::Enum(_)
        );
        if !ok {
            return Err(CompileError::new(
                "E127",
                s.line,
                1,
                format!(
                    "依赖包「{}」的顶层只允许声明（导入/令/函数/类/枚举），不能出现{}；\
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
        StmtKind::Assert(_, _) => "断言",
        StmtKind::If { .. } => "如果",
        StmtKind::While(_, _) => "当",
        StmtKind::Repeat(_, _) => "循环",
        StmtKind::ForEach(_, _, _) => "遍历",
        StmtKind::Break => "中断",
        StmtKind::Continue => "继续",
        StmtKind::FuncDef { .. } => "函数",
        StmtKind::Class(_) => "类",
        StmtKind::Enum(_) => "枚举",
        StmtKind::Match { .. } => "匹配",
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
    enums: &'a HashSet<String>,
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

    /// 这个名字是不是某个依赖包的枚举（依赖包自己的枚举已改名，根包不该再看到）。
    fn dep_enum_owner(&self, n: &str) -> Option<&str> {
        if self.shadowed(n) || self.globals.contains(n) {
            return None;
        }
        self.deps
            .iter()
            .find(|(_, v)| v.enums.contains(n))
            .map(|(k, _)| k.as_str())
    }

    /// 依赖包的类型是否可跨包：默认公开；`私有` 永不导出；
    /// 写了 `[导出] 类型` 时按该清单过滤。
    fn type_exported(&self, pkg: &str, name: &str) -> bool {
        let Some(view) = self.deps.get(pkg) else {
            return false;
        };
        if view.private.contains(name) {
            return false;
        }
        view.types
            .as_ref()
            .map(|t| t.contains(name))
            .unwrap_or(true)
    }

    fn is_private(&self, pkg: &str, name: &str) -> bool {
        self.deps
            .get(pkg)
            .map(|v| v.private.contains(name))
            .unwrap_or(false)
    }

    /// 把类型标注里的跨包引用改成合并后的名字。
    /// `包名.类名` → `包名::类名`；依赖包内部的 `类名` → `包名::类名`。
    fn mangle_ann(&self, ann: &mut Option<String>, line: u32) -> Result<()> {
        let Some(name) = ann.as_mut() else {
            return Ok(());
        };
        if let Some((pkg, ty)) = name.split_once('.') {
            if self.is_dep_pkg(pkg) {
                let view = &self.deps[pkg];
                if !view.classes.contains(ty) && !view.enums.contains(ty) {
                    return Err(CompileError::new(
                        "E128",
                        line,
                        1,
                        format!("包「{}」里没有类型「{}」", pkg, ty),
                    ));
                }
                if self.is_private(pkg, ty) {
                    return Err(CompileError::new(
                        "E128",
                        line,
                        1,
                        format!("类型「{}.{}」是私有声明，不能跨包使用", pkg, ty),
                    ));
                }
                if !self.type_exported(pkg, ty) {
                    return Err(CompileError::new(
                        "E128",
                        line,
                        1,
                        format!(
                            "类型「{}.{}」没有导出；请在它的 砾包.toml [导出] 类型 里列出这个名字",
                            pkg, ty
                        ),
                    ));
                }
                *name = format!("{}::{}", pkg, ty);
            }
        } else if !self.is_root
            && (self.classes.contains(name.as_str()) || self.enums.contains(name.as_str()))
        {
            *name = self.mangle(name);
        }
        Ok(())
    }

    fn global_ref(&self, n: &str) -> Option<String> {
        if !self.is_root && !self.shadowed(n) && self.globals.contains(n) {
            Some(self.mangle(n))
        } else {
            None
        }
    }

    /// 改写分支模式里的枚举名（依赖包内改名为 `包名::枚举名`；跨包看 `[导出] 类型`）。
    fn match_patterns(&mut self, pats: &mut [MatchPattern], line: u32) -> Result<()> {
        for pat in pats {
            let MatchPattern::EnumMember(n, _) = pat else {
                continue;
            };
            // `包名.枚举名.成员`：解析器把枚举名拼成 `包名.枚举名`。
            if let Some((pkg, en)) = n.split_once('.') {
                let pkg = pkg.to_string();
                let en = en.to_string();
                if !self.is_dep_pkg(&pkg) {
                    return Err(CompileError::new(
                        "E128",
                        line,
                        1,
                        format!("包「{}」不在依赖里；跨包枚举要写 包名.枚举名.成员", pkg),
                    ));
                }
                if !self.deps[&pkg].enums.contains(&en) {
                    return Err(CompileError::new(
                        "E128",
                        line,
                        1,
                        format!("包「{}」里没有枚举「{}」", pkg, en),
                    ));
                }
                if self.is_private(&pkg, &en) {
                    return Err(CompileError::new(
                        "E128",
                        line,
                        1,
                        format!("枚举「{}.{}」是私有声明，不能跨包使用", pkg, en),
                    ));
                }
                if !self.type_exported(&pkg, &en) {
                    return Err(CompileError::new(
                        "E128",
                        line,
                        1,
                        format!(
                            "枚举「{}.{}」没有导出；请在它的 砾包.toml [导出] 类型 里列出这个名字",
                            pkg, en
                        ),
                    ));
                }
                *n = format!("{}::{}", pkg, en);
                continue;
            }
            if let Some(m) = self.global_ref(n) {
                *n = m;
            } else if self.is_root {
                if let Some(owner) = self.dep_enum_owner(n) {
                    return Err(CompileError::new(
                        "E128",
                        line,
                        1,
                        format!(
                            "枚举「{}.{}」不能直接写；跨包请用 {}.{}.成员，并在对方 [导出] 类型 里列出",
                            owner, n, owner, n
                        ),
                    )
                    .with_hint("枚举跨包需要显式导出；枚举名本身不能当值用"));
                }
            }
        }
        Ok(())
    }

    fn check_export(&self, pkg: &str, name: &str, line: u32) -> Result<()> {
        let view = &self.deps[pkg];
        if view.private.contains(name) {
            return Err(CompileError::new(
                "E128",
                line,
                1,
                format!(
                    "「{}.{}」是私有声明，不能跨包使用；去掉 `私有` 或在对方导出对应名字",
                    pkg, name
                ),
            ));
        }
        if view.classes.contains(name) {
            return Err(CompileError::new(
                "E128",
                line,
                1,
                format!(
                    "类「{}.{}」不能当值用；跨包创建请写 `新建 {}.{}(...)`，类型需在对方 [导出] 类型 里列出",
                    pkg, name, pkg, name
                ),
            ));
        }
        if view.enums.contains(name) {
            return Err(CompileError::new(
                "E128",
                line,
                1,
                format!(
                    "枚举「{}.{}」不能当值用；请写 `{}.{}.成员`，类型需在对方 [导出] 类型 里列出",
                    pkg, name, pkg, name
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
        let line = s.line;
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
            StmtKind::Assert(c, m) => {
                self.expr(c)?;
                if let Some(m) = m {
                    self.expr(m)?;
                }
                Ok(())
            }
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
            StmtKind::FuncDef {
                name,
                params,
                param_types,
                ret_type,
                body,
                ..
            } => {
                if module {
                    if let Some(m) = self.global_ref(name) {
                        *name = m;
                    }
                }
                for t in param_types.iter_mut() {
                    self.mangle_ann(t, line)?;
                }
                self.mangle_ann(ret_type, line)?;
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
                    // 父类名与类名走同一套改名规则（依赖包内的父类要同步）。
                    if let Some(m) = c.parent.as_deref().and_then(|p| self.global_ref(p)) {
                        c.parent = Some(m);
                    }
                }
                for (_, e) in c.fields.iter_mut() {
                    self.expr(e)?;
                }
                for m in c.methods.iter_mut() {
                    for t in m.param_types.iter_mut() {
                        self.mangle_ann(t, line)?;
                    }
                    self.mangle_ann(&mut m.ret_type, line)?;
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
            StmtKind::Enum(e) => {
                if module {
                    if let Some(m) = self.global_ref(&e.name) {
                        e.name = m;
                    }
                }
                Ok(())
            }
            StmtKind::Match {
                subject,
                arms,
                else_body,
            } => {
                self.expr(subject)?;
                for (pats, body) in arms.iter_mut() {
                    self.match_patterns(pats, line)?;
                    self.stmts(body, false)?;
                }
                if let Some(body) = else_body {
                    self.stmts(body, false)?;
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
        // `包名.枚举名.成员`：把内层整体折成 `包名::枚举名`，外层只留一个成员访问。
        let (eline, ecol) = (e.line, e.col);
        if let ExprKind::Member(base, _) = &mut e.kind {
            let mut folded: Option<String> = None;
            if let ExprKind::Member(inner, en) = &base.kind {
                if let ExprKind::Name(p) = &inner.kind {
                    if self.is_dep_pkg(p) && self.deps[p].enums.contains(en) {
                        if self.is_private(p, en) {
                            return Err(CompileError::new(
                                "E128",
                                eline,
                                ecol,
                                format!("枚举「{}.{}」是私有声明，不能跨包使用", p, en),
                            ));
                        }
                        if !self.type_exported(p, en) {
                            return Err(CompileError::new(
                                "E128",
                                eline,
                                ecol,
                                format!(
                                    "枚举「{}.{}」没有导出；请在它的 砾包.toml [导出] 类型 里列出这个名字",
                                    p, en
                                ),
                            ));
                        }
                        folded = Some(format!("{}::{}", p, en));
                    }
                }
            }
            if let Some(name) = folded {
                base.kind = ExprKind::Name(name);
            }
        }

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

        // 根包直接写依赖包的枚举名（`状态.运行`）：提示改用 `包名.状态.运行`。
        if self.is_root {
            if let ExprKind::Member(base, _) = &e.kind {
                if let ExprKind::Name(n) = &base.kind {
                    if let Some(owner) = self.dep_enum_owner(n) {
                        return Err(CompileError::new(
                            "E128",
                            e.line,
                            e.col,
                            format!(
                                "枚举「{}.{}」不能直接写；跨包请用 {}.{}.成员，并在对方 [导出] 类型 里列出",
                                owner, n, owner, n
                            ),
                        )
                        .with_hint("枚举跨包需要显式导出；枚举名本身不能当值用"));
                    }
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
                if let Some((pkg, name)) = cls.split_once('.') {
                    let pkg = pkg.to_string();
                    let name = name.to_string();
                    if !self.is_dep_pkg(&pkg) {
                        return Err(CompileError::new(
                            "E128",
                            e.line,
                            e.col,
                            format!(
                                "包「{}」不在依赖里；跨包创建要写 `新建 包名.类名(...)`",
                                pkg
                            ),
                        ));
                    }
                    if !self.deps[&pkg].classes.contains(&name) {
                        return Err(CompileError::new(
                            "E128",
                            e.line,
                            e.col,
                            format!("包「{}」里没有类「{}」", pkg, name),
                        ));
                    }
                    if self.is_private(&pkg, &name) {
                        return Err(CompileError::new(
                            "E128",
                            e.line,
                            e.col,
                            format!("类「{}.{}」是私有声明，不能跨包使用", pkg, name),
                        ));
                    }
                    if !self.type_exported(&pkg, &name) {
                        return Err(CompileError::new(
                            "E128",
                            e.line,
                            e.col,
                            format!(
                                "类「{}.{}」没有导出；请在它的 砾包.toml [导出] 类型 里列出这个名字",
                                pkg, name
                            ),
                        ));
                    }
                    *cls = format!("{}::{}", pkg, name);
                } else if !self.is_root && self.classes.contains(cls.as_str()) {
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
            ExprKind::SuperCall(_, args) => {
                for a in args {
                    self.expr(a)?;
                }
            }
            ExprKind::AnonFunc(params, param_types, ret_type, body) => {
                for t in param_types.iter_mut() {
                    self.mangle_ann(t, e.line)?;
                }
                self.mangle_ann(ret_type, e.line)?;
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
                r?;
            }
            ExprKind::Match(subject, arms, else_value) => {
                self.expr(subject)?;
                for (pats, v) in arms.iter_mut() {
                    self.match_patterns(pats, e.line)?;
                    self.expr(v)?;
                }
                self.expr(else_value)?;
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
                StmtKind::Match {
                    arms, else_body, ..
                } => {
                    for (_, b) in arms {
                        walk(b, out);
                    }
                    if let Some(b) = else_body {
                        walk(b, out);
                    }
                }
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
            enums: &u.enums,
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
            ExprKind::SuperCall(_, args) => {
                let mut n = 0;
                for x in args {
                    n += expr(x, cf);
                }
                n
            }
            ExprKind::AnonFunc(_, _, _, body) => walk_stmts(body, cf),
            ExprKind::Match(subject, arms, else_value) => {
                let mut n = expr(subject, cf);
                for (_, v) in arms {
                    n += expr(v, cf);
                }
                n + expr(else_value, cf)
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
                StmtKind::Assert(c, m) => expr(c, cf) + m.as_ref().map(|x| expr(x, cf)).unwrap_or(0),
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
                StmtKind::Match {
                    subject,
                    arms,
                    else_body,
                } => {
                    let mut m = expr(subject, cf);
                    for (_, b) in arms {
                        m += walk_stmts(b, cf);
                    }
                    if let Some(b) = else_body {
                        m += walk_stmts(b, cf);
                    }
                    m
                }
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

/// 合并后的类名 → 字段数（用于新建实例的堆下界）；含继承来的字段。
pub fn class_field_counts(stmts: &[Stmt]) -> HashMap<String, usize> {
    let mut own: HashMap<String, (Option<String>, usize)> = HashMap::new();
    for s in stmts {
        if let StmtKind::Class(c) = &s.kind {
            own.insert(c.name.clone(), (c.parent.clone(), c.fields.len()));
        }
    }
    let mut out = HashMap::new();
    for name in own.keys() {
        let mut total = 0usize;
        let mut cur = Some(name.clone());
        let mut hops = 0usize;
        while let Some(c) = cur {
            let Some((p, n)) = own.get(&c) else { break };
            total += *n;
            hops += 1;
            if hops > own.len() {
                break; // 成环时停在已累计部分，真正报错由 codegen 负责
            }
            cur = p.clone();
        }
        out.insert(name.clone(), total);
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
            StmtKind::Match {
                arms, else_body, ..
            } => {
                for (_, b) in arms {
                    collect_imports(b, out);
                }
                if let Some(b) = else_body {
                    collect_imports(b, out);
                }
            }
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

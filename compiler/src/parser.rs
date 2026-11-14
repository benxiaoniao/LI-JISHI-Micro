use crate::ast::*;
use crate::error::{CompileError, Result};
use crate::lexer::{tokenize, Kw, Tok, Token};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// 块嵌套深度：0 = 模块顶层；`私有` 只允许在顶层。
    depth: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
        }
    }

    pub fn parse_program(&mut self) -> Result<Vec<Stmt>> {
        let mut out = Vec::new();
        while !matches!(self.peek().tok, Tok::Eof) {
            if matches!(self.peek().tok, Tok::Newline) {
                self.pos += 1;
                continue;
            }
            out.push(self.parse_stmt()?);
        }
        Ok(out)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek_at(&self, n: usize) -> &Token {
        &self.tokens[(self.pos + n).min(self.tokens.len() - 1)]
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos.min(self.tokens.len() - 1)].clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn err(&self, code: &'static str, msg: impl Into<String>) -> CompileError {
        let (l, c) = (self.peek().line, self.peek().col);
        CompileError::new(code, l, c, msg)
    }

    fn expect_op(&mut self, op: &'static str) -> Result<()> {
        if let Tok::Op(o) = &self.peek().tok {
            if *o == op {
                self.pos += 1;
                return Ok(());
            }
        }
        Err(self.err("E109", format!("这里应该写「{}」", op)))
    }

    fn expect_kw(&mut self, kw: Kw, text: &str) -> Result<()> {
        if let Tok::Kw(k) = self.peek().tok {
            if k == kw {
                self.pos += 1;
                return Ok(());
            }
        }
        Err(self.err("E109", format!("这里应该写「{}」", text)))
    }

    fn expect_newline(&mut self) -> Result<()> {
        match self.peek().tok {
            Tok::Newline => {
                self.pos += 1;
                Ok(())
            }
            Tok::Eof | Tok::Dedent => Ok(()),
            _ => Err(self.err("E109", "这一行到这儿应该结束了")),
        }
    }

    fn expect_ident(&mut self) -> Result<(String, u32, u32)> {
        let t = self.peek().clone();
        while matches!(t.tok, Tok::Newline) {
            // 不跨行找名字
            break;
        }
        if let Tok::Ident(name) = t.tok {
            self.pos += 1;
            Ok((name, t.line, t.col))
        } else {
            Err(self.err("E109", "这里应该写一个名字"))
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        self.expect_op(":")?;
        self.expect_newline()?;
        if !matches!(self.peek().tok, Tok::Indent) {
            return Err(self.err("E109", "这里应该换行并缩进"));
        }
        self.pos += 1;
        self.depth += 1;
        let mut stmts = Vec::new();
        while !matches!(self.peek().tok, Tok::Dedent | Tok::Eof) {
            if matches!(self.peek().tok, Tok::Newline) {
                self.pos += 1;
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        self.depth -= 1;
        if matches!(self.peek().tok, Tok::Dedent) {
            self.pos += 1;
        }
        Ok(stmts)
    }

    /// 解析 `(参数, ...)` 与可选的 `-> 类型`。
    /// 返回（参数名、与参数一一对应的类型标注、返回类型标注）。
    fn parse_params(
        &mut self,
    ) -> Result<(Vec<String>, Vec<Option<String>>, Option<String>)> {
        self.expect_op("(")?;
        let mut params = Vec::new();
        let mut types = Vec::new();
        if !matches!(self.peek().tok, Tok::Op(")")) {
            loop {
                let (p, pl, pc) = self.expect_ident()?;
                let mut ty = None;
                if matches!(self.peek().tok, Tok::Op(":")) {
                    self.pos += 1;
                    ty = Some(self.parse_type_name()?);
                }
                if matches!(self.peek().tok, Tok::Op("=")) {
                    return Err(CompileError::new(
                        "E107",
                        pl,
                        pc,
                        "Micro Core v0 不支持参数默认值",
                    ));
                }
                params.push(p);
                types.push(ty);
                if matches!(self.peek().tok, Tok::Op(",")) {
                    self.pos += 1;
                    continue;
                }
                break;
            }
        }
        self.expect_op(")")?;
        let ret = if matches!(self.peek().tok, Tok::Op("->")) {
            self.pos += 1;
            Some(self.parse_type_name()?)
        } else {
            None
        };
        Ok((params, types, ret))
    }

    /// 类型名：允许 `包名.类名` 两段（跨包类型），单段与原来一致。
    fn parse_type_name(&mut self) -> Result<String> {
        let (mut name, _, _) = self.expect_ident()?;
        while matches!(self.peek().tok, Tok::Op(".")) {
            self.pos += 1;
            let (seg, _, _) = self.expect_ident()?;
            name.push('.');
            name.push_str(&seg);
        }
        Ok(name)
    }

    /// 函数体：`函数 f(x):` + 缩进块，或 `函数 f(x): 表达式` 的单行写法。
    fn parse_func_body(&mut self) -> Result<Vec<Stmt>> {
        self.expect_op(":")?;
        if !matches!(self.peek().tok, Tok::Newline) {
            let e = self.parse_expr()?;
            let line = e.line;
            if !matches!(
                self.peek().tok,
                Tok::Newline | Tok::Dedent | Tok::Eof | Tok::Op(")") | Tok::Op(",") | Tok::Op("]")
            ) {
                return Err(self.err("E109", "单行函数体后面应该换行"));
            }
            // 换行留给外层语句消费（`令 f = 函数(x): x + 1` 的「令」要自己收尾）。
            return Ok(vec![Stmt {
                kind: StmtKind::Return(Some(e)),
                line,
                is_private: false,
            }]);
        }
        self.pos += 1;
        if !matches!(self.peek().tok, Tok::Indent) {
            return Err(self.err("E109", "这里应该换行并缩进"));
        }
        self.pos += 1;
        self.depth += 1;
        let mut stmts = Vec::new();
        while !matches!(self.peek().tok, Tok::Dedent | Tok::Eof) {
            if matches!(self.peek().tok, Tok::Newline) {
                self.pos += 1;
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        self.depth -= 1;
        self.close_block();
        Ok(stmts)
    }

    /// 缩进块结束：吃掉 Dedent，并处理紧随其后的换行。
    ///
    /// 块体出现在表达式位置时（`令 f = 函数(x):` + 缩进块、`令 x = 匹配 …`），
    /// 源码里那一行是有换行的，但词法上的 Dedent 已经先出现；这里补一个换行 token，
    /// 让外层语句（令/返回/调用等）照常收尾。
    fn close_block(&mut self) {
        if matches!(self.peek().tok, Tok::Dedent) {
            self.pos += 1;
        }
        match self.peek().tok {
            Tok::Newline => self.pos += 1,
            Tok::Eof
            | Tok::Dedent
            | Tok::Op(")")
            | Tok::Op(",")
            | Tok::Op("]")
            | Tok::Op("}") => {}
            _ => {
                let (l, c) = (self.peek().line, self.peek().col);
                self.tokens.insert(
                    self.pos,
                    Token {
                        tok: Tok::Newline,
                        line: l,
                        col: c,
                    },
                );
            }
        }
    }

    /// `私有` 开头：只允许修饰模块顶层的 `令/函数/类/枚举`（可带 `最终 类`）。
    fn parse_private_decl(&mut self, line: u32, col: u32) -> Result<Stmt> {
        const MSG: &str = "「私有」只能修饰模块顶层的 令/函数/类/枚举";
        if self.depth > 0 {
            return Err(CompileError::new("E144", line, col, MSG));
        }
        self.pos += 1;
        if matches!(self.peek().tok, Tok::Kw(Kw::Private)) {
            return Err(CompileError::new("E144", line, col, "「私有」重复"));
        }
        let ok = match self.peek().tok {
            Tok::Kw(Kw::Ling) | Tok::Kw(Kw::Class) | Tok::Kw(Kw::Enum) | Tok::Kw(Kw::Final) => true,
            Tok::Kw(Kw::Func) => !matches!(self.peek_at(1).tok, Tok::Op("(")),
            _ => false,
        };
        if !ok {
            return Err(CompileError::new("E144", line, col, MSG));
        }
        let mut stmt = self.parse_stmt()?;
        stmt.is_private = true;
        match &mut stmt.kind {
            StmtKind::FuncDef { is_private, .. } => *is_private = true,
            StmtKind::Class(c) => c.is_private = true,
            StmtKind::Enum(e) => e.is_private = true,
            _ => {}
        }
        Ok(stmt)
    }

    /// 类体：调用前已经吃掉 `类`（或 `最终 类`）。
    fn parse_class_body(&mut self, line: u32, is_final: bool) -> Result<Stmt> {
        let (name, _, _) = self.expect_ident()?;
        let parent = if matches!(self.peek().tok, Tok::Kw(Kw::Inherit)) {
            self.pos += 1;
            Some(self.expect_ident()?.0)
        } else {
            None
        };
        self.expect_op(":")?;
        self.expect_newline()?;
        if !matches!(self.peek().tok, Tok::Indent) {
            return Err(self.err("E109", "这里应该换行并缩进"));
        }
        self.pos += 1;
        let mut fields: Vec<(String, Expr)> = Vec::new();
        let mut methods: Vec<MethodDecl> = Vec::new();
        while !matches!(self.peek().tok, Tok::Dedent | Tok::Eof) {
            if matches!(self.peek().tok, Tok::Newline) {
                self.pos += 1;
                continue;
            }
            match self.peek().tok {
                Tok::Kw(Kw::Ling) => {
                    self.pos += 1;
                    let (fname, _, _) = self.expect_ident()?;
                    self.expect_op("=")?;
                    let e = self.parse_expr()?;
                    self.expect_newline()?;
                    fields.push((fname, e));
                }
                Tok::Kw(Kw::Func) => methods.push(self.parse_func_decl()?),
                Tok::Kw(Kw::Private) => {
                    let bad = self.peek().clone();
                    return Err(CompileError::new(
                        "E144",
                        bad.line,
                        bad.col,
                        "「私有」只能修饰模块顶层的 令/函数/类/枚举，不能写在类体里",
                    ));
                }
                Tok::Kw(Kw::Final) => {
                    let bad = self.peek().clone();
                    self.pos += 1;
                    if !matches!(self.peek().tok, Tok::Kw(Kw::Func)) {
                        return Err(CompileError::new(
                            "E142",
                            bad.line,
                            bad.col,
                            "「最终」只能修饰类（最终 类）或类方法（最终 函数）",
                        ));
                    }
                    let mut m = self.parse_func_decl()?;
                    m.is_final = true;
                    methods.push(m);
                }
                _ => {
                    let bad = self.peek().clone();
                    return Err(CompileError::new(
                        "E118",
                        bad.line,
                        bad.col,
                        "类体里只能写字段（令 名 = 默认值）或函数（函数 名(...):）",
                    ));
                }
            }
        }
        if matches!(self.peek().tok, Tok::Dedent) {
            self.pos += 1;
        }
        Ok(Stmt {
            kind: StmtKind::Class(ClassDecl {
                name,
                parent,
                is_final,
                is_private: false,
                fields,
                methods,
            }),
            line,
            is_private: false,
        })
    }

    fn parse_func_decl(&mut self) -> Result<MethodDecl> {
        let t = self.peek().clone();
        let line = t.line;
        self.pos += 1;
        let (name, _, _) = self.expect_ident()?;
        let (params, param_types, ret_type) = self.parse_params()?;
        let body = self.parse_func_body()?;
        Ok(MethodDecl {
            name,
            params,
            param_types,
            ret_type,
            body,
            is_final: false,
            line,
        })
    }

    /// `匹配 表达式:` + 缩进分支，每个分支体是单行表达式；必须有 `情形 其他`。
    /// 调用方已经吃掉「匹配」关键字。
    fn parse_match_expr(&mut self, line: u32, col: u32) -> Result<Expr> {
        let subject = self.parse_expr()?;
        self.expect_op(":")?;
        self.expect_newline()?;
        if !matches!(self.peek().tok, Tok::Indent) {
            return Err(self.err("E139", "「匹配」下面应该换行并缩进，写「情形」分支"));
        }
        self.pos += 1;
        let mut arms: Vec<(Vec<MatchPattern>, Expr)> = Vec::new();
        let mut else_value: Option<Expr> = None;
        while !matches!(self.peek().tok, Tok::Dedent | Tok::Eof) {
            if matches!(self.peek().tok, Tok::Newline) {
                self.pos += 1;
                continue;
            }
            if !matches!(self.peek().tok, Tok::Kw(Kw::Case)) {
                return Err(self.err("E139", "「匹配」里只能写「情形」分支"));
            }
            if else_value.is_some() {
                return Err(self.err("E139", "「情形 其他」后面不能再有分支"));
            }
            self.pos += 1;
            let pats = self.parse_match_patterns()?;
            self.expect_op(":")?;
            if matches!(self.peek().tok, Tok::Newline) {
                return Err(self.err(
                    "E139",
                    "匹配作为表达式时分支只写一个表达式（如 情形 0: \"零\"）",
                ));
            }
            let v = self.parse_expr()?;
            self.expect_newline()?;
            if pats.len() == 1 && matches!(pats[0], MatchPattern::Else) {
                else_value = Some(v);
            } else {
                arms.push((pats, v));
            }
        }
        self.close_block();
        let Some(else_value) = else_value else {
            return Err(CompileError::new(
                "E139",
                line,
                col,
                "匹配作为表达式必须有「情形 其他」兜底",
            ));
        };
        if arms.is_empty() {
            return Err(CompileError::new(
                "E139",
                line,
                col,
                "「匹配」至少要有一个「情形」分支",
            ));
        }
        Ok(Expr {
            kind: ExprKind::Match(Box::new(subject), arms, Box::new(else_value)),
            line,
            col,
        })
    }

    /// 解析一个情形模式：整数、负数、闭区间 `起..止`、枚举成员或 `其他`。
    fn parse_match_pattern(&mut self) -> Result<MatchPattern> {
        let t = self.peek().clone();
        match t.tok {
            Tok::Int(v) => {
                self.pos += 1;
                if matches!(self.peek().tok, Tok::Op("..")) {
                    self.pos += 1;
                    let hi = self.expect_pattern_int(t.line, t.col)?;
                    if v > hi {
                        return Err(CompileError::new(
                            "E139",
                            t.line,
                            t.col,
                            format!("「情形」区间的起点 {} 大于终点 {}", v, hi),
                        ));
                    }
                    return Ok(MatchPattern::Range(v, hi));
                }
                Ok(MatchPattern::Int(v))
            }
            Tok::Op("-") => {
                self.pos += 1;
                let lo = self.expect_pattern_int(t.line, t.col)?;
                let lo = -lo;
                if matches!(self.peek().tok, Tok::Op("..")) {
                    self.pos += 1;
                    let hi = self.expect_pattern_int(t.line, t.col)?;
                    if lo > hi {
                        return Err(CompileError::new(
                            "E139",
                            t.line,
                            t.col,
                            format!("「情形」区间的起点 {} 大于终点 {}", lo, hi),
                        ));
                    }
                    return Ok(MatchPattern::Range(lo, hi));
                }
                Ok(MatchPattern::Int(lo))
            }
            Tok::Ident(name) => {
                // 单独一个「其他」是兜底分支；`其他.成员` 仍按枚举成员解析。
                if name == "其他" && !matches!(self.peek_at(1).tok, Tok::Op(".")) {
                    self.pos += 1;
                    return Ok(MatchPattern::Else);
                }
                self.pos += 1;
                if !matches!(self.peek().tok, Tok::Op(".")) {
                    return Err(CompileError::new(
                        "E139",
                        t.line,
                        t.col,
                        "「情形」只能是整数字面量、区间（起..止）、枚举成员（名.成员）或「其他」",
                    ));
                }
                self.pos += 1;
                let (member, _, _) = self.expect_ident()?;
                // `包名.枚举名.成员`：三段式模式，枚举名先拼成 `包名.枚举名`，
                // 由跨包合并阶段改写成 `包名::枚举名`。
                if matches!(self.peek().tok, Tok::Op(".")) {
                    self.pos += 1;
                    let (member2, _, _) = self.expect_ident()?;
                    return Ok(MatchPattern::EnumMember(
                        format!("{}.{}", name, member),
                        member2,
                    ));
                }
                Ok(MatchPattern::EnumMember(name, member))
            }
            _ => Err(CompileError::new(
                "E139",
                t.line,
                t.col,
                "「情形」只能是整数字面量、区间（起..止）、枚举成员（名.成员）或「其他」",
            )),
        }
    }

    /// 区间右端点：只允许整数字面量（可带负号）。
    fn expect_pattern_int(&mut self, line: u32, col: u32) -> Result<i64> {
        let neg = if matches!(self.peek().tok, Tok::Op("-")) {
            self.pos += 1;
            true
        } else {
            false
        };
        if let Tok::Int(v) = self.peek().tok {
            self.pos += 1;
            Ok(if neg { -v } else { v })
        } else {
            Err(CompileError::new(
                "E139",
                line,
                col,
                "「情形」区间只能写整数字面量（如 1..5 或 -3..-1）",
            ))
        }
    }

    /// 一个「情形」后面可以写多个模式，用逗号分隔：`情形 1, 3, 5:`。
    fn parse_match_patterns(&mut self) -> Result<Vec<MatchPattern>> {
        let mut pats = vec![self.parse_match_pattern()?];
        while matches!(self.peek().tok, Tok::Op(",")) {
            self.pos += 1;
            pats.push(self.parse_match_pattern()?);
        }
        if pats.len() > 1 && pats.iter().any(|p| matches!(p, MatchPattern::Else)) {
            return Err(self.err("E139", "「其他」必须单独成一个分支，不能和别的值写在一起"));
        }
        Ok(pats)
    }

    /// 字符串字面量的文本插值：`"值 = {x}"` → `"值 = " + 文本(x)`。
    ///
    /// - `{表达式}`：花括号里按普通表达式解析，结果自动套 `文本()`；
    /// - `{{` / `}}` 或 `\{` / `\}`：字面花括号（后者在词法阶段已换成哨兵字符）；
    /// - 单独的 `}` 按字面处理。
    fn parse_string_literal(&mut self, s: String, line: u32, col: u32) -> Result<Expr> {
        let chars: Vec<char> = s.chars().collect();
        let mut parts: Vec<Expr> = Vec::new();
        let mut text = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match c {
                '\u{E000}' => {
                    text.push('{');
                    i += 1;
                }
                '\u{E001}' => {
                    text.push('}');
                    i += 1;
                }
                '{' => {
                    if i + 1 < chars.len() && chars[i + 1] == '{' {
                        text.push('{');
                        i += 2;
                        continue;
                    }
                    let start = i + 1;
                    let mut j = start;
                    let mut depth = 1;
                    while j < chars.len() {
                        match chars[j] {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if j >= chars.len() {
                        return Err(CompileError::new(
                            "E139",
                            line,
                            col,
                            "文本插值缺少右花括号「}」",
                        ));
                    }
                    let raw: String = chars[start..j].iter().collect();
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        return Err(CompileError::new(
                            "E139",
                            line,
                            col,
                            "文本插值里要写表达式，如 \"值 = {x}\"",
                        ));
                    }
                    if !text.is_empty() {
                        parts.push(Expr {
                            kind: ExprKind::Str(std::mem::take(&mut text)),
                            line,
                            col,
                        });
                    }
                    // 片段开头的空白不参与切词（否则会被当成缩进），列号相应右移。
                    let lead = raw.chars().take_while(|c| c.is_whitespace()).count() as u32;
                    let v = self.parse_interp_expr(
                        trimmed,
                        line,
                        col + 1 + start as u32 + lead,
                    )?;
                    let (vl, vc) = (v.line, v.col);
                    parts.push(Expr {
                        kind: ExprKind::Call(
                            Box::new(Expr {
                                kind: ExprKind::Name("文本".to_string()),
                                line: vl,
                                col: vc,
                            }),
                            vec![v],
                        ),
                        line: vl,
                        col: vc,
                    });
                    i = j + 1;
                }
                '}' => {
                    if i + 1 < chars.len() && chars[i + 1] == '}' {
                        i += 2;
                    } else {
                        i += 1;
                    }
                    text.push('}');
                }
                _ => {
                    text.push(c);
                    i += 1;
                }
            }
        }
        if !text.is_empty() || parts.is_empty() {
            parts.push(Expr {
                kind: ExprKind::Str(text),
                line,
                col,
            });
        }
        let mut iter = parts.into_iter();
        let mut acc = iter.next().unwrap();
        for p in iter {
            let (l, c) = (acc.line, acc.col);
            acc = Expr {
                kind: ExprKind::Bin(BinOp::Add, Box::new(acc), Box::new(p)),
                line: l,
                col: c,
            };
        }
        Ok(acc)
    }

    /// 解析插值花括号里的表达式：单独切词，位置按所在字符串的列号平移。
    fn parse_interp_expr(&mut self, src: &str, line: u32, col: u32) -> Result<Expr> {
        let mut toks = tokenize(src).map_err(|e| {
            CompileError::new("E139", line, col, format!("插值里的表达式无法解析：{}", e))
        })?;
        for t in toks.iter_mut() {
            if t.line == 1 {
                t.col = t.col.saturating_add(col.saturating_sub(1));
            }
            t.line = t.line.saturating_add(line.saturating_sub(1));
        }
        let mut p = Parser::new(toks);
        let e = p.parse_expr()?;
        if matches!(p.peek().tok, Tok::Newline) {
            p.pos += 1;
        }
        if !matches!(p.peek().tok, Tok::Eof) {
            return Err(CompileError::new(
                "E139",
                p.peek().line,
                p.peek().col,
                "插值里的表达式后面还有多余内容",
            ));
        }
        Ok(e)
    }

    fn parse_stmt(&mut self) -> Result<Stmt> {
        let t = self.peek().clone();
        let line = t.line;
        if matches!(t.tok, Tok::Kw(Kw::Private)) {
            return self.parse_private_decl(t.line, t.col);
        }
        match t.tok {
            Tok::Kw(Kw::Ling) => {
                self.pos += 1;
                let (name, _, _) = self.expect_ident()?;
                self.expect_op("=")?;
                let e = self.parse_expr()?;
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Let(name, e),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::If) => {
                self.pos += 1;
                let mut branches = Vec::new();
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                branches.push((cond, body));
                let mut else_body = None;
                loop {
                    match self.peek().tok {
                        Tok::Kw(Kw::Elif) => {
                            self.pos += 1;
                            let cond = self.parse_expr()?;
                            let body = self.parse_block()?;
                            branches.push((cond, body));
                        }
                        Tok::Kw(Kw::Else) => {
                            self.pos += 1;
                            else_body = Some(self.parse_block()?);
                            break;
                        }
                        _ => break,
                    }
                }
                Ok(Stmt {
                    kind: StmtKind::If { branches, else_body },
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::While) => {
                self.pos += 1;
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt {
                    kind: StmtKind::While(cond, body),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Loop) => {
                self.pos += 1;
                let count = self.parse_expr()?;
                if matches!(self.peek().tok, Tok::Kw(Kw::Times)) {
                    self.pos += 1;
                }
                let body = self.parse_block()?;
                Ok(Stmt {
                    kind: StmtKind::Repeat(count, body),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::ForEach) => {
                self.pos += 1;
                let (name, _, _) = self.expect_ident()?;
                self.expect_kw(Kw::In, "在")?;
                let iterable = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt {
                    kind: StmtKind::ForEach(name, iterable, body),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Func) if !matches!(self.peek_at(1).tok, Tok::Op("(")) => {
                let m = self.parse_func_decl()?;
                Ok(Stmt {
                    kind: StmtKind::FuncDef {
                        name: m.name,
                        params: m.params,
                        param_types: m.param_types,
                        ret_type: m.ret_type,
                        body: m.body,
                        is_private: false,
                    },
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Class) => {
                self.pos += 1;
                self.parse_class_body(line, false)
            }
            Tok::Kw(Kw::Final) => {
                self.pos += 1;
                if matches!(self.peek().tok, Tok::Kw(Kw::Func)) {
                    return Err(CompileError::new(
                        "E142",
                        line,
                        t.col,
                        "「最终 函数」只能写在类体里（修饰类方法）",
                    ));
                }
                self.expect_kw(Kw::Class, "类")?;
                self.parse_class_body(line, true)
            }
            Tok::Kw(Kw::Enum) => {
                self.pos += 1;
                let (name, _, _) = self.expect_ident()?;
                self.expect_op(":")?;
                self.expect_newline()?;
                if !matches!(self.peek().tok, Tok::Indent) {
                    return Err(self.err("E139", "「枚举」下面应该换行并缩进，写成员名"));
                }
                self.pos += 1;
                let mut members = Vec::new();
                while !matches!(self.peek().tok, Tok::Dedent | Tok::Eof) {
                    if matches!(self.peek().tok, Tok::Newline) {
                        self.pos += 1;
                        continue;
                    }
                    let (m, ml, _) = self.expect_ident()?;
                    let value = if matches!(self.peek().tok, Tok::Op("=")) {
                        self.pos += 1;
                        let neg = if matches!(self.peek().tok, Tok::Op("-")) {
                            self.pos += 1;
                            true
                        } else {
                            false
                        };
                        let v = match self.peek().tok.clone() {
                            Tok::Int(v) => v,
                            _ => {
                                return Err(CompileError::new(
                                    "E139",
                                    ml,
                                    1,
                                    "枚举成员的显式值必须是整数字面量（如 运行 = 10）",
                                ))
                            }
                        };
                        self.pos += 1;
                        Some(if neg { -v } else { v })
                    } else {
                        None
                    };
                    members.push(EnumMember {
                        name: m,
                        value,
                        line: ml,
                    });
                    self.expect_newline()?;
                }
                if matches!(self.peek().tok, Tok::Dedent) {
                    self.pos += 1;
                }
                if members.is_empty() {
                    return Err(CompileError::new(
                        "E139",
                        line,
                        1,
                        format!("枚举「{}」至少要有一个成员", name),
                    ));
                }
                Ok(Stmt {
                    kind: StmtKind::Enum(EnumDecl {
                        name,
                        members,
                        is_private: false,
                    }),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Match) => {
                self.pos += 1;
                let subject = self.parse_expr()?;
                self.expect_op(":")?;
                self.expect_newline()?;
                if !matches!(self.peek().tok, Tok::Indent) {
                    return Err(self.err("E139", "「匹配」下面应该换行并缩进，写「情形」分支"));
                }
                self.pos += 1;
                let mut arms: Vec<(Vec<MatchPattern>, Vec<Stmt>)> = Vec::new();
                let mut else_body: Option<Vec<Stmt>> = None;
                let mut saw_else = false;
                while !matches!(self.peek().tok, Tok::Dedent | Tok::Eof) {
                    if matches!(self.peek().tok, Tok::Newline) {
                        self.pos += 1;
                        continue;
                    }
                    if !matches!(self.peek().tok, Tok::Kw(Kw::Case)) {
                        return Err(self.err("E139", "「匹配」里只能写「情形」分支"));
                    }
                    if saw_else {
                        return Err(self.err("E139", "「情形 其他」后面不能再有分支"));
                    }
                    self.pos += 1;
                    let pats = self.parse_match_patterns()?;
                    if matches!(self.peek().tok, Tok::Op(":"))
                        && !matches!(self.peek_at(1).tok, Tok::Newline)
                    {
                        return Err(self.err(
                            "E139",
                            "语句形式的「匹配」分支要换行缩进；单行表达式分支请写成「令 名字 = 匹配 …」或「返回 匹配 …」",
                        ));
                    }
                    let body = self.parse_block()?;
                    if pats.len() == 1 && matches!(pats[0], MatchPattern::Else) {
                        else_body = Some(body);
                        saw_else = true;
                    } else {
                        arms.push((pats, body));
                    }
                }
                if matches!(self.peek().tok, Tok::Dedent) {
                    self.pos += 1;
                }
                if arms.is_empty() && else_body.is_none() {
                    return Err(CompileError::new(
                        "E139",
                        line,
                        1,
                        "「匹配」至少要有一个「情形」分支",
                    ));
                }
                Ok(Stmt {
                    kind: StmtKind::Match {
                        subject,
                        arms,
                        else_body,
                    },
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Case) => Err(CompileError::new(
                "E139",
                line,
                t.col,
                "「情形」只能写在「匹配」里",
            )),
            Tok::Kw(Kw::Assert) => {
                self.pos += 1;
                let cond = self.parse_expr()?;
                let msg = if matches!(self.peek().tok, Tok::Op(",")) {
                    self.pos += 1;
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Assert(cond, msg),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Return) => {
                self.pos += 1;
                let e = if matches!(self.peek().tok, Tok::Newline | Tok::Dedent | Tok::Eof) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Return(e),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Throw) => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Throw(e),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Try) => {
                self.pos += 1;
                let body = self.parse_block()?;
                if !matches!(self.peek().tok, Tok::Kw(Kw::Catch)) {
                    return Err(self.err(
                        "E109",
                        "「尝试」块后面必须紧跟同层的「捕获」块",
                    ));
                }
                self.pos += 1;
                let catch_name = if let Tok::Ident(name) = self.peek().tok.clone() {
                    self.pos += 1;
                    Some(name)
                } else {
                    None
                };
                let catch_body = self.parse_block()?;
                Ok(Stmt {
                    kind: StmtKind::Try {
                        body,
                        catch_name,
                        catch_body,
                    },
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Catch) => Err(self.err(
                "E109",
                "「捕获」必须紧跟在「尝试」块之后",
            )),
            Tok::Kw(Kw::Break) => {
                self.pos += 1;
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Break,
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Continue) => {
                self.pos += 1;
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Continue,
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Import) => {
                self.pos += 1;
                let (module, _, _) = self.expect_ident()?;
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Import(module),
                    line,
                    is_private: false,
                })
            }
            Tok::Kw(Kw::Unsupported(name)) => Err(CompileError::new(
                "E103",
                line,
                t.col,
                format!("使用了 v0 不支持的关键字「{}」（如用等）", name),
            )),
            _ => {
                let e = self.parse_expr()?;
                let aug = match self.peek().tok {
                    Tok::Op("+=") => Some(BinOp::Add),
                    Tok::Op("-=") => Some(BinOp::Sub),
                    Tok::Op("*=") => Some(BinOp::Mul),
                    Tok::Op("/=") => Some(BinOp::Div),
                    Tok::Op("//=") => Some(BinOp::FloorDiv),
                    Tok::Op("%=") => Some(BinOp::Mod),
                    Tok::Op("**=") => Some(BinOp::Pow),
                    _ => None,
                };
                if let Some(bop) = aug {
                    self.pos += 1;
                    let rhs = self.parse_expr()?;
                    self.expect_newline()?;
                    let lhs = e.clone();
                    match &e.kind {
                        ExprKind::Name(n) => Ok(Stmt {
                            kind: StmtKind::Assign(
                                n.clone(),
                                Expr {
                                    kind: ExprKind::Bin(bop, Box::new(lhs), Box::new(rhs)),
                                    line: e.line,
                                    col: e.col,
                                },
                            ),
                            line,
                            is_private: false,
                        }),
                        ExprKind::Index(obj, idx) => Ok(Stmt {
                            kind: StmtKind::AssignIndex(
                                (**obj).clone(),
                                idx.clone(),
                                Box::new(Expr {
                                    kind: ExprKind::Bin(bop, Box::new(lhs), Box::new(rhs)),
                                    line: e.line,
                                    col: e.col,
                                }),
                            ),
                            line,
                            is_private: false,
                        }),
                        ExprKind::Member(obj, name) => Ok(Stmt {
                            kind: StmtKind::AssignMember(
                                (**obj).clone(),
                                name.clone(),
                                Box::new(Expr {
                                    kind: ExprKind::Bin(bop, Box::new(lhs), Box::new(rhs)),
                                    line: e.line,
                                    col: e.col,
                                }),
                            ),
                            line,
                            is_private: false,
                        }),
                        ExprKind::SelfRef => Err(CompileError::new(
                            "E120",
                            e.line,
                            e.col,
                            "「本」是只读绑定，不能赋值",
                        )),
                        _ => Err(CompileError::new(
                            "E109",
                            e.line,
                            e.col,
                            "复合赋值的左边只能是名字、下标或字段",
                        )),
                    }
                } else if matches!(self.peek().tok, Tok::Op("=")) {
                    self.pos += 1;
                    let rhs = self.parse_expr()?;
                    self.expect_newline()?;
                    match e.kind {
                        ExprKind::Name(n) => Ok(Stmt {
                            kind: StmtKind::Assign(n, rhs),
                            line,
                            is_private: false,
                        }),
                        ExprKind::Index(obj, idx) => Ok(Stmt {
                            kind: StmtKind::AssignIndex(*obj, idx, Box::new(rhs)),
                            line,
                            is_private: false,
                        }),
                        ExprKind::Member(obj, name) => Ok(Stmt {
                            kind: StmtKind::AssignMember(*obj, name, Box::new(rhs)),
                            line,
                            is_private: false,
                        }),
                        ExprKind::SelfRef => Err(CompileError::new(
                            "E120",
                            e.line,
                            e.col,
                            "「本」是只读绑定，不能赋值",
                        )),
                        _ => Err(CompileError::new(
                            "E109",
                            e.line,
                            e.col,
                            "等号左边只能是名字、下标或字段",
                        )),
                    }
                } else {
                    self.expect_newline()?;
                    Ok(Stmt {
                        kind: StmtKind::Expr(e),
                        line,
                        is_private: false,
                    })
                }
            }
        }
    }

    pub fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while matches!(self.peek().tok, Tok::Kw(Kw::Or)) {
            self.pos += 1;
            let right = self.parse_and()?;
            let (line, col) = (left.line, left.col);
            left = Expr {
                kind: ExprKind::Bin(BinOp::Or, Box::new(left), Box::new(right)),
                line,
                col,
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_not()?;
        while matches!(self.peek().tok, Tok::Kw(Kw::And)) {
            self.pos += 1;
            let right = self.parse_not()?;
            let (line, col) = (left.line, left.col);
            left = Expr {
                kind: ExprKind::Bin(BinOp::And, Box::new(left), Box::new(right)),
                line,
                col,
            };
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr> {
        if matches!(self.peek().tok, Tok::Kw(Kw::Not)) {
            let t = self.bump();
            let e = self.parse_not()?;
            return Ok(Expr {
                kind: ExprKind::Unary(UnOp::Not, Box::new(e)),
                line: t.line,
                col: t.col,
            });
        }
        self.parse_compare()
    }

    fn parse_compare(&mut self) -> Result<Expr> {
        let first = self.parse_bor()?;
        let mut pairs: Vec<(CmpOp, Box<Expr>)> = Vec::new();
        loop {
            let op = match self.peek().tok {
                Tok::Op("==") => CmpOp::Eq,
                Tok::Op("!=") => CmpOp::Ne,
                Tok::Op("<") => CmpOp::Lt,
                Tok::Op(">") => CmpOp::Gt,
                Tok::Op("<=") => CmpOp::Le,
                Tok::Op(">=") => CmpOp::Ge,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_bor()?;
            pairs.push((op, Box::new(right)));
        }
        if pairs.is_empty() {
            Ok(first)
        } else {
            let (line, col) = (first.line, first.col);
            Ok(Expr {
                kind: ExprKind::Compare(Box::new(first), pairs),
                line,
                col,
            })
        }
    }

    /// 位或：最低的位运算层，比比较更高（`a & b == c` 是 `(a & b) == c`）。
    fn parse_bor(&mut self) -> Result<Expr> {
        let mut left = self.parse_bxor()?;
        while matches!(self.peek().tok, Tok::Op("|")) {
            self.pos += 1;
            let right = self.parse_bxor()?;
            let (line, col) = (left.line, left.col);
            left = Expr {
                kind: ExprKind::Bin(BinOp::BitOr, Box::new(left), Box::new(right)),
                line,
                col,
            };
        }
        Ok(left)
    }

    fn parse_bxor(&mut self) -> Result<Expr> {
        let mut left = self.parse_band()?;
        while matches!(self.peek().tok, Tok::Op("^")) {
            self.pos += 1;
            let right = self.parse_band()?;
            let (line, col) = (left.line, left.col);
            left = Expr {
                kind: ExprKind::Bin(BinOp::BitXor, Box::new(left), Box::new(right)),
                line,
                col,
            };
        }
        Ok(left)
    }

    fn parse_band(&mut self) -> Result<Expr> {
        let mut left = self.parse_shift()?;
        while matches!(self.peek().tok, Tok::Op("&")) {
            self.pos += 1;
            let right = self.parse_shift()?;
            let (line, col) = (left.line, left.col);
            left = Expr {
                kind: ExprKind::Bin(BinOp::BitAnd, Box::new(left), Box::new(right)),
                line,
                col,
            };
        }
        Ok(left)
    }

    /// 移位：高于 `&`、低于 `+ -`。
    fn parse_shift(&mut self) -> Result<Expr> {
        let mut left = self.parse_add()?;
        loop {
            let op = match self.peek().tok {
                Tok::Op("<<") => BinOp::Shl,
                Tok::Op(">>") => BinOp::Shr,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_add()?;
            let (line, col) = (left.line, left.col);
            left = Expr {
                kind: ExprKind::Bin(op, Box::new(left), Box::new(right)),
                line,
                col,
            };
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek().tok {
                Tok::Op("+") => BinOp::Add,
                Tok::Op("-") => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_mul()?;
            let (line, col) = (left.line, left.col);
            left = Expr {
                kind: ExprKind::Bin(op, Box::new(left), Box::new(right)),
                line,
                col,
            };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek().tok {
                Tok::Op("*") => BinOp::Mul,
                Tok::Op("/") => BinOp::Div,
                Tok::Op("//") => BinOp::FloorDiv,
                Tok::Op("%") => BinOp::Mod,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            let (line, col) = (left.line, left.col);
            left = Expr {
                kind: ExprKind::Bin(op, Box::new(left), Box::new(right)),
                line,
                col,
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if matches!(self.peek().tok, Tok::Op("-")) {
            let t = self.bump();
            let e = self.parse_unary()?;
            return Ok(Expr {
                kind: ExprKind::Unary(UnOp::Neg, Box::new(e)),
                line: t.line,
                col: t.col,
            });
        }
        if matches!(self.peek().tok, Tok::Op("~")) {
            let t = self.bump();
            let e = self.parse_unary()?;
            return Ok(Expr {
                kind: ExprKind::Unary(UnOp::Bnot, Box::new(e)),
                line: t.line,
                col: t.col,
            });
        }
        self.parse_power()
    }

    fn parse_power(&mut self) -> Result<Expr> {
        let left = self.parse_postfix()?;
        if matches!(self.peek().tok, Tok::Op("**")) {
            self.pos += 1;
            let right = self.parse_unary()?;
            let (line, col) = (left.line, left.col);
            return Ok(Expr {
                kind: ExprKind::Bin(BinOp::Pow, Box::new(left), Box::new(right)),
                line,
                col,
            });
        }
        Ok(left)
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek().tok {
                Tok::Op("(") => {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if !matches!(self.peek().tok, Tok::Op(")")) {
                        loop {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek().tok, Tok::Op(",")) {
                                self.pos += 1;
                                if matches!(self.peek().tok, Tok::Op(")")) {
                                    break;
                                }
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_op(")")?;
                    let (line, col) = (e.line, e.col);
                    e = Expr {
                        kind: ExprKind::Call(Box::new(e), args),
                        line,
                        col,
                    };
                }
                Tok::Op("[") => {
                    self.pos += 1;
                    let idx = self.parse_expr()?;
                    self.expect_op("]")?;
                    let (line, col) = (e.line, e.col);
                    e = Expr {
                        kind: ExprKind::Index(Box::new(e), Box::new(idx)),
                        line,
                        col,
                    };
                }
                Tok::Op(".") => {
                    self.pos += 1;
                    if !matches!(self.peek().tok, Tok::Ident(_)) {
                        return Err(self.err("E108", "「.」后面应该写字段名或方法名"));
                    }
                    let (name, _, _) = self.expect_ident()?;
                    let (line, col) = (e.line, e.col);
                    if matches!(self.peek().tok, Tok::Op("(")) {
                        self.pos += 1;
                        let mut args = Vec::new();
                        if !matches!(self.peek().tok, Tok::Op(")")) {
                            loop {
                                args.push(self.parse_expr()?);
                                if matches!(self.peek().tok, Tok::Op(",")) {
                                    self.pos += 1;
                                    if matches!(self.peek().tok, Tok::Op(")")) {
                                        break;
                                    }
                                    continue;
                                }
                                break;
                            }
                        }
                        self.expect_op(")")?;
                        e = Expr {
                            kind: ExprKind::MethodCall(Box::new(e), name, args),
                            line,
                            col,
                        };
                    } else {
                        e = Expr {
                            kind: ExprKind::Member(Box::new(e), name),
                            line,
                            col,
                        };
                    }
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let t = self.peek().clone();
        let (line, col) = (t.line, t.col);
        match t.tok {
            Tok::Int(v) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::Int(v),
                    line,
                    col,
                })
            }
            Tok::Dec(v) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::Dec(v),
                    line,
                    col,
                })
            }
            Tok::Str(s) => {
                self.pos += 1;
                self.parse_string_literal(s, line, col)
            }
            Tok::Kw(Kw::True) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::True,
                    line,
                    col,
                })
            }
            Tok::Kw(Kw::False) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::False,
                    line,
                    col,
                })
            }
            Tok::Kw(Kw::Nil) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::Nil,
                    line,
                    col,
                })
            }
            Tok::Kw(Kw::SelfRef) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::SelfRef,
                    line,
                    col,
                })
            }
            Tok::Kw(Kw::Super) => {
                self.pos += 1;
                if !matches!(self.peek().tok, Tok::Op(".")) {
                    return Err(CompileError::new(
                        "E141",
                        line,
                        col,
                        "「超」只能写 超.方法(...)，不能当值或赋值",
                    ));
                }
                self.pos += 1;
                let (name, nline, ncol) = self.expect_ident()?;
                if !matches!(self.peek().tok, Tok::Op("(")) {
                    return Err(CompileError::new(
                        "E141",
                        nline,
                        ncol,
                        "「超」后面必须跟方法调用，写 超.方法(...)",
                    ));
                }
                self.expect_op("(")?;
                let mut args = Vec::new();
                if !matches!(self.peek().tok, Tok::Op(")")) {
                    loop {
                        args.push(self.parse_expr()?);
                        if matches!(self.peek().tok, Tok::Op(",")) {
                            self.pos += 1;
                            if matches!(self.peek().tok, Tok::Op(")")) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect_op(")")?;
                Ok(Expr {
                    kind: ExprKind::SuperCall(name, args),
                    line,
                    col,
                })
            }
            Tok::Kw(Kw::Final) => Err(CompileError::new(
                "E142",
                line,
                col,
                "「最终」只能修饰类（最终 类）或类方法（最终 函数）",
            )),
            Tok::Kw(Kw::Private) => Err(CompileError::new(
                "E144",
                line,
                col,
                "「私有」只能修饰模块顶层的 令/函数/类/枚举",
            )),
            Tok::Kw(Kw::New) => {
                self.pos += 1;
                let cls = self.parse_type_name()?;
                self.expect_op("(")?;
                let mut args = Vec::new();
                if !matches!(self.peek().tok, Tok::Op(")")) {
                    loop {
                        args.push(self.parse_expr()?);
                        if matches!(self.peek().tok, Tok::Op(",")) {
                            self.pos += 1;
                            if matches!(self.peek().tok, Tok::Op(")")) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect_op(")")?;
                Ok(Expr {
                    kind: ExprKind::New(cls, args),
                    line,
                    col,
                })
            }
            Tok::Kw(Kw::Match) => {
                self.pos += 1;
                self.parse_match_expr(line, col)
            }
            Tok::Kw(Kw::Func) => {
                // 表达式位置的 `函数(...)` 是匿名函数。
                self.pos += 1;
                let (params, param_types, ret_type) = self.parse_params()?;
                let body = self.parse_func_body()?;
                Ok(Expr {
                    kind: ExprKind::AnonFunc(params, param_types, ret_type, body),
                    line,
                    col,
                })
            }
            Tok::Ident(name) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::Name(name),
                    line,
                    col,
                })
            }
            Tok::Op("(") => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.expect_op(")")?;
                Ok(e)
            }
            Tok::Op("[") => {
                self.pos += 1;
                let mut items = Vec::new();
                if !matches!(self.peek().tok, Tok::Op("]")) {
                    loop {
                        items.push(self.parse_expr()?);
                        if matches!(self.peek().tok, Tok::Op(",")) {
                            self.pos += 1;
                            if matches!(self.peek().tok, Tok::Op("]")) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect_op("]")?;
                Ok(Expr {
                    kind: ExprKind::List(items),
                    line,
                    col,
                })
            }
            Tok::Op("{") => {
                self.pos += 1;
                let mut pairs = Vec::new();
                if !matches!(self.peek().tok, Tok::Op("}")) {
                    loop {
                        let k = self.parse_expr()?;
                        self.expect_op(":")?;
                        let v = self.parse_expr()?;
                        pairs.push((k, v));
                        if matches!(self.peek().tok, Tok::Op(",")) {
                            self.pos += 1;
                            if matches!(self.peek().tok, Tok::Op("}")) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
                self.expect_op("}")?;
                Ok(Expr {
                    kind: ExprKind::Dict(pairs),
                    line,
                    col,
                })
            }
            _ => Err(CompileError::new(
                "E109",
                line,
                col,
                "这里应该是一个表达式",
            )),
        }
    }
}

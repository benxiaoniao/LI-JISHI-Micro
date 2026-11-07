use crate::ast::*;
use crate::error::{CompileError, Result};
use crate::lexer::{Kw, Tok, Token};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
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
        let mut stmts = Vec::new();
        while !matches!(self.peek().tok, Tok::Dedent | Tok::Eof) {
            if matches!(self.peek().tok, Tok::Newline) {
                self.pos += 1;
                continue;
            }
            stmts.push(self.parse_stmt()?);
        }
        if matches!(self.peek().tok, Tok::Dedent) {
            self.pos += 1;
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt> {
        let t = self.peek().clone();
        let line = t.line;
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
                })
            }
            Tok::Kw(Kw::While) => {
                self.pos += 1;
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt {
                    kind: StmtKind::While(cond, body),
                    line,
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
                })
            }
            Tok::Kw(Kw::Func) => {
                self.pos += 1;
                let (name, _, _) = self.expect_ident()?;
                self.expect_op("(")?;
                let mut params = Vec::new();
                if !matches!(self.peek().tok, Tok::Op(")")) {
                    loop {
                        let (p, pl, pc) = self.expect_ident()?;
                        if matches!(self.peek().tok, Tok::Op(":")) {
                            // 类型标注：读掉类型名，不参与运行
                            self.pos += 1;
                            let _ = self.expect_ident()?;
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
                        if matches!(self.peek().tok, Tok::Op(",")) {
                            self.pos += 1;
                            continue;
                        }
                        break;
                    }
                }
                self.expect_op(")")?;
                if matches!(self.peek().tok, Tok::Op("->")) {
                    self.pos += 1;
                    let _ = self.expect_ident()?;
                }
                let body = self.parse_block()?;
                Ok(Stmt {
                    kind: StmtKind::FuncDef { name, params, body },
                    line,
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
                })
            }
            Tok::Kw(Kw::Break) => {
                self.pos += 1;
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Break,
                    line,
                })
            }
            Tok::Kw(Kw::Continue) => {
                self.pos += 1;
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Continue,
                    line,
                })
            }
            Tok::Kw(Kw::Import) => {
                self.pos += 1;
                let (module, _, _) = self.expect_ident()?;
                self.expect_newline()?;
                Ok(Stmt {
                    kind: StmtKind::Import(module),
                    line,
                })
            }
            Tok::Kw(Kw::Unsupported(name)) => Err(CompileError::new(
                "E103",
                line,
                t.col,
                format!("Micro Core v0 不支持「{}」", name),
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
                        }),
                        _ => Err(CompileError::new(
                            "E109",
                            e.line,
                            e.col,
                            "复合赋值的左边只能是名字或下标",
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
                        }),
                        ExprKind::Index(obj, idx) => Ok(Stmt {
                            kind: StmtKind::AssignIndex(*obj, idx, Box::new(rhs)),
                            line,
                        }),
                        _ => Err(CompileError::new(
                            "E109",
                            e.line,
                            e.col,
                            "等号左边只能是名字或下标",
                        )),
                    }
                } else {
                    self.expect_newline()?;
                    Ok(Stmt {
                        kind: StmtKind::Expr(e),
                        line,
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
        let first = self.parse_add()?;
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
            let right = self.parse_add()?;
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
                    return Err(self.err("E108", "Micro Core v0 不支持属性访问「.」"));
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
                Ok(Expr {
                    kind: ExprKind::Str(s),
                    line,
                    col,
                })
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

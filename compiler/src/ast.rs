#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    And,
    Or,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    /// 小数：Q12 定点原始值（真实值 × 4096）。
    Dec(i64),
    Str(String),
    Nil,
    True,
    False,
    Name(String),
    List(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Compare(Box<Expr>, Vec<(CmpOp, Box<Expr>)>),
    Call(Box<Expr>, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Import(String),
    Let(String, Expr),
    Assign(String, Expr),
    AssignIndex(Expr, Box<Expr>, Box<Expr>),
    Expr(Expr),
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
    },
    While(Expr, Vec<Stmt>),
    Repeat(Expr, Vec<Stmt>),
    ForEach(String, Expr, Vec<Stmt>),
    Break,
    Continue,
    FuncDef {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
}

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
    /// `新建 类名(实参...)`
    New(String, Vec<Expr>),
    /// `对象.字段`（读）
    Member(Box<Expr>, String),
    /// `对象.方法(实参...)`
    MethodCall(Box<Expr>, String, Vec<Expr>),
    /// 方法内的 `本`
    SelfRef,
}

/// 类方法声明（`初始化` 也是普通方法，名字为「初始化」）。
#[derive(Clone, Debug)]
pub struct MethodDecl {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct ClassDecl {
    pub name: String,
    pub fields: Vec<(String, Expr)>,
    pub methods: Vec<MethodDecl>,
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
    /// `对象.字段 = 值`（接收者表达式、字段名、值）
    AssignMember(Expr, String, Box<Expr>),
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
    /// 顶层类声明；类体只允许字段与函数。
    Class(ClassDecl),
    Return(Option<Expr>),
    Throw(Expr),
    Try {
        body: Vec<Stmt>,
        catch_name: Option<String>,
        catch_body: Vec<Stmt>,
    },
}

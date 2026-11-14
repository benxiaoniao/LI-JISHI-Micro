#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    And,
    Or,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Not,
    /// 按位取反 `~`；编译期降级为 `x ^ -1`。
    Bnot,
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
    /// 匿名函数：`函数(参数): 表达式` 或 `函数(参数):` + 缩进块。
    /// 参数名、参数类型标注（与参数一一对应）、返回类型标注、函数体。
    AnonFunc(Vec<String>, Vec<Option<String>>, Option<String>, Vec<Stmt>),
    /// 匹配表达式：每个分支体是单行表达式，必须有 `情形 其他` 兜底。
    Match(Box<Expr>, Vec<(Vec<MatchPattern>, Expr)>, Box<Expr>),
    /// `超.方法(实参...)`：显式调用父类实现，编译期确定目标代码对象。
    SuperCall(String, Vec<Expr>),
    /// 方法内的 `本`
    SelfRef,
}

/// 类方法声明（`初始化` 也是普通方法，名字为「初始化」）。
#[derive(Clone, Debug)]
pub struct MethodDecl {
    pub name: String,
    pub params: Vec<String>,
    /// 与 `params` 一一对应的类型标注；没写为 `None`（`--strict-type` 才校验）。
    pub param_types: Vec<Option<String>>,
    pub ret_type: Option<String>,
    pub body: Vec<Stmt>,
    /// `最终 函数`：子类不能覆写这个方法。
    pub is_final: bool,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct ClassDecl {
    pub name: String,
    /// `继承 父类`；没有父类时为 `None`（单继承）。
    pub parent: Option<String>,
    /// `最终 类`：不能被继承。
    pub is_final: bool,
    /// `私有 类`：不导出到包外。
    pub is_private: bool,
    pub fields: Vec<(String, Expr)>,
    pub methods: Vec<MethodDecl>,
}

/// 枚举声明：成员取编译期整数常量，不产生运行期对象。
#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: String,
    pub members: Vec<EnumMember>,
    /// `私有 枚举`：不导出到包外。
    pub is_private: bool,
}

/// 枚举成员：`名字` 或 `名字 = 显式值`；没写显式值时取上一个值 +1（首个为 0）。
#[derive(Clone, Debug)]
pub struct EnumMember {
    pub name: String,
    pub value: Option<i64>,
    pub line: u32,
}

/// `匹配` 的分支模式。
#[derive(Clone, Debug)]
pub enum MatchPattern {
    Int(i64),
    /// 闭区间 `起..止`：两端都算命中。
    Range(i64, i64),
    /// `枚举名.成员`
    EnumMember(String, String),
    /// `其他`
    Else,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
    /// 顶层声明前的 `私有`：只在 `micc pack` 的跨包可见性里生效。
    pub is_private: bool,
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
        param_types: Vec<Option<String>>,
        ret_type: Option<String>,
        body: Vec<Stmt>,
        /// `私有 函数`：不导出到包外。
        is_private: bool,
    },
    /// 顶层类声明；类体只允许字段与函数。
    Class(ClassDecl),
    /// 顶层枚举声明；只产生编译期整数常量。
    Enum(EnumDecl),
    /// 匹配语句：主体只求值一次，按 `==`/区间顺序比较；只做语句，无返回值。
    Match {
        subject: Expr,
        arms: Vec<(Vec<MatchPattern>, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
    },
    /// `断言 条件[, 消息]`：条件为假时抛出消息（无消息则抛 "断言失败"）。
    Assert(Expr, Option<Expr>),
    Return(Option<Expr>),
    Throw(Expr),
    Try {
        body: Vec<Stmt>,
        catch_name: Option<String>,
        catch_body: Vec<Stmt>,
    },
}

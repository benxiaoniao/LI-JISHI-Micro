use crate::ast::*;
use crate::error::{CompileError, Result};
use crate::native_spec;
use crate::stdlib_spec;
use crate::vm_spec;
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

/// 语句级调试标记：指令流偏移 → 源文件与行号。
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub off: u32,
    pub line: u32,
    pub file_id: u8,
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
    /// 局部槽符号表：`(槽号, 名字)`，按槽号递增。无名槽（临时值）不收录。
    pub locals: Vec<(u8, String)>,
}

#[derive(Debug)]
pub struct ClassObj {
    pub name_idx: u8,
    pub init_code: u8,
    /// 父类下标；255 表示没有父类（单继承）。
    pub parent: u8,
    /// `最终 类`：不能再被继承。
    pub is_final: bool,
    /// 字段名在名字表中的下标，按声明顺序（父类扁平字段在前）。
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
    /// 枚举名 → (成员名 → 编译期整数常量)。
    enums: HashMap<String, HashMap<String, i32>>,
    /// `--no-assert`：整句 `断言` 不生成字节码。
    no_assert: bool,
    /// `--strict-type`：校验类型标注（默认关闭，保持宽松）。
    strict: bool,
    /// 顶层函数签名（名字 → 参数/返回类型标注）。
    func_sigs: HashMap<String, FuncSig>,
    /// 类下标 + 方法名 → 签名。
    method_sigs: HashMap<(u8, String), FuncSig>,
    /// 类下标 → `初始化` 签名。
    init_sigs: HashMap<u8, FuncSig>,
    /// 模块顶层声明/赋值的名字；这些名字即便与标准库同名，也按用户自己的算。
    declared: HashSet<String>,
}

/// 类型标注（只做编译期校验，不进入字节码）。
#[derive(Clone, Default)]
struct FuncSig {
    param_types: Vec<Option<String>>,
    ret_type: Option<String>,
}

/// 严格模式下能静态确认的类型。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    Int,
    Dec,
    Str,
    Bool,
    Nil,
    List,
    Dict,
    Bytes,
    Set,
    Range,
    Func,
    Class(u8),
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
            enums: HashMap::new(),
            no_assert: false,
            strict: false,
            func_sigs: HashMap::new(),
            method_sigs: HashMap::new(),
            init_sigs: HashMap::new(),
            declared: HashSet::new(),
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

    /// 在类表里找字段名下标（字段数组已按继承扁平化）。
    fn class_field_idx(&self, class_idx: u8, name: &str) -> Option<u8> {
        let c = self.classes.get(class_idx as usize)?;
        c.fields
            .iter()
            .copied()
            .find(|&ni| self.names.get(ni as usize).map(|s| s == name).unwrap_or(false))
    }

    /// 在类表里找方法名下标；沿父链上溯，子类同名方法优先（覆写）。
    fn class_method_idx(&self, class_idx: u8, name: &str) -> Option<u8> {
        let mut ci = class_idx;
        let mut hops = 0usize;
        while ci != 255 {
            let c = self.classes.get(ci as usize)?;
            if let Some(&(ni, _)) = c.methods.iter().find(|&&(ni, _)| {
                self.names.get(ni as usize).map(|s| s == name).unwrap_or(false)
            }) {
                return Some(ni);
            }
            hops += 1;
            if hops > self.classes.len() {
                return None;
            }
            ci = c.parent;
        }
        None
    }

    /// 沿父链找方法并返回代码对象下标（`超.方法(...)` 的静态调用目标）。
    fn class_method_code_idx(&self, class_idx: u8, name: &str) -> Option<u8> {
        let mut ci = class_idx;
        let mut hops = 0usize;
        while ci != 255 {
            let c = self.classes.get(ci as usize)?;
            if let Some(&(_, code)) = c.methods.iter().find(|&&(ni, _)| {
                self.names.get(ni as usize).map(|s| s == name).unwrap_or(false)
            }) {
                return Some(code);
            }
            hops += 1;
            if hops > self.classes.len() {
                return None;
            }
            ci = c.parent;
        }
        None
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

/// 把类型标注名解析成静态类型；未知返回 `None`。
/// 枚举在运行期就是整数，`枚举名` 作标注时按整数处理。
fn resolve_ty(b: &Builder, name: &str) -> Option<Ty> {
    Some(match name {
        "整数" => Ty::Int,
        "小数" => Ty::Dec,
        "文本" => Ty::Str,
        "布尔" => Ty::Bool,
        "空" => Ty::Nil,
        "列表" => Ty::List,
        "字典" => Ty::Dict,
        "字节" => Ty::Bytes,
        "集合" => Ty::Set,
        "范围" => Ty::Range,
        "函数" => Ty::Func,
        _ => {
            if let Some(&ci) = b.class_map.get(name) {
                return Some(Ty::Class(ci));
            }
            if b.enums.contains_key(name) {
                return Some(Ty::Int);
            }
            return None;
        }
    })
}

fn ty_name(b: &Builder, t: Ty) -> String {
    match t {
        Ty::Int => "整数".to_string(),
        Ty::Dec => "小数".to_string(),
        Ty::Str => "文本".to_string(),
        Ty::Bool => "布尔".to_string(),
        Ty::Nil => "空".to_string(),
        Ty::List => "列表".to_string(),
        Ty::Dict => "字典".to_string(),
        Ty::Bytes => "字节".to_string(),
        Ty::Set => "集合".to_string(),
        Ty::Range => "范围".to_string(),
        Ty::Func => "函数".to_string(),
        Ty::Class(ci) => b
            .classes
            .get(ci as usize)
            .and_then(|c| b.names.get(c.name_idx as usize))
            .cloned()
            .unwrap_or_else(|| "<类>".to_string()),
    }
}

/// 严格模式：标注的类型名必须是内建类型或已定义的类。
fn check_ann(b: &Builder, name: &str, line: u32, col: u32) -> Result<()> {
    if !b.strict || resolve_ty(b, name).is_some() {
        return Ok(());
    }
    Err(CompileError::new(
        "E143",
        line,
        col,
        format!(
            "类型标注「{}」不认识；可用：整数/小数/文本/布尔/空/列表/字典/字节/集合/范围/函数，或已定义的类/枚举（跨包用 包名.类名）",
            name
        ),
    ))
}

/// 实际类型能否传给标注类型：整数可当小数；`空` 可给引用类型；子类实例可给父类。
fn assignable(b: &Builder, actual: Ty, expected: Ty) -> bool {
    if actual == expected {
        return true;
    }
    match (actual, expected) {
        (Ty::Int, Ty::Dec) => true,
        (Ty::Nil, Ty::Class(_) | Ty::List | Ty::Dict | Ty::Bytes | Ty::Set | Ty::Func) => true,
        (Ty::Class(a), Ty::Class(e)) => {
            let mut k = a;
            let mut hops = 0usize;
            while k != 255 {
                if k == e {
                    return true;
                }
                hops += 1;
                if hops > b.classes.len() {
                    return false;
                }
                k = match b.classes.get(k as usize) {
                    Some(c) => c.parent,
                    None => return false,
                };
            }
            false
        }
        _ => false,
    }
}

/// 沿父链找方法签名（`本.方法` 与 `超.方法` 的实参检查用）。
fn method_sig<'a>(b: &'a Builder, class_idx: u8, name: &str) -> Option<&'a FuncSig> {
    let mut ci = class_idx;
    let mut hops = 0usize;
    while ci != 255 {
        if let Some(sig) = b.method_sigs.get(&(ci, name.to_string())) {
            return Some(sig);
        }
        hops += 1;
        if hops > b.classes.len() {
            return None;
        }
        ci = b.classes.get(ci as usize)?.parent;
    }
    None
}

/// 少数内建的返回类型（够严格模式做保守推断）。
fn builtin_ret(name: &str) -> Option<Ty> {
    Some(match name {
        "字节" => Ty::Bytes,
        "集合" => Ty::Set,
        "范围" => Ty::Range,
        "文本" | "字节文本" | "字节十六进制" | "十六进制字节" | "子串" | "替换" | "连接"
        | "去空白" | "转大写" | "转小写" => Ty::Str,
        "分割" | "切片" | "反转" | "排序" | "键列表" | "字节列表" => Ty::List,
        "整数" | "长度" | "查找" | "字节取" => Ty::Int,
        _ => return None,
    })
}

fn numeric_ty(a: Ty, b: Ty) -> Option<Ty> {
    match (a, b) {
        (Ty::Int, Ty::Int) => Some(Ty::Int),
        (Ty::Int, Ty::Dec) | (Ty::Dec, Ty::Int) | (Ty::Dec, Ty::Dec) => Some(Ty::Dec),
        _ => None,
    }
}

fn bin_ty(op: BinOp, a: Ty, c: Ty) -> Option<Ty> {
    match op {
        BinOp::Add => match (a, c) {
            (Ty::Str, Ty::Str) => Some(Ty::Str),
            (Ty::List, Ty::List) => Some(Ty::List),
            (Ty::Bytes, Ty::Bytes) => Some(Ty::Bytes),
            _ => numeric_ty(a, c),
        },
        BinOp::And | BinOp::Or => Some(Ty::Bool),
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
            if a == Ty::Int && c == Ty::Int {
                Some(Ty::Int)
            } else {
                None
            }
        }
        _ => numeric_ty(a, c),
    }
}

/// 严格模式下“能静态看出”的表达式类型；看不出来返回 `None`（不报错）。
fn obvious_ty(b: &Builder, fs: &FnState, e: &Expr) -> Option<Ty> {
    match &e.kind {
        ExprKind::Int(_) => Some(Ty::Int),
        ExprKind::Dec(_) => Some(Ty::Dec),
        ExprKind::Str(_) => Some(Ty::Str),
        ExprKind::True | ExprKind::False => Some(Ty::Bool),
        ExprKind::Nil => Some(Ty::Nil),
        ExprKind::List(_) => Some(Ty::List),
        ExprKind::Dict(_) => Some(Ty::Dict),
        ExprKind::New(cls, _) => b.class_map.get(cls).map(|&ci| Ty::Class(ci)),
        ExprKind::AnonFunc(..) => Some(Ty::Func),
        ExprKind::SelfRef => fs.current_class.map(Ty::Class),
        ExprKind::Member(obj, _) => match &obj.kind {
            ExprKind::Name(en) if b.enums.contains_key(en) => Some(Ty::Int),
            _ => None,
        },
        ExprKind::Unary(op, x) => match op {
            UnOp::Neg => match obvious_ty(b, fs, x)? {
                Ty::Int => Some(Ty::Int),
                Ty::Dec => Some(Ty::Dec),
                _ => None,
            },
            UnOp::Not => Some(Ty::Bool),
            UnOp::Bnot => Some(Ty::Int),
        },
        ExprKind::Bin(op, x, y) => {
            let a = obvious_ty(b, fs, x)?;
            let c = obvious_ty(b, fs, y)?;
            bin_ty(*op, a, c)
        }
        ExprKind::Compare(..) => Some(Ty::Bool),
        ExprKind::Call(callee, _) => match &callee.kind {
            ExprKind::Name(n) => builtin_ret(n).or_else(|| {
                b.func_sigs
                    .get(n)
                    .and_then(|s| s.ret_type.as_deref())
                    .and_then(|t| resolve_ty(b, t))
            }),
            _ => None,
        },
        ExprKind::MethodCall(obj, name, _) if matches!(obj.kind, ExprKind::SelfRef) => fs
            .current_class
            .and_then(|ci| method_sig(b, ci, name))
            .and_then(|s| s.ret_type.as_deref())
            .and_then(|t| resolve_ty(b, t)),
        ExprKind::SuperCall(name, _) => {
            let ci = fs.current_class?;
            let parent = b.classes.get(ci as usize)?.parent;
            if parent == 255 {
                return None;
            }
            method_sig(b, parent, name)
                .and_then(|s| s.ret_type.as_deref())
                .and_then(|t| resolve_ty(b, t))
        }
        ExprKind::Match(_, arms, else_value) => {
            let mut found: Option<Ty> = None;
            for (_, v) in arms {
                let t = obvious_ty(b, fs, v)?;
                match found {
                    Some(prev) if prev != t => return None,
                    _ => found = Some(t),
                }
            }
            let t = obvious_ty(b, fs, else_value)?;
            match found {
                Some(prev) if prev != t => None,
                _ => Some(t),
            }
        }
        _ => None,
    }
}

/// 严格模式：按签名逐项检查能静态看出的实参。
fn check_call_args(
    b: &Builder,
    fs: &FnState,
    sig: &FuncSig,
    args: &[Expr],
    line: u32,
) -> Result<()> {
    if !b.strict {
        return Ok(());
    }
    for (i, ann) in sig.param_types.iter().enumerate() {
        let Some(ann) = ann else { continue };
        let Some(arg) = args.get(i) else { break };
        let Some(actual) = obvious_ty(b, fs, arg) else {
            continue;
        };
        let Some(expected) = resolve_ty(b, ann) else {
            continue;
        };
        if !assignable(b, actual, expected) {
            return Err(CompileError::new(
                "E143",
                line,
                arg.col,
                format!(
                    "第 {} 个实参标注为「{}」，实际是「{}」",
                    i + 1,
                    ann,
                    ty_name(b, actual)
                ),
            ));
        }
    }
    Ok(())
}

/// 模块顶层声明/赋值：这些名字优先于同名标准库函数。
fn collect_declared(stmts: &[Stmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in stmts {
        match &s.kind {
            StmtKind::Let(n, _) | StmtKind::Assign(n, _) | StmtKind::ForEach(n, _, _) => {
                out.insert(n.clone());
            }
            StmtKind::FuncDef { name, .. } => {
                out.insert(name.clone());
            }
            StmtKind::Class(c) => {
                out.insert(c.name.clone());
            }
            StmtKind::Enum(e) => {
                out.insert(e.name.clone());
            }
            _ => {}
        }
    }
    out
}

/// 收集顶层函数签名（严格模式下调用点检查用）。
fn collect_func_sigs(b: &mut Builder, stmts: &[Stmt]) {
    for s in stmts {
        if let StmtKind::FuncDef {
            name,
            param_types,
            ret_type,
            ..
        } = &s.kind
        {
            b.func_sigs.insert(
                name.clone(),
                FuncSig {
                    param_types: param_types.clone(),
                    ret_type: ret_type.clone(),
                },
            );
        }
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
    /// 当前函数/方法的返回类型标注（`--strict-type` 用来校验 `返回`）。
    ret_ann: Option<String>,
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
                last.file_id = self.file_id;
                return;
            }
            if last.line == line && last.file_id == self.file_id {
                return;
            }
        }
        self.marks.push(Mark {
            off,
            line,
            file_id: self.file_id,
        });
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
    /// 合并参数/局部槽与捕获槽，按槽号递增；同名同槽去重。
    /// 临时槽（`alloc_temp`）没有名字，自然不会出现在这里。
    fn symbols(&self) -> Vec<(u8, String)> {
        let mut out: Vec<(u8, String)> = Vec::new();
        for (name, &slot) in self.slots.iter().chain(self.captures.iter()) {
            if out.iter().any(|(s, n)| *s == slot && n == name) {
                continue;
            }
            out.push((slot, name.clone()));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        // 槽号必须一一对应：命名槽只从 nlocals 单调分配，临时槽不进 slots。
        debug_assert!(
            out.windows(2).all(|w| w[0].0 != w[1].0),
            "同一函数里出现两个名字共用一个局部槽"
        );
        out
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

/// 变量/参数名不能与枚举重名：枚举名只能用 `枚举名.成员` 的形式读。
fn reject_enum_name(b: &Builder, name: &str, line: u32, col: u32) -> Result<()> {
    if b.enums.contains_key(name) {
        return Err(CompileError::new(
            "E139",
            line,
            col,
            format!(
                "名字「{}」与枚举重名；枚举名不能用作变量或参数",
                name
            ),
        )
        .with_hint("改一个名字，或把枚举名写成 枚举名.成员 来取值"));
    }
    Ok(())
}

fn store_name(fs: &mut FnState, b: &mut Builder, name: &str, line: u32, col: u32) -> Result<()> {
    reject_enum_name(b, name, line, col)?;
    if native_spec::entry(name).is_some() || vm_spec::reserved(name) {
        return Err(CompileError::new(
            "E115",
            line,
            col,
            format!("原生/任务函数名「{}」是保留名，不能声明或赋值", name),
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
        Res::Global(g) => {
            check_std_import(fs, b, name, line, col)?;
            fs.emit_u8(OP_LOAD_GLOBAL, g);
        }
    }
    Ok(())
}

/// 标准库函数必须先 `导入` 对应模块；用户自己声明的同名全局优先。
fn check_std_import(
    fs: &FnState,
    b: &Builder,
    name: &str,
    line: u32,
    col: u32,
) -> Result<()> {
    let mods = stdlib_spec::modules_for(name);
    if mods.is_empty() || fs.imported.contains(name) || b.declared.contains(name) {
        return Ok(());
    }
    let hint = if mods.len() == 1 {
        format!("先写：导入 {}", mods[0])
    } else {
        format!("先写：导入 {} 或 导入 {}", mods[0], mods[1])
    };
    Err(CompileError::new(
        "E112",
        line,
        col,
        format!(
            "标准库函数「{}」属于模块「{}」，还没有导入",
            name,
            mods.join("、")
        ),
    )
    .with_hint(hint))
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
            StmtKind::Match {
                arms, else_body, ..
            } => {
                for (_, body) in arms {
                    collect_decls(body, out);
                }
                if let Some(body) = else_body {
                    collect_decls(body, out);
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
        ExprKind::SuperCall(_, args) => {
            note_free(out, bound, "本");
            for a in args {
                collect_free_expr(a, bound, out);
            }
        }
        ExprKind::AnonFunc(params, _, _, body) => {
            // 匿名函数体里的自由名字冒泡到本层，供中间层提前建立捕获。
            let mut child_bound: HashSet<String> = params.iter().cloned().collect();
            let mut decls = Vec::new();
            collect_decls(body, &mut decls);
            child_bound.extend(decls);
            collect_free_stmts(body, &child_bound, out);
        }
        ExprKind::Match(subject, arms, else_value) => {
            collect_free_expr(subject, bound, out);
            for (_, v) in arms {
                collect_free_expr(v, bound, out);
            }
            collect_free_expr(else_value, bound, out);
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
            StmtKind::Enum(_) => {}
            StmtKind::Match {
                subject,
                arms,
                else_body,
            } => {
                collect_free_expr(subject, bound, out);
                for (_, body) in arms {
                    collect_free_stmts(body, bound, out);
                }
                if let Some(body) = else_body {
                    collect_free_stmts(body, bound, out);
                }
            }
            StmtKind::Expr(e) => collect_free_expr(e, bound, out),
            StmtKind::Assert(c, m) => {
                collect_free_expr(c, bound, out);
                if let Some(m) = m {
                    collect_free_expr(m, bound, out);
                }
            }
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
    no_assert: bool,
    strict_type: bool,
) -> Result<Program> {
    // 先汇总顶层导入：`导入 模块` 是纯编译期声明，不生成字节码，
    // 但模块里的原生名字会进入本文件的名字作用域（供静态检查）。
    let mut imported: HashSet<String> = HashSet::new();
    for s in stmts {
        if let StmtKind::Import(m) = &s.kind {
            if m == vm_spec::TASK_MODULE {
                for f in vm_spec::TASK_FNS {
                    imported.insert(f.name.to_string());
                }
                continue;
            }
            if let Some(spec) = stdlib_spec::module(m) {
                for f in spec.fns {
                    imported.insert(f.to_string());
                }
                continue;
            }
            let Some(spec) = native_spec::module(m) else {
                return Err(CompileError::new(
                    "E112",
                    s.line,
                    1,
                    format!("不认识的导入模块「{}」", m),
                )
                .with_hint(format!(
                    "v0 可用：{}、{}、{}",
                    native_spec::module_names(),
                    vm_spec::module_names(),
                    stdlib_spec::module_names()
                )));
            };
            for e in spec.entries {
                imported.insert(e.name.to_string());
            }
        }
    }

    let mut b = Builder::new();
    b.no_assert = no_assert;
    b.strict = strict_type;
    b.declared = collect_declared(stmts);
    collect_func_sigs(&mut b, stmts);
    let module_name = b.intern_name("<模块>", 1, 1)?;
    b.codes.push(CodeObj {
        name_idx: module_name,
        nparams: 0,
        nlocals: 0,
        code: Vec::new(),
        caps: Vec::new(),
        file_id: 0,
        marks: Vec::new(),
        locals: Vec::new(),
    });

    // 枚举是纯编译期常量，先在类之前收集好，供类字段与方法体引用。
    collect_enums(&mut b, stmts)?;

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
        ret_ann: None,
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

    b.codes[0].locals = fs.symbols();
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

fn collect_enums(b: &mut Builder, stmts: &[Stmt]) -> Result<()> {
    // 顶层名字：枚举名不能与变量、函数、类重名。
    let mut top: HashSet<String> = HashSet::new();
    for s in stmts {
        match &s.kind {
            StmtKind::Let(n, _) | StmtKind::ForEach(n, ..) => {
                top.insert(n.clone());
            }
            StmtKind::FuncDef { name, .. } => {
                top.insert(name.clone());
            }
            StmtKind::Class(c) => {
                top.insert(c.name.clone());
            }
            _ => {}
        }
    }
    for s in stmts {
        let StmtKind::Enum(e) = &s.kind else {
            continue;
        };
        if native_spec::entry(&e.name).is_some() || vm_spec::reserved(&e.name) {
            return Err(CompileError::new(
                "E139",
                s.line,
                1,
                format!("枚举名「{}」与内建/任务函数重名", e.name),
            ));
        }
        if crate::langcard::BUILTIN_DOCS.iter().any(|d| d.name == e.name) {
            return Err(CompileError::new(
                "E139",
                s.line,
                1,
                format!("枚举名「{}」与内建函数重名", e.name),
            ));
        }
        if top.contains(&e.name) || b.enums.contains_key(&e.name) {
            return Err(CompileError::new(
                "E139",
                s.line,
                1,
                format!("枚举名「{}」与顶层名字（变量/函数/类/枚举）重复", e.name),
            ));
        }
        let mut map: HashMap<String, i32> = HashMap::new();
        // 成员取值：显式值优先，否则上一个取值 +1（首个为 0）。
        let mut by_value: HashMap<i64, String> = HashMap::new();
        let mut next: i64 = 0;
        for m in &e.members {
            let v = m.value.unwrap_or(next);
            if !(-(1i64 << 28)..=(1i64 << 28) - 1).contains(&v) {
                return Err(CompileError::new(
                    "E110",
                    m.line,
                    1,
                    "枚举成员的取值超出 29 位范围（-268435456 .. 268435455）",
                ));
            }
            if map.insert(m.name.clone(), v as i32).is_some() {
                return Err(CompileError::new(
                    "E139",
                    m.line,
                    1,
                    format!("枚举「{}」里重复的成员「{}」", e.name, m.name),
                ));
            }
            if let Some(prev) = by_value.get(&v) {
                return Err(CompileError::new(
                    "E139",
                    m.line,
                    1,
                    format!(
                        "枚举「{}」里「{}」与「{}」的取值都是 {}；取值不能重复",
                        e.name, prev, m.name, v
                    ),
                ));
            }
            by_value.insert(v, m.name.clone());
            next = v + 1;
        }
        b.enums.insert(e.name.clone(), map);
    }
    Ok(())
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
    let mut decl_lines: Vec<u32> = Vec::new();
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
                parent: 255,
                is_final: c.is_final,
                fields: Vec::new(),
                methods: Vec::new(),
            });
            decls.push((c, file_at(spans, si)));
            decl_lines.push(s.line);
        }
    }

    // 父类解析：按名字查类表，父类可以写在子类之后。
    let mut parents: Vec<u8> = vec![255; decls.len()];
    for (ci, (c, _)) in decls.iter().enumerate() {
        let Some(p) = &c.parent else { continue };
        let Some(&pi) = b.class_map.get(p) else {
            return Err(CompileError::new(
                "E140",
                decl_lines[ci],
                1,
                format!("父类「{}」没有定义（继承只支持同一模块里的类）", p),
            ));
        };
        if pi as usize == ci {
            return Err(CompileError::new(
                "E140",
                decl_lines[ci],
                1,
                format!("类「{}」不能继承自己", c.name),
            ));
        }
        if b.classes[pi as usize].is_final {
            return Err(CompileError::new(
                "E142",
                decl_lines[ci],
                1,
                format!("类「{}」是最终类，不能继承", decls[pi as usize].0.name),
            ));
        }
        // LI012 没有「超」，父构造只能零实参自动调用。
        if let Some(m) = decls[pi as usize]
            .0
            .methods
            .iter()
            .find(|m| m.name == "初始化")
        {
            if !m.params.is_empty() {
                return Err(CompileError::new(
                    "E140",
                    decl_lines[ci],
                    1,
                    format!(
                        "父类「{}」的初始化带参数，LI012 不支持自动传参（等「超」）",
                        decls[pi as usize].0.name
                    ),
                ));
            }
        }
        parents[ci] = pi;
        b.classes[ci].parent = pi;
    }
    // 防环：沿父链最多走 nclasses 步。
    for ci in 0..decls.len() {
        let mut k = parents[ci];
        let mut hops = 0usize;
        while k != 255 {
            hops += 1;
            if hops > decls.len() {
                return Err(CompileError::new(
                    "E140",
                    decl_lines[ci],
                    1,
                    format!("类「{}」的继承关系成环", decls[ci].0.name),
                ));
            }
            k = parents[k as usize];
        }
    }

    // 第二遍：类名 / 字段名 / 方法名进全局名字表，做类内唯一性检查。
    let mut own_fields: Vec<Vec<u8>> = Vec::new();
    let mut own_method_names: Vec<Vec<(String, bool)>> = Vec::new();
    for (ci, (c, _)) in decls.iter().enumerate() {
        let name_idx = b.intern_name(&c.name, 1, 1)?;
        b.classes[ci].name_idx = name_idx;
        for m in &c.methods {
            let sig = FuncSig {
                param_types: m.param_types.clone(),
                ret_type: m.ret_type.clone(),
            };
            if m.name == "初始化" {
                b.init_sigs.insert(ci as u8, sig);
            } else {
                b.method_sigs.insert((ci as u8, m.name.clone()), sig);
            }
        }
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
        own_fields.push(fields);
        let mut mnames = Vec::new();
        for m in &c.methods {
            if m.name == "初始化" {
                if m.is_final {
                    return Err(CompileError::new(
                        "E142",
                        m.line,
                        1,
                        "「初始化」不能标记为最终（构造走构造链，不参与覆写）",
                    ));
                }
                continue;
            }
            b.intern_name(&m.name, m.line, 1)?;
            mnames.push((m.name.clone(), m.is_final));
        }
        own_method_names.push(mnames);
    }

    // 第三遍：按继承深度升序扁平化字段（父类字段在前），并做跨类成员冲突检查。
    let mut depth = vec![0usize; decls.len()];
    for (ci, d) in depth.iter_mut().enumerate() {
        let mut k = parents[ci];
        while k != 255 {
            *d += 1;
            k = parents[k as usize];
        }
    }
    let mut order: Vec<usize> = (0..decls.len()).collect();
    order.sort_by_key(|&ci| depth[ci]);
    for &ci in &order {
        let (c, _) = decls[ci];
        let mut flat: Vec<u8> = if parents[ci] == 255 {
            Vec::new()
        } else {
            b.classes[parents[ci] as usize].fields.clone()
        };
        let ancestor_fields: Vec<String> = flat
            .iter()
            .map(|&ni| b.names[ni as usize].clone())
            .collect();
        let mut ancestor_methods: HashSet<String> = HashSet::new();
        let mut ancestor_finals: HashSet<String> = HashSet::new();
        let mut k = parents[ci];
        while k != 255 {
            for (n, fin) in &own_method_names[k as usize] {
                ancestor_methods.insert(n.clone());
                if *fin {
                    ancestor_finals.insert(n.clone());
                }
            }
            k = parents[k as usize];
        }
        for (i, (fname, fexpr)) in c.fields.iter().enumerate() {
            if ancestor_fields.iter().any(|n| n == fname) {
                return Err(CompileError::new(
                    "E140",
                    fexpr.line,
                    fexpr.col,
                    format!("子类不能重定义父类字段「{}」", fname),
                ));
            }
            if ancestor_methods.contains(fname) {
                return Err(CompileError::new(
                    "E140",
                    fexpr.line,
                    fexpr.col,
                    format!("「{}」在父类里是方法，子类不能当字段", fname),
                ));
            }
            flat.push(own_fields[ci][i]);
        }
        for (mname, _) in &own_method_names[ci] {
            if ancestor_fields.iter().any(|n| n == mname) {
                return Err(CompileError::new(
                    "E140",
                    decl_lines[ci],
                    1,
                    format!("「{}」在父类里是字段，子类不能当方法", mname),
                ));
            }
            if ancestor_finals.contains(mname) {
                return Err(CompileError::new(
                    "E142",
                    decl_lines[ci],
                    1,
                    format!("父类的最终方法「{}」不能被覆写", mname),
                ));
            }
        }
        if flat.len() > 255 {
            return Err(CompileError::new(
                "E105",
                decl_lines[ci],
                1,
                "类的字段超过 255 个（含继承）",
            ));
        }
        b.classes[ci].fields = flat;
    }

    // 第四遍：先给所有类登记方法名（含父类声明在后面的情况），再编译方法体。
    for (ci, (c, _)) in decls.iter().enumerate() {
        for m in &c.methods {
            if m.name == "初始化" {
                continue; // 构造方法只由合成的初始化代码真正执行
            }
            let midx = b.intern_name(&m.name, m.line, 1)?;
            b.classes[ci].methods.push((midx, 0));
        }
    }
    for &ci in &order {
        let (c, fid) = decls[ci];
        let ci = ci as u8;
        for (k, m) in c.methods.iter().filter(|m| m.name != "初始化").enumerate() {
            let midx = b.classes[ci as usize].methods[k].0;
            let code = compile_method(
                b,
                ci,
                midx,
                &m.params,
                &m.param_types,
                m.ret_type.as_deref(),
                &m.body,
                imported,
                m.line,
                fid,
            )?;
            b.classes[ci as usize].methods[k].1 = code;
        }
        let init_code = compile_class_init(b, c, ci, imported, fid)?;
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
    param_types: &[Option<String>],
    ret_type: Option<&str>,
    body: &[Stmt],
    imported: &HashSet<String>,
    line: u32,
    file_id: u8,
) -> Result<u8> {
    if b.strict {
        for t in param_types.iter().flatten() {
            check_ann(b, t, line, 1)?;
        }
        if let Some(t) = ret_type {
            check_ann(b, t, line, 1)?;
        }
    }
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
        locals: Vec::new(),
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
        ret_ann: ret_type.map(|s| s.to_string()),
        file_id,
        marks: Vec::new(),
    };
    fs.mark(line);
    fs.slots.insert("本".to_string(), 0);

    for (i, p) in params.iter().enumerate() {
        reject_enum_name(b, p, line, 1)?;
        if native_spec::entry(p).is_some() || vm_spec::reserved(p) {
            return Err(CompileError::new(
                "E115",
                line,
                1,
                format!("原生/任务函数名「{}」是保留名，不能当参数", p),
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

    b.codes[idx as usize].locals = fs.symbols();
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
    if b.strict {
        if let Some(u) = user {
            for t in u.param_types.iter().flatten() {
                check_ann(b, t, u.line, 1)?;
            }
            if let Some(t) = &u.ret_type {
                check_ann(b, t, u.line, 1)?;
            }
        }
    }

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
        locals: Vec::new(),
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
        ret_ann: None,
        file_id,
        marks: Vec::new(),
    };
    fs.mark(user.map(|m| m.line).unwrap_or(1));
    fs.slots.insert("本".to_string(), 0);

    for (i, p) in params.iter().enumerate() {
        reject_enum_name(b, p, user.map(|m| m.line).unwrap_or(1), 1)?;
        if native_spec::entry(p).is_some() || vm_spec::reserved(p) {
            return Err(CompileError::new(
                "E115",
                user.map(|m| m.line).unwrap_or(1),
                1,
                format!("原生/任务函数名「{}」是保留名，不能当参数", p),
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

    // 构造链：先让父类初始化跑一遍（LI012 只支持零实参父构造）。
    let parent = b.classes[class_idx as usize].parent;
    if parent != 255 {
        fs.emit_u8(OP_LOAD_LOCAL, 0);
        fs.emit_u8u8(OP_CALL_INIT, parent, 0);
        fs.emit(OP_POP);
    }

    // 字段默认值：按声明顺序求值，禁止引用 `本`。
    // 字段数组是「父类扁平字段 ++ 本类字段」，本类字段从 base 开始。
    let base = b.classes[class_idx as usize].fields.len() - c.fields.len();
    for (i, (_, fexpr)) in c.fields.iter().enumerate() {
        fs.emit_u8(OP_LOAD_LOCAL, 0);
        fs.self_blocked = true;
        compile_expr(&mut fs, b, fexpr)?;
        fs.self_blocked = false;
        let fidx = b.classes[class_idx as usize].fields[base + i];
        fs.emit_u8(OP_SET_FIELD, fidx);
    }

    if let Some(u) = user {
        compile_stmts(&mut fs, b, &u.body)?;
    }

    fs.emit_u8(OP_LOAD_LOCAL, 0);
    fs.emit(OP_RETURN_METHOD);

    b.codes[idx as usize].locals = fs.symbols();
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
    self_name: Option<&str>,
    params: &[String],
    param_types: &[Option<String>],
    ret_type: Option<&str>,
    body: &[Stmt],
    parent_slots: HashMap<String, u8>,
    imported: &HashSet<String>,
    current_class: Option<u8>,
    line: u32,
    col: u32,
    file_id: u8,
) -> Result<u8> {
    if b.strict {
        for t in param_types.iter().flatten() {
            check_ann(b, t, line, col)?;
        }
        if let Some(t) = ret_type {
            check_ann(b, t, line, col)?;
        }
    }
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
        locals: Vec::new(),
    });

    let mut fs = FnState {
        code: Vec::new(),
        slots: HashMap::new(),
        captures: HashMap::new(),
        parent_slots,
        caps: Vec::new(),
        self_name: self_name.map(|s| s.to_string()),
        nlocals: params.len() as u8,
        temps: Vec::new(),
        loops: Vec::new(),
        try_stack: Vec::new(),
        is_module: false,
        imported: imported.clone(),
        current_class,
        self_blocked: false,
        in_init: false,
        ret_ann: ret_type.map(|s| s.to_string()),
        file_id,
        marks: Vec::new(),
    };
    fs.mark(line);

    for (i, p) in params.iter().enumerate() {
        reject_enum_name(b, p, line, col)?;
        if native_spec::entry(p).is_some() || vm_spec::reserved(p) {
            return Err(CompileError::new(
                "E115",
                line,
                col,
                format!("原生/任务函数名「{}」是保留名，不能当参数", p),
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

    b.codes[idx as usize].locals = fs.symbols();
    b.codes[idx as usize].nlocals = fs.nlocals;
    b.codes[idx as usize].caps = fs.caps;
    b.codes[idx as usize].marks = fs.marks;
    b.codes[idx as usize].code = fs.code;
    Ok(idx)
}

/// 在当前位置生成一个函数值（具名函数定义与匿名函数表达式共用）。
///
/// `name` 是名字表/调试里显示的名字；`self_name` 是函数体内可以递归引用的名字
/// （匿名函数为 `None`）。产物是一个函数/闭包值，留在操作数栈上。
#[allow(clippy::too_many_arguments)]
fn emit_function_value(
    fs: &mut FnState,
    b: &mut Builder,
    name: &str,
    self_name: Option<&str>,
    params: &[String],
    param_types: &[Option<String>],
    ret_type: Option<&str>,
    body: &[Stmt],
    line: u32,
    col: u32,
) -> Result<()> {
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
            if let Res::Capture(slot) = resolve(fs, b, &n, line, col)? {
                visible.insert(n, slot);
            }
        }
    }
    let idx = compile_function(
        b,
        name,
        self_name,
        params,
        param_types,
        ret_type,
        body,
        visible,
        &fs.imported,
        fs.current_class,
        line,
        col,
        fs.file_id,
    )?;
    if b.codes[idx as usize].caps.is_empty() {
        fs.emit_u8(OP_MAKE_FUNCTION, idx);
    } else {
        fs.emit_u8(OP_MAKE_CLOSURE, idx);
    }
    Ok(())
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
        StmtKind::Enum(_) => {
            if !fs.is_module {
                return Err(CompileError::new(
                    "E139",
                    s.line,
                    1,
                    "「枚举」只能写在模块顶层",
                ));
            }
            // 顶层枚举已由 collect_enums 收集，这里不生成模块字节码。
        }
        StmtKind::Match {
            subject,
            arms,
            else_body,
        } => {
            compile_match(fs, b, subject, arms, else_body.as_deref(), s.line)?;
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
        StmtKind::Assert(cond, msg) => {
            // `--no-assert` 时整句消除：条件和消息都不求值。
            if !b.no_assert {
                compile_expr(fs, b, cond)?;
                let jf = fs.jump(OP_JUMP_IF_FALSE);
                let jend = fs.jump(OP_JUMP);
                let fail = fs.here();
                fs.patch(jf, fail);
                match msg {
                    Some(m) => compile_expr(fs, b, m)?,
                    None => {
                        let c = b.const_str("断言失败", s.line, 1)?;
                        fs.emit_u8(OP_CONST, c);
                    }
                }
                fs.emit(OP_THROW);
                let end = fs.here();
                fs.patch(jend, end);
            }
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
        StmtKind::FuncDef {
            name,
            params,
            param_types,
            ret_type,
            body,
            ..
        } => {
            emit_function_value(
                fs,
                b,
                name,
                Some(name),
                params,
                param_types,
                ret_type.as_deref(),
                body,
                s.line,
                1,
            )?;
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
            if b.strict {
                if let Some(ann) = fs.ret_ann.clone() {
                    let expected = resolve_ty(b, &ann).unwrap();
                    let actual = match &e {
                        Some(x) => obvious_ty(b, fs, x),
                        None => Some(Ty::Nil),
                    };
                    if let Some(a) = actual {
                        if !assignable(b, a, expected) {
                            return Err(CompileError::new(
                                "E143",
                                s.line,
                                e.as_ref().map(|x| x.col).unwrap_or(1),
                                format!("返回值标注为「{}」，实际是「{}」", ann, ty_name(b, a)),
                            ));
                        }
                    }
                }
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

fn compile_match(
    fs: &mut FnState,
    b: &mut Builder,
    subject: &Expr,
    arms: &[(Vec<MatchPattern>, Vec<Stmt>)],
    else_body: Option<&[Stmt]>,
    line: u32,
) -> Result<()> {
    // 先把分支模式解析成整数区间，顺带检查未知枚举成员与重复/重叠。
    let resolved = resolve_match_arms(b, arms.iter().map(|(p, _)| p.as_slice()), line)?;

    // 主体只求值一次，放进临时槽，再逐支比较。
    compile_expr(fs, b, subject)?;
    let t = fs.alloc_temp(line, 1)?;
    fs.emit_u8(OP_STORE_LOCAL, t);
    let mut ends: Vec<usize> = Vec::new();
    for ((_, body), pats) in arms.iter().zip(resolved.iter()) {
        let mut hits: Vec<usize> = Vec::new();
        emit_match_test(fs, b, t, pats, line, &mut hits)?;
        let jskip = fs.jump(OP_JUMP);
        let body_start = fs.here();
        for j in hits {
            fs.patch(j, body_start);
        }
        compile_stmts(fs, b, body)?;
        ends.push(fs.jump(OP_JUMP));
        let next = fs.here();
        fs.patch(jskip, next);
    }
    if let Some(body) = else_body {
        compile_stmts(fs, b, body)?;
    }
    let end = fs.here();
    for p in ends {
        fs.patch(p, end);
    }
    fs.free_temp(t);
    Ok(())
}

/// 把每个分支的模式解析成闭区间 `[起, 止]`，并检查未知枚举成员与区间重叠。
fn resolve_match_arms<'a>(
    b: &Builder,
    patterns: impl Iterator<Item = &'a [MatchPattern]>,
    line: u32,
) -> Result<Vec<Vec<(i64, i64)>>> {
    let mut out: Vec<Vec<(i64, i64)>> = Vec::new();
    let mut seen: Vec<(i64, i64)> = Vec::new();
    for pats in patterns {
        let mut cur: Vec<(i64, i64)> = Vec::new();
        for p in pats {
            let (lo, hi) = match p {
                MatchPattern::Int(v) => (*v, *v),
                MatchPattern::Range(a, z) => (*a, *z),
                MatchPattern::EnumMember(en, member) => {
                    let Some(map) = b.enums.get(en) else {
                        return Err(CompileError::new(
                            "E139",
                            line,
                            1,
                            format!("「情形」里的「{}」不是枚举", en),
                        ));
                    };
                    match map.get(member) {
                        Some(&v) => (v as i64, v as i64),
                        None => {
                            return Err(CompileError::new(
                                "E139",
                                line,
                                1,
                                format!("枚举「{}」里没有成员「{}」", en, member),
                            ))
                        }
                    }
                }
                MatchPattern::Else => {
                    return Err(CompileError::new(
                        "E139",
                        line,
                        1,
                        "「其他」不能出现在普通分支里",
                    ))
                }
            };
            if seen.iter().any(|&(a, z)| lo <= z && a <= hi) {
                return Err(CompileError::new(
                    "E139",
                    line,
                    1,
                    format!(
                        "「匹配」里的情形值与之前的分支重复或区间重叠（{}..{}）",
                        lo, hi
                    ),
                ));
            }
            seen.push((lo, hi));
            cur.push((lo, hi));
        }
        out.push(cur);
    }
    Ok(out)
}

/// 为一条分支发出比较链：任一模式命中就跳到分支体。
fn emit_match_test(
    fs: &mut FnState,
    b: &mut Builder,
    slot: u8,
    pats: &[(i64, i64)],
    line: u32,
    hits: &mut Vec<usize>,
) -> Result<()> {
    for &(lo, hi) in pats {
        fs.emit_u8(OP_LOAD_LOCAL, slot);
        let c = b.const_int(lo, line, 1)?;
        fs.emit_u8(OP_CONST, c);
        if lo == hi {
            fs.emit_u8(OP_CMP, CMP_EQ);
            hits.push(fs.jump(OP_JUMP_IF_TRUE));
        } else {
            // 区间：`起 <= 值 <= 止`，两次比较，不新增指令。
            fs.emit_u8(OP_CMP, CMP_GE);
            let jf = fs.jump(OP_JUMP_IF_FALSE);
            fs.emit_u8(OP_LOAD_LOCAL, slot);
            let c2 = b.const_int(hi, line, 1)?;
            fs.emit_u8(OP_CONST, c2);
            fs.emit_u8(OP_CMP, CMP_LE);
            hits.push(fs.jump(OP_JUMP_IF_TRUE));
            let skip = fs.here();
            fs.patch(jf, skip);
        }
    }
    Ok(())
}

/// 匹配表达式里能静态看出的字面量种类（非字面量返回 `None`）。
fn literal_kind(e: &Expr) -> Option<&'static str> {
    match &e.kind {
        ExprKind::Int(_) => Some("整数"),
        ExprKind::Dec(_) => Some("小数"),
        ExprKind::Str(_) => Some("文本"),
        ExprKind::True | ExprKind::False => Some("布尔"),
        ExprKind::Nil => Some("空"),
        _ => None,
    }
}

/// 表达式形式的匹配：各分支（含兜底）的字面量结果必须同类型。
fn check_match_value_kinds(
    arms: &[(Vec<MatchPattern>, Expr)],
    else_value: &Expr,
    line: u32,
    col: u32,
) -> Result<()> {
    let mut first: Option<(&'static str, u32)> = None;
    let mut note = |kind: &'static str, at: u32| -> Result<()> {
        match first {
            None => {
                first = Some((kind, at));
                Ok(())
            }
            Some((f, _)) if f == kind => Ok(()),
            Some((f, _)) => Err(CompileError::new(
                "E139",
                at,
                col,
                format!(
                    "匹配分支的结果类型不一致：{} 与 {}；表达式形式要求各分支同类型（第 {} 行已经出现 {}）",
                    f, kind, line, f
                ),
            )),
        }
    };
    for (_, v) in arms {
        if let Some(k) = literal_kind(v) {
            note(k, v.line)?;
        }
    }
    if let Some(k) = literal_kind(else_value) {
        note(k, else_value.line)?;
    }
    Ok(())
}

/// 匹配表达式：主体求值一次，命中分支的值放进结果槽，最后压栈。
fn compile_match_value(
    fs: &mut FnState,
    b: &mut Builder,
    subject: &Expr,
    arms: &[(Vec<MatchPattern>, Expr)],
    else_value: &Expr,
    line: u32,
    col: u32,
) -> Result<()> {
    check_match_value_kinds(arms, else_value, line, col)?;
    let resolved = resolve_match_arms(b, arms.iter().map(|(p, _)| p.as_slice()), line)?;
    compile_expr(fs, b, subject)?;
    let t = fs.alloc_temp(line, col)?;
    fs.emit_u8(OP_STORE_LOCAL, t);
    let r = fs.alloc_temp(line, col)?;
    let mut ends: Vec<usize> = Vec::new();
    for ((_, value), pats) in arms.iter().zip(resolved.iter()) {
        let mut hits: Vec<usize> = Vec::new();
        emit_match_test(fs, b, t, pats, line, &mut hits)?;
        let jskip = fs.jump(OP_JUMP);
        let body_start = fs.here();
        for j in hits {
            fs.patch(j, body_start);
        }
        compile_expr(fs, b, value)?;
        fs.emit_u8(OP_STORE_LOCAL, r);
        ends.push(fs.jump(OP_JUMP));
        let next = fs.here();
        fs.patch(jskip, next);
    }
    compile_expr(fs, b, else_value)?;
    fs.emit_u8(OP_STORE_LOCAL, r);
    let end = fs.here();
    for p in ends {
        fs.patch(p, end);
    }
    fs.emit_u8(OP_LOAD_LOCAL, r);
    fs.free_temp(r);
    fs.free_temp(t);
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
        BinOp::BitAnd => BIN_BAND,
        BinOp::BitOr => BIN_BOR,
        BinOp::BitXor => BIN_BXOR,
        BinOp::Shl => BIN_SHL,
        BinOp::Shr => BIN_SHR,
        BinOp::And | BinOp::Or => unreachable!("短路运算单独生成"),
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
            if b.enums.contains_key(name) {
                return Err(CompileError::new(
                    "E139",
                    e.line,
                    e.col,
                    format!("枚举名「{}」不能当值使用，请写 {}.成员", name, name),
                ));
            }
            if vm_spec::reserved(name) {
                if fs.imported.contains(name) {
                    return Err(CompileError::new(
                        "E115",
                        e.line,
                        e.col,
                        format!("任务函数「{}」只能直接调用，不能当值使用", name),
                    ));
                }
                return Err(CompileError::new(
                    "E113",
                    e.line,
                    e.col,
                    format!(
                        "任务函数「{}」属于模块「{}」，还没有导入",
                        name,
                        vm_spec::TASK_MODULE
                    ),
                )
                .with_hint(format!("先写：导入 {}", vm_spec::TASK_MODULE)));
            }
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
        ExprKind::Match(subject, arms, else_value) => {
            compile_match_value(fs, b, subject, arms, else_value, e.line, e.col)?;
        }
        ExprKind::AnonFunc(params, param_types, ret_type, body) => {
            // 匿名函数没有可递归引用的名字，所以 self_name 传 None。
            let name = format!("匿名函数{}", e.line);
            emit_function_value(
                fs,
                b,
                &name,
                None,
                params,
                param_types,
                ret_type.as_deref(),
                body,
                e.line,
                e.col,
            )?;
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
        ExprKind::Unary(UnOp::Bnot, x) => {
            // `~x` 降级为 `x ^ -1`：32 位补码下与按位取反等价，不新增操作码。
            compile_expr(fs, b, x)?;
            let c = b.const_int(-1, e.line, e.col)?;
            fs.emit_u8(OP_CONST, c);
            fs.emit_u8(OP_BINOP, BIN_BXOR);
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
            if b.strict {
                if let ExprKind::Name(n) = &callee.kind {
                    if let Some(sig) = b.func_sigs.get(n).cloned() {
                        check_call_args(&*b, fs, &sig, args, e.line)?;
                    }
                }
            }
            match &callee.kind {
                ExprKind::Name(name) => {
                    if let Some(tf) = vm_spec::task_entry(name) {
                        if !fs.imported.contains(name) {
                            return Err(CompileError::new(
                                "E113",
                                callee.line,
                                callee.col,
                                format!(
                                    "任务函数「{}」属于模块「{}」，还没有导入",
                                    name, vm_spec::TASK_MODULE
                                ),
                            )
                            .with_hint(format!("先写：导入 {}", vm_spec::TASK_MODULE)));
                        }
                        if args.len() != tf.nargs as usize {
                            return Err(CompileError::new(
                                "E115",
                                callee.line,
                                callee.col,
                                format!(
                                    "任务函数「{}」需要 {} 个参数，这里给了 {} 个",
                                    name, tf.nargs, args.len()
                                ),
                            ));
                        }
                        for a in args {
                            compile_expr(fs, b, a)?;
                        }
                        fs.emit(tf.opcode);
                        return Ok(());
                    }
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
                        // 带超时的队列/信号量等待：降到 OP_TASK_WAIT，由 VM 挂起重试。
                        if let Some(wf) = vm_spec::wait_entry(name) {
                            if args.len() == wf.nargs as usize {
                                for a in args {
                                    compile_expr(fs, b, a)?;
                                }
                                fs.emit_u8(OP_TASK_WAIT, wf.kind);
                                return Ok(());
                            }
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
            if b.strict {
                if let Some(sig) = b.init_sigs.get(&ci).cloned() {
                    check_call_args(&*b, fs, &sig, args, e.line)?;
                }
            }
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
            // `枚举名.成员`：编译期直接折成整数常量，不产生对象访问。
            let enum_hit = match &obj.kind {
                ExprKind::Name(en) if b.enums.contains_key(en) => {
                    Some((en.clone(), b.enums[en].get(name).copied()))
                }
                _ => None,
            };
            if let Some((en, found)) = enum_hit {
                let Some(v) = found else {
                    return Err(CompileError::new(
                        "E139",
                        e.line,
                        e.col,
                        format!("枚举「{}」里没有成员「{}」", en, name),
                    ));
                };
                let c = b.const_int(v as i64, e.line, e.col)?;
                fs.emit_u8(OP_CONST, c);
                return Ok(());
            }
            let ni = field_name_idx(fs, b, obj, name, e.line, e.col)?;
            compile_expr(fs, b, obj)?;
            fs.emit_u8(OP_GET_FIELD, ni);
        }
        ExprKind::MethodCall(obj, name, args) => {
            if b.strict {
                if let (ExprKind::SelfRef, Some(ci)) = (&obj.kind, fs.current_class) {
                    if let Some(sig) = method_sig(b, ci, name).cloned() {
                        check_call_args(&*b, fs, &sig, args, e.line)?;
                    }
                }
            }
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
        ExprKind::SuperCall(name, args) => {
            let Some(ci) = fs.current_class else {
                return Err(CompileError::new(
                    "E141",
                    e.line,
                    e.col,
                    "「超」只能在类方法（或其中的闭包）里使用",
                ));
            };
            let cname = b.names[b.classes[ci as usize].name_idx as usize].clone();
            if name == "初始化" {
                return Err(CompileError::new(
                    "E141",
                    e.line,
                    e.col,
                    "父构造已自动执行，不能写 超.初始化(...)",
                ));
            }
            let parent = b.classes[ci as usize].parent;
            if parent == 255 {
                return Err(CompileError::new(
                    "E141",
                    e.line,
                    e.col,
                    format!("类「{}」没有父类，不能用「超」", cname),
                ));
            }
            let Some(code) = b.class_method_code_idx(parent, name) else {
                return Err(CompileError::new(
                    "E141",
                    e.line,
                    e.col,
                    format!("父类里没有方法「{}」", name),
                ));
            };
            if b.strict {
                if let Some(sig) = method_sig(b, parent, name).cloned() {
                    check_call_args(&*b, fs, &sig, args, e.line)?;
                }
            }
            load_self(fs, e.line, e.col)?;
            for a in args {
                compile_expr(fs, b, a)?;
            }
            if args.len() > 255 {
                return Err(CompileError::new("E114", e.line, e.col, "实参超过 255 个"));
            }
            fs.emit_u8u8(OP_CALL_SUPER, code, args.len() as u8);
        }
        ExprKind::Index(obj, idx) => {
            compile_expr(fs, b, obj)?;
            compile_expr(fs, b, idx)?;
            fs.emit(OP_GET_ITEM);
        }
    }
    Ok(())
}

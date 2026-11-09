//! H1 语言卡生成器。
//!
//! 语言卡不是手写的第二份规范，而是从规范来源渲染出来：
//! - 操作码 / 运算子码 / 比较子码 → `opcodes.rs`
//! - 关键字与全角归一表 → `lexer.rs`
//! - 内建函数名与 id → `runtime/src/jm_builtin.c` 的 `BUILTINS[]`
//! - 编译期错误码 → `error.rs` 的 `COMPILE_CODES`
//! - 运行期错误码 → `runtime/src/jm_internal.h` 的 `#define E_*`
//!
//! `micc --check-langcard` 会重新渲染并比对入库文件，任何一侧改动而卡没更新都会失败。

use crate::lexer;
use crate::native_spec;
use crate::opcodes::{self, Operand};
use crate::vm_spec;
use crate::writer::{MAGIC, VERSION};
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "jishi.langcard/1";
pub const LANG_VERSION: &str = "Micro Core v0";

pub struct BuiltinDoc {
    pub name: &'static str,
    pub min_args: u8,
    pub max_args: u8,
    pub doc: &'static str,
}

/// 内建函数的文档元数据（参数个数与用途）。
/// 名字与 id 不在本表里——它们来自 C 侧 `BUILTINS[]`，本表只按名字挂说明。
pub const BUILTIN_DOCS: &[BuiltinDoc] = &[
    BuiltinDoc {
        name: "打印",
        min_args: 0,
        max_args: 255,
        doc: "把参数按显示形式输出，空格分隔，末尾换行；无返回值（空）。",
    },
    BuiltinDoc {
        name: "文本",
        min_args: 1,
        max_args: 1,
        doc: "把任意值转成文本；文本原样返回。",
    },
    BuiltinDoc {
        name: "整数",
        min_args: 1,
        max_args: 1,
        doc: "整数原样返回；小数向零截断；数字文本按十进制解析；其他值报类型错误。",
    },
    BuiltinDoc {
        name: "小数",
        min_args: 1,
        max_args: 1,
        doc: "整数或小数转成 Q12 定点小数；超出 ±524288 报溢出。",
    },
    BuiltinDoc {
        name: "长度",
        min_args: 1,
        max_args: 1,
        doc: "文本按 UTF-8 字符数、列表按元素数、字典按键数、范围按元素数。",
    },
    BuiltinDoc {
        name: "范围",
        min_args: 1,
        max_args: 3,
        doc: "范围(止) / 范围(起, 止) / 范围(起, 止, 步长)，左闭右开，步长不能为 0。",
    },
    BuiltinDoc {
        name: "类型",
        min_args: 1,
        max_args: 1,
        doc: "返回类型名文本：整数 / 小数 / 文本 / 列表 / 字典 / 范围 / 函数 / 内建 / 实例 / 空 / 真 / 假 / 未定义。",
    },
    BuiltinDoc {
        name: "总和",
        min_args: 1,
        max_args: 1,
        doc: "对列表或范围求和，只接受整数元素。",
    },
    BuiltinDoc {
        name: "最大",
        min_args: 1,
        max_args: 1,
        doc: "列表或范围内的最大整数。",
    },
    BuiltinDoc {
        name: "最小",
        min_args: 1,
        max_args: 1,
        doc: "列表或范围内的最小整数。",
    },
    BuiltinDoc {
        name: "绝对值",
        min_args: 1,
        max_args: 1,
        doc: "整数或小数的绝对值；超出各自范围报溢出。",
    },
];

pub struct RuntimeErrorDoc {
    pub name: &'static str,
    pub meaning: &'static str,
}

pub const RUNTIME_ERROR_DOCS: &[RuntimeErrorDoc] = &[
    RuntimeErrorDoc { name: "E_STACK", meaning: "值栈溢出（调用层数或表达式嵌套太深）" },
    RuntimeErrorDoc { name: "E_UNDERFLOW", meaning: "值栈下溢（字节码不平衡，通常由编译器 bug 引起）" },
    RuntimeErrorDoc { name: "E_CONST", meaning: "常量下标越界或常量种类未知" },
    RuntimeErrorDoc { name: "E_NAME", meaning: "读到了未定义的名字" },
    RuntimeErrorDoc { name: "E_NAMES", meaning: "名字表越界" },
    RuntimeErrorDoc { name: "E_LOCAL", meaning: "局部槽越界或读取未初始化的局部变量" },
    RuntimeErrorDoc { name: "E_ZERO_DIV", meaning: "除以零" },
    RuntimeErrorDoc { name: "E_ZERO_MOD", meaning: "取模零" },
    RuntimeErrorDoc { name: "E_OVERFLOW", meaning: "整数运算溢出 29 位范围" },
    RuntimeErrorDoc { name: "E_TYPE", meaning: "类型不支持该运算/比较/调用" },
    RuntimeErrorDoc { name: "E_NOT_CALLABLE", meaning: "这个值不能调用" },
    RuntimeErrorDoc { name: "E_ARGC", meaning: "实参个数与形参不符" },
    RuntimeErrorDoc { name: "E_INDEX_TYPE", meaning: "下标类型不对（如用文本下标列表）" },
    RuntimeErrorDoc { name: "E_INDEX", meaning: "下标越界或字典里没有这个键" },
    RuntimeErrorDoc { name: "E_NOT_ITER", meaning: "这个值不能遍历" },
    RuntimeErrorDoc { name: "E_ITER", meaning: "迭代器状态错误" },
    RuntimeErrorDoc { name: "E_OOM", meaning: "运行时堆不足" },
    RuntimeErrorDoc { name: "E_OBJ", meaning: "对象错误（嵌套太深等）" },
    RuntimeErrorDoc { name: "E_KEY", meaning: "字典键必须是不可变值（整数/文本/空/真/假）" },
    RuntimeErrorDoc { name: "E_CODE", meaning: "函数代码对象越界" },
    RuntimeErrorDoc { name: "E_FRAMES", meaning: "调用层数太深（上限 32）" },
    RuntimeErrorDoc { name: "E_OPCODE", meaning: "未知指令或指令流越界" },
    RuntimeErrorDoc { name: "E_DIV_NONINT", meaning: "整数真除法结果不是整数；改用 //，或让任一操作数成为小数（如 3300.0）" },
    RuntimeErrorDoc { name: "E_POW", meaning: "幂指数不能是负数" },
    RuntimeErrorDoc { name: "E_IMPORT", meaning: "导入指令保留（编译期导入不生成该指令）" },
    RuntimeErrorDoc { name: "E_FILE", meaning: "字节码文件损坏或超出边界" },
    RuntimeErrorDoc { name: "E_UNCAUGHT", meaning: "未捕获的异常：`抛出` 的值一路传到了最外层（`错误[E027] 未捕获的异常：<值>`）" },
    RuntimeErrorDoc { name: "E_TRY_DEPTH", meaning: "`尝试` 嵌套超过当前任务上限（根任务 8 层、子任务 4 层）" },
    RuntimeErrorDoc { name: "E_TRY_STATE", meaning: "`尝试` 栈与当前帧不匹配（字节码不平衡，通常由编译器 bug 引起）" },
    RuntimeErrorDoc { name: "E_FIELD", meaning: "实例没有这个字段（E035）" },
    RuntimeErrorDoc { name: "E_NOT_INSTANCE", meaning: "对非实例取字段或调方法（E036）" },
    RuntimeErrorDoc { name: "E_METHOD", meaning: "实例没有这个方法（E037）" },
    RuntimeErrorDoc { name: "E_TASK_LIMIT", meaning: "子任务数超过上限（根任务 + 最多 3 个子任务）" },
    RuntimeErrorDoc { name: "E_TASK_STATE", meaning: "任务状态不允许此操作，或没有可运行的任务（调度器内部错误）" },
];

/// `任务` 模块函数的说明（名字必须与 vm_spec::TASK_FNS 对齐）。
const TASK_FN_DOCS: &[(&str, &str)] = &[
    ("创建", "创建一个无参子任务，返回任务句柄；最多 3 个子任务"),
    ("删除", "删除(句柄)：取消一个未在运行的任务（就绪/延时/等待中），释放它的值栈与任务体并返回 真；句柄无效、已结束或指向当前任务返回 假。槽位暂不复用。"),
    ("让出", "主动让出，交给下一个就绪任务；没有就绪任务时立即返回"),
    ("延时", "睡够指定毫秒（0..60000）再继续，不占用 VM"),
];

/// 带超时等待函数的说明（名字必须与 vm_spec::WAIT_FNS 对齐）。
const WAIT_FN_DOCS: &[(&str, &str)] = &[
    ("队列接收", "带超时：队列空就挂起，到期返回 空"),
    ("队列发送", "带超时：队列满就挂起，到期返回 假"),
    ("信号量等待", "带超时：拿不到就挂起，到期返回 假"),
];

struct SyntaxRow {
    construct: &'static str,
    form: &'static str,
    example: &'static str,
}

const SYNTAX: &[SyntaxRow] = &[
    SyntaxRow {
        construct: "声明/赋值",
        form: "令 名字 = 表达式",
        example: "令 数量 = 3",
    },
    SyntaxRow {
        construct: "导入原生模块",
        form: "导入 模块名",
        example: "导入 引脚\n引脚写(13, 0)",
    },
    SyntaxRow {
        construct: "重新赋值",
        form: "名字 = 表达式",
        example: "数量 = 数量 + 1",
    },
    SyntaxRow {
        construct: "下标赋值",
        form: "容器[下标] = 表达式",
        example: "统计[\"a\"] = 1",
    },
    SyntaxRow {
        construct: "复合赋值",
        form: "左值 += 表达式（-= *= /= //= %= **= 同理）",
        example: "总数 += 2",
    },
    SyntaxRow {
        construct: "条件",
        form: "如果 条件: / 否则如果 条件: / 否则:",
        example: "如果 n > 0:\n    打印(n)\n否则:\n    打印(0)",
    },
    SyntaxRow {
        construct: "当循环",
        form: "当 条件:",
        example: "当 n > 0:\n    n -= 1",
    },
    SyntaxRow {
        construct: "计数循环",
        form: "循环 N 次:",
        example: "循环 3 次:\n    打印(\"砾\")",
    },
    SyntaxRow {
        construct: "遍历",
        form: "遍历 名字 在 可迭代:",
        example: "遍历 x 在 范围(5):\n    打印(x)",
    },
    SyntaxRow {
        construct: "循环控制",
        form: "中断 / 继续",
        example: "循环 5 次:\n    如果 真:\n        继续",
    },
    SyntaxRow {
        construct: "函数",
        form: "函数 名字(参数, ...):",
        example: "函数 平方(x):\n    返回 x * x",
    },
    SyntaxRow {
        construct: "类型标注（仅忽略）",
        form: "函数 名字(参数: 类型) -> 类型:",
        example: "函数 加一(x: 整数) -> 整数:\n    返回 x + 1",
    },
    SyntaxRow {
        construct: "返回",
        form: "返回 [表达式]",
        example: "返回 0",
    },
    SyntaxRow {
        construct: "抛出",
        form: "抛出 表达式",
        example: "抛出 \"坏输入\"",
    },
    SyntaxRow {
        construct: "尝试/捕获",
        form: "尝试: / 捕获 名字: / 捕获:",
        example: "尝试:\n    抛出 \"坏输入\"\n捕获 原因:\n    打印(原因)",
    },
    SyntaxRow {
        construct: "调用",
        form: "被调用者(实参, ...)",
        example: "打印(长度(列表))",
    },
    SyntaxRow {
        construct: "类",
        form: "类 名字:\n    令 字段 = 默认值\n    函数 初始化(参数): ...\n    函数 方法(参数): ...",
        example: "类 计数器:\n    令 值 = 0\n    函数 初始化(起点):\n        本.值 = 起点\n    函数 加(n):\n        本.值 += n",
    },
    SyntaxRow {
        construct: "新建 / 成员",
        form: "新建 类名(实参...)；对象.字段；对象.方法(...)；本（方法内）",
        example: "令 c = 新建 计数器(5)\nc.加(3)\n打印(c.值)",
    },
    SyntaxRow {
        construct: "下标",
        form: "容器[下标]",
        example: "列表[0]",
    },
    SyntaxRow {
        construct: "列表",
        form: "[元素, ...]",
        example: "[1, 2, 3]",
    },
    SyntaxRow {
        construct: "字典",
        form: "{键: 值, ...}",
        example: "{\"a\": 1, \"b\": 2}",
    },
];

const GRAMMAR: &[&str] = &[
    "program := { statement }",
    "statement := import | class | let | assign | if | while | repeat | foreach | funcdef | return | break | continue | throw | try | expr_stmt",
    "class := '类' IDENT ':' NEWLINE INDENT { class_member } DEDENT",
    "class_member := '令' IDENT '=' expr NEWLINE | funcdef",
    "import := '导入' IDENT",
    "let := '令' IDENT '=' expr",
    "assign := lvalue '=' expr | lvalue AUGOP expr",
    "lvalue := IDENT | postfix '[' expr ']' | postfix '.' IDENT",
    "if := '如果' expr block { '否则如果' expr block } [ '否则' block ]",
    "while := '当' expr block",
    "repeat := '循环' expr [ '次' ] block",
    "foreach := '遍历' IDENT '在' expr block",
    "funcdef := '函数' IDENT '(' [ param { ',' param } ] ')' [ '->' IDENT ] block",
    "param := IDENT [ ':' IDENT ]",
    "return := '返回' [ expr ]",
    "throw := '抛出' expr",
    "try := '尝试' block '捕获' [ IDENT ] block",
    "block := ':' NEWLINE INDENT { statement } DEDENT",
    "expr := or",
    "or := and { '或' and }",
    "and := not { '与' not }",
    "not := '非' not | compare",
    "compare := add { ('=='|'!='|'<'|'>'|'<='|'>=') add }",
    "add := mul { ('+'|'-') mul }",
    "mul := unary { ('*'|'/'|'//'|'%') unary }",
    "unary := '-' unary | power",
    "power := postfix [ '**' unary ]",
    "postfix := primary { '(' args ')' | '[' expr ']' | '.' IDENT [ '(' args ')' ] }",
    "primary := INT | STR | '真' | '假' | '空' | IDENT | '本' | '新建' IDENT '(' args ')' | '(' expr ')' | list | dict",
    "list := '[' [ expr { ',' expr } [ ',' ] ] ']'",
    "dict := '{' [ expr ':' expr { ',' expr ':' expr } [ ',' ] ] '}'",
];

struct SemanticRow {
    topic: &'static str,
    rule: &'static str,
}

const SEMANTICS: &[SemanticRow] = &[
    SemanticRow { topic: "整数", rule: "29 位有符号：-268435456 .. 268435455；字面量与运算结果越界都报错。" },
    SemanticRow { topic: "小数", rule: "Q12 定点：raw = 值 × 4096，值域约 ±524288，精度 1/4096（0.000244）；字面量小数点后必须有数字，不支持指数（E102）。" },
    SemanticRow { topic: "加法重载", rule: "`+` 支持 整数/小数相加、文本+文本（拼接）、列表+列表（连接）；其他组合报类型错误。" },
    SemanticRow { topic: "除法", rule: "整数/整数时 `/` 要求整除，否则报 E023；任一操作数是小数时 `/` 返回小数（四舍五入）。`//` 向下取整且返回整数；`%` 结果符号跟随除数。" },
    SemanticRow { topic: "幂", rule: "`**` 的指数必须是非负整数；结果按重复乘实现，溢出即报错。" },
    SemanticRow { topic: "比较", rule: "`< > <= >=` 支持整数/小数混合按数值比较、文本对文本（按 UTF-8 字节字典序）；`== !=` 对所有类型成立。" },
    SemanticRow { topic: "真值", rule: "假：0、0.0、空、假、未定义、空文本、空列表、空字典、空范围；其余为真（含函数与内建）。" },
    SemanticRow { topic: "下标", rule: "列表/文本支持负下标（-1 是最后一个）；字典按键查找，键不存在报 E_INDEX；文本下标按字符而不是字节。" },
    SemanticRow { topic: "原生模块", rule: "`导入 模块` 只能写在模块顶层；导入后模块里的原生函数名才能调用，名字与参数个数在编译期校验（E112/E113/E115）；`.jmb` 仍只带名字，运行期由 native 表绑定。" },
    SemanticRow { topic: "绿色任务", rule: "`导入 任务` 后用 `创建(函数)` 起一个无参子任务（根任务之外最多 3 个），`让出()` 主动换出，`延时(毫秒)` 睡到指定时刻；`删除(句柄)` 可取消一个未在运行的任务。调度是协作式、按槽轮转，不抢占、无优先级。子任务值栈 96 槽、调用帧 24、`尝试` 最多 4 层（第 5 层报 E028）；根任务仍是 512 槽 / 32 帧 / 8 层。子任务退出的槽位暂不复用。" },
    SemanticRow { topic: "任务等待", rule: "带超时的 `队列接收(队, 毫秒)`、`队列发送(队, 值, 毫秒)`、`信号量等待(信号量, 毫秒)` 会先探测一次，失败就只睡 1 tick 再重试，其他任务照常运行；到期仍不成功返回 `空`（接收）或 `假`（发送/信号量）。不带超时的形式仍是普通原生调用。" },
    SemanticRow { topic: "作用域", rule: "函数内的 `令` 是局部槽；内层函数读取外层局部变量时会按值捕获（定义时快照、只读）；未在函数内声明而同名的读取会落到模块级全局。" },
    SemanticRow { topic: "函数", rule: "模块级函数与闭包都是一等值：可以赋值、传参、作为返回值；实参按值传递；形参与实参个数必须一致；调用层数上限 32。" },
    SemanticRow { topic: "闭包", rule: "内层函数读取外层局部变量即按值捕获（定义那一刻的快照，之后外层再改也不影响）；捕获变量只读（赋值报 E117），不能在函数里捕获自己的名字（E117）；单个函数捕获上限 255（E116）。" },
    SemanticRow { topic: "字符串", rule: "单引号、双引号、中文引号都可定界；反斜杠转义支持 \\n \\t \\\\ \\\" \\'；文本按 UTF-8 存。" },
    SemanticRow { topic: "字典键", rule: "键必须是整数/文本/空/真/假这类不可变值；列表、字典、函数、实例不能当键。" },
    SemanticRow { topic: "类", rule: "`类` 只能写在模块顶层，类名全局唯一；类体只允许字段（`令 名 = 默认值`，默认值必填且不能引用 `本`）与方法。`新建 C(...)` 调用构造：有 `初始化` 时实参个数必须与它一致，否则只能零实参；`初始化` 里禁止 `返回`，构造结果固定是实例本身。方法内 `本` 是只读绑定（不能 `本 = ...`），`本.字段` 读写与 `对象.字段` 相同。没有继承、没有虚分派：字段和方法都在运行期按名字在类的单层表里查找（结构化多态）。未声明字段/方法报 E035/E037，对非实例取成员报 E036；`本.未知` 与 `本.方法` 当值取是编译期 E122/E123。" },
    SemanticRow { topic: "实例", rule: "实例是普通堆对象：可进列表/字典值、可 `抛出` 并被 `捕获`、可被闭包捕获（含捕获 `本`）；相等比较按身份；`长度`/下标/遍历不适用；native 只能把实例当入参透传，不能构造或返回。" },
    SemanticRow { topic: "迭代", rule: "可遍历：列表、文本（按字符）、字典（按键）、范围。" },
    SemanticRow { topic: "异常", rule: "`抛出 表达式` 抛任意值；`尝试: ... 捕获 名字: ...`（或匿名 `捕获:`）在同一函数内捕获。只有显式 `抛出` 可捕获：除零（E007）、堆耗尽（E017）等 VM 故障与原生错误（E030/E031/E033）一律终止，不能被 `捕获` 拦下。没有 `最后`，一个 `尝试` 只能跟一个 `捕获`。" },
    SemanticRow { topic: "异常传播", rule: "抛出后按调用栈向外找最近的 `捕获`；展开时释放被跳过帧的局部槽（含闭包捕获槽）与操作数临时值，抛出值引用计数平衡。到最外层仍未捕获则 `jm_run` 返回 -1，错误为 `错误[E027] 未捕获的异常：<值>`。`返回`/`中断`/`继续` 离开 `尝试` 时都会先结束该层记录。" },
    SemanticRow { topic: "捕获变量", rule: "`捕获 名字` 绑定的名字是当前作用域的普通可写变量（函数内为局部槽，模块顶层为全局）；`捕获:` 不绑定名字，丢弃抛出值。" },
    SemanticRow { topic: "内存", rule: "脚本对象都放在固定堆里（固件默认 6 KB）；闭包、容器与实例都按引用计数释放（实例释放时连带释放全部字段），但相互引用成环时 rc 不归零，这块堆不会回收（没有 GC，接受环引用泄漏），反复创建最终报 E_OOM；其余堆满也报 E_OOM，不会崩溃。单个实例带块头占 16+4×字段数 字节。" },
];

struct LimitRow {
    item: &'static str,
    value: &'static str,
}

const LIMITS: &[LimitRow] = &[
    LimitRow { item: "整数范围", value: "-268435456 .. 268435455（29 位）" },
    LimitRow { item: "小数范围/精度", value: "约 ±524288，精度 1/4096（Q12 定点）" },
    LimitRow { item: "名字/常量", value: "各 255 个" },
    LimitRow { item: "函数", value: "255 个" },
    LimitRow { item: "局部变量/函数", value: "255 个" },
    LimitRow { item: "实参/形参", value: "各 255 个" },
    LimitRow { item: "闭包捕获/函数", value: "255 个（E116）" },
    LimitRow { item: "列表元素/字典项", value: "各 255 个（字面量编译期）" },
    LimitRow { item: "调用层数", value: "32 层（JM_MAX_FRAMES）" },
    LimitRow { item: "尝试嵌套", value: "每任务独立 try 栈：根任务 8 层、子任务 4 层（超过报 E028）" },
    LimitRow { item: "绿色任务", value: "根任务 + 最多 3 个子任务（`创建` 第 4 次报 E038）" },
    LimitRow { item: "子任务限制", value: "值栈 96 槽、调用帧 24、`尝试` 最多 4 层（超过报 E028）" },
    LimitRow { item: "未捕获异常显示", value: "固定 64 B 缓冲（60 B 正文 + 3 B「…」 + NUL）；正文放不下就截断并以「…」结尾，不会静默丢弃" },
    LimitRow { item: "类", value: "255 个；每类字段 255 个、方法 255 个" },
    LimitRow { item: "值栈", value: "512 槽（JM_STACK_SLOTS）" },
    LimitRow { item: "脚本堆", value: "固件 6 KB / 宿主默认 12 KB" },
    LimitRow { item: "字节码", value: "单个函数 <64 KB，整包 <64 KB" },
];

struct Idiom {
    title: &'static str,
    snippet: &'static str,
}

const IDIOMS: &[Idiom] = &[
    Idiom {
        title: "用范围做计数循环",
        snippet: "遍历 i 在 范围(0, 10):\n    打印(i)",
    },
    Idiom {
        title: "累加器",
        snippet: "令 总和 = 0\n遍历 x 在 列表:\n    总和 += x\n打印(总和)",
    },
    Idiom {
        title: "字典当计数表",
        snippet: "令 次数 = {}\n遍历 x 在 范围(3):\n    次数[x] = x * x\n打印(次数)",
    },
    Idiom {
        title: "文本拼接",
        snippet: "令 n = 7\n打印(\"n = \" + 文本(n))",
    },
    Idiom {
        title: "提前返回",
        snippet: "函数 符号(x):\n    如果 x > 0:\n        返回 1\n    如果 x < 0:\n        返回 -1\n    返回 0",
    },
    Idiom {
        title: "共享数据放模块级",
        snippet: "令 计数 = 0\n函数 加一():\n    计数 += 1",
    },
];

struct Pitfall {
    title: &'static str,
    bad: &'static str,
    good: &'static str,
    why: &'static str,
}

const PITFALLS: &[Pitfall] = &[
    Pitfall {
        title: "ADC 换算推荐小数：原始 / 4095.0 * 3300",
        bad: "打印(4094 * 3300.0)",
        good: "打印(4094 / 4095.0 * 3300)",
        why: "推荐写法是 `原始 / 4095.0 * 3300`，得到 3299.1943 这样的毫伏小数。小数是 Q12 定点，值域约 ±524288，所以必须先除后乘：写成 `原始 * 3300.0` 会得到 13,510,200，直接报 E009 溢出。只有明确要整数毫伏时才改用整数整除 `//`（见下一条）。",
    },
    Pitfall {
        title: "纯整数场景：结果要整除，否则用 //",
        bad: "打印(4094 * 3300 / 4095)",
        good: "打印(4094 * 3300 // 4095)\n打印((4094 * 3300 + 2047) // 4095)",
        why: "整数/整数的 `/` 仍是真除法，结果必须整除，否则运行期报 E023（E_DIV_NONINT）：ADC 读 4094 时 `4094 * 3300 / 4095 = 3299.19…` 会在板上中止。确实只要整数毫伏时用 `//` 得 3299；四舍五入加半个除数，`(4094 * 3300 + 2047) // 4095` 得 3299。需要小数就回到上一条的 `4095.0` 写法。",
    },
    Pitfall {
        title: "缩进必须是 4 个半角空格",
        bad: "如果 真:\n\t打印(1)",
        good: "如果 真:\n    打印(1)",
        why: "Tab 与全角空格在缩进位置报 E109；全角标点只在代码里自动转半角。",
    },
    Pitfall {
        title: "闭包按值捕获，捕获变量只读",
        bad: "函数 外层():\n    令 x = 1\n    函数 内层():\n        x = 2\n        返回 x",
        good: "函数 外层():\n    令 x = 1\n    函数 内层():\n        返回 x + 1",
        why: "内层函数读到的 `x` 是定义那一刻的快照，之后外层把 `x` 改成 2 也不影响闭包（仍是 1）。给捕获变量赋值报 E117；要改状态就放模块级 `令`，或在函数里用 `令` 新建同名局部变量。",
    },
    Pitfall {
        title: "只有显式抛出能被捕获",
        bad: "尝试:\n    打印(1 / 0)\n捕获 原因:\n    打印(\"捕获了\")",
        good: "如果 除数 == 0:\n    抛出 \"除数为零\"\n返回 被除数 / 除数",
        why: "除零（E007）、堆耗尽（E017）这类 VM 故障和原生错误（E030/E031/E033）是终止性的，`捕获` 拦不住，程序会直接以 `运行失败:` 结束。要可恢复的错误就先用条件判断，再显式 `抛出` 并 `捕获`。",
    },
    Pitfall {
        title: "尝试必须紧跟同层捕获",
        bad: "尝试:\n    抛出 1\n打印(\"继续\")",
        good: "尝试:\n    抛出 1\n捕获 原因:\n    打印(原因)",
        why: "v0 没有 `最后`，也不允许 `尝试` 后没有 `捕获`：缺少同层 `捕获` 报编译错误 E109；单独出现的 `捕获` 同样报 E109。",
    },
    Pitfall {
        title: "闭包成环会一直占堆（没有 GC）",
        bad: "令 列表 = []\n令 回调 = 造回调(列表)\n列表追加(列表, 回调)",
        good: "只把闭包传给需要它的函数，不让它再被写回自己捕获的容器",
        why: "闭包与它捕获的列表/字典互相引用时，引用计数无法归零，这块堆不会回收；实例字段存回自身（如 `本.自己 = 本`）同样会成环。反复创建这类环会逐步吃满堆，最终运行期报 E017（E_OOM）。当前无 GC，接受环引用泄漏：别在实例字段里存回指自身/父级的引用。",
    },
    Pitfall {
        title: "类没有继承；方法是“调用”，不是值",
        bad: "类 子 继承 父:\n    函数 读():\n        返回 本.值\n令 f = c.读",
        good: "类 计数器:\n    令 值 = 0\n    函数 读():\n        返回 本.值\n打印(c.读())",
        why: "`继承` 仍是 v0 不支持的关键字（E103），没有父类与虚分派。方法只能 `对象.方法(...)` 调用；把方法当值取（`令 f = c.读`）拿不到方法，`本.读` 会直接报 E123，普通对象则运行期报 E035。",
    },
    Pitfall {
        title: "不要给未声明的名字赋值",
        bad: "计数 = 计数 + 1",
        good: "令 计数 = 0\n计数 = 计数 + 1",
        why: "读取未定义名字会报 E_NAME；先 `令` 再赋值。",
    },
    Pitfall {
        title: "原生函数要先导入",
        bad: "引脚写(13, 0)",
        good: "导入 引脚\n引脚写(13, 0)",
        why: "没写 `导入 引脚` 直接调用会报 E113；名字拼错或参数个数不符报 E115，都在编译期，不会等到运行时 E004。",
    },
    Pitfall {
        title: "列表/文本下标别越界",
        bad: "令 x = [1, 2][5]",
        good: "令 x = [1, 2][长度([1, 2]) - 1]",
        why: "越界是运行错误 E_INDEX，不是返回空；负下标从 -1 开始。",
    },
];

pub struct Example {
    pub id: &'static str,
    pub title: &'static str,
    pub source: &'static str,
    pub prints: &'static str,
}

const EXAMPLES: &[Example] = &[
    Example {
        id: "hello",
        title: "你好",
        source: "打印(\"你好，砾\")",
        prints: "你好，砾",
    },
    Example {
        id: "fib",
        title: "递归斐波那契",
        source: "函数 斐波(n):\n    如果 n <= 1:\n        返回 n\n    返回 斐波(n - 1) + 斐波(n - 2)\n\n打印(斐波(10))",
        prints: "55",
    },
    Example {
        id: "sum-range",
        title: "范围遍历求和",
        source: "令 总和 = 0\n遍历 i 在 范围(1, 6):\n    总和 += i\n打印(总和)",
        prints: "15",
    },
    Example {
        id: "dict-count",
        title: "字典计数",
        source: "令 次数 = {\"a\": 1}\n次数[\"b\"] = 2\n打印(次数[\"b\"])",
        prints: "2",
    },
    Example {
        id: "text",
        title: "文本拼接与内建",
        source: "令 n = 3\n打印(\"n = \" + 文本(n), 绝对值(-n))",
        prints: "n = 3 3",
    },
    Example {
        id: "decimal",
        title: "小数与定点除法",
        source: "打印(1.5 + 2.25)\n打印(10.0 / 4)\n打印(4094 / 4095.0 * 3300)",
        prints: "3.75\n2.5\n3299.1943",
    },
    Example {
        id: "closure",
        title: "闭包（按值捕获，只读）",
        source: "函数 计数器(起点):\n    令 n = 起点\n    函数 取():\n        返回 n\n    返回 取\n\n令 f = 计数器(10)\n打印(f())",
        prints: "10",
    },
    Example {
        id: "exception",
        title: "抛出与捕获",
        source: "函数 取数(n):\n    如果 n < 0:\n        抛出 \"负数\"\n    返回 n\n\n尝试:\n    打印(取数(-1))\n捕获 原因:\n    打印(\"捕获\", 原因)",
        prints: "捕获 负数",
    },
    Example {
        id: "board-led",
        title: "板级点灯（导入原生模块）",
        source: "导入 引脚\n引脚写(13, 0)\n等待毫秒(400)\n引脚写(13, 1)",
        prints: "（板上执行，无主机输出）",
    },
];

/// 仓库根目录：`compiler/..`。
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler 目录没有上级目录")
        .to_path_buf()
}

fn read_or_fail(path: &Path, what: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("读不了{} {}：{}", what, path.display(), e))
}

/// 从 C 侧 `jm_builtin.c` 解析内建表：`#define B_X n` + `BUILTINS[]` 里的 `{"名字", B_X}`。
fn c_builtins() -> Result<Vec<(u8, String)>, String> {
    let path = repo_root().join("runtime/src/jm_builtin.c");
    let src = read_or_fail(&path, "内建函数表")?;

    let mut ids: Vec<(String, u8)> = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#define B_") {
            let mut it = rest.split_whitespace();
            if let (Some(name), Some(num)) = (it.next(), it.next()) {
                if let Ok(v) = num.parse::<u8>() {
                    ids.push((name.to_string(), v));
                }
            }
        }
    }
    if ids.is_empty() {
        return Err("jm_builtin.c 里没找到 #define B_* 的内建 id".to_string());
    }

    let mut out: Vec<(u8, String)> = Vec::new();
    let mut in_table = false;
    for line in src.lines() {
        let line = line.trim();
        if line.contains("BUILTINS[]") {
            in_table = true;
            continue;
        }
        if in_table {
            if line.starts_with("};") {
                break;
            }
            let mut rest = line;
            while let Some(p) = rest.find("{\"") {
                rest = &rest[p + 1..];
                let Some(end_name) = rest[1..].find('"') else {
                    break;
                };
                let name = rest[1..1 + end_name].to_string();
                let after = &rest[1 + end_name + 1..];
                let Some(p2) = after.find("B_") else {
                    break;
                };
                let tail = &after[p2 + 2..];
                let key: String = tail
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let Some(&(_, id)) = ids.iter().find(|(n, _)| *n == key) else {
                    return Err(format!(
                        "BUILTINS[] 里的 {} 没有对应的 #define B_{}", name, key
                    ));
                };
                out.push((id, name));
                rest = tail;
            }
        }
    }
    if out.is_empty() {
        return Err("jm_builtin.c 里没解析到 BUILTINS[] 条目".to_string());
    }
    out.sort_by_key(|(id, _)| *id);
    Ok(out)
}

/// 从 `jm_internal.h` 解析运行期错误码序号。
fn c_runtime_errors() -> Result<Vec<(String, u8)>, String> {
    let path = repo_root().join("runtime/src/jm_internal.h");
    let src = read_or_fail(&path, "运行期错误码")?;
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#define E_") {
            let mut it = rest.split_whitespace();
            if let (Some(name), Some(num)) = (it.next(), it.next()) {
                if let Ok(v) = num.parse::<u8>() {
                    out.push((format!("E_{}", name), v));
                }
            }
        }
    }
    if out.is_empty() {
        return Err("jm_internal.h 里没找到 #define E_*".to_string());
    }
    out.sort_by_key(|(_, v)| *v);
    Ok(out)
}

/// 从 `jm_native.h` 解析 Native ABI 版本号（`#define JM_NATIVE_ABI_VERSION 1u`）。
fn c_native_abi_version() -> Result<u8, String> {
    let path = repo_root().join("runtime/include/jm_native.h");
    let src = read_or_fail(&path, "Native ABI 版本")?;
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#define JM_NATIVE_ABI_VERSION") {
            let value = rest.trim().trim_end_matches('u').trim();
            let parsed = value.parse::<u8>().map_err(|_| {
                format!(
                    "jm_native.h 里 JM_NATIVE_ABI_VERSION 不是 0~255 的整数：{}",
                    value
                )
            })?;
            if parsed != crate::native_spec::NATIVE_ABI_VERSION {
                return Err(format!(
                    "Native ABI 版本不一致：jm_native.h 是 v{}，native_spec.rs 是 v{}；请同步两侧",
                    parsed,
                    crate::native_spec::NATIVE_ABI_VERSION
                ));
            }
            return Ok(parsed);
        }
    }
    Err("jm_native.h 里没找到 #define JM_NATIVE_ABI_VERSION".to_string())
}

fn builtin_table() -> Result<Vec<(u8, String, u8, u8, &'static str)>, String> {
    let c = c_builtins()?;
    let mut out = Vec::new();
    for (id, name) in &c {
        let Some(doc) = BUILTIN_DOCS.iter().find(|d| d.name == name) else {
            return Err(format!(
                "内建 {}（id {}）缺少文档元数据，请在 compiler/src/langcard.rs 的 BUILTIN_DOCS 里补上",
                name, id
            ));
        };
        out.push((*id, name.clone(), doc.min_args, doc.max_args, doc.doc));
    }
    for doc in BUILTIN_DOCS {
        if !c.iter().any(|(_, n)| n == doc.name) {
            return Err(format!(
                "BUILTIN_DOCS 里的 {} 在 runtime/src/jm_builtin.c 的 BUILTINS[] 里已不存在",
                doc.name
            ));
        }
    }
    Ok(out)
}

fn operand_name(o: Operand) -> &'static str {
    match o {
        Operand::None => "无",
        Operand::U8 => "1 字节",
        Operand::U16 => "2 字节",
        Operand::U8U8 => "1 字节 + 1 字节",
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn json_str_array(items: &[&str], indent: &str) -> String {
    let mut s = String::from("[");
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('\n');
        s.push_str(indent);
        s.push_str(&format!("\"{}\"", esc(it)));
    }
    s.push('\n');
    s.push_str(&indent[..indent.len().saturating_sub(2)]);
    s.push(']');
    s
}

/// 生成 `spec/langcard.json`。
pub fn render_json() -> Result<String, String> {
    let builtins = builtin_table()?;
    let runtime_errs = c_runtime_errors()?;
    let native_abi = c_native_abi_version()?;
    let magic = std::str::from_utf8(&MAGIC)
        .map_err(|_| "writer.rs 里的 JMB 魔数不是合法 UTF-8".to_string())?;

    let mut j = String::new();
    j.push_str("{\n");
    j.push_str(&format!("  \"schema\": \"{}\",\n", SCHEMA));
    j.push_str(&format!("  \"lang_version\": \"{}\",\n", LANG_VERSION));
    j.push_str(&format!("  \"bytecode_magic\": \"{}\",\n", magic));
    j.push_str(&format!("  \"bytecode_version\": {},\n", VERSION));
    j.push_str(&format!("  \"native_abi_version\": {},\n", native_abi));
    // 兼容字段：旧消费者读 abi_version，值仍等同于魔数；新消费者改用上面两个字段。
    j.push_str(&format!("  \"abi_version\": \"{}\",\n", magic));
    j.push_str("  \"abi_version_note\": \"已弃用：字节码版本看 bytecode_version，Native ABI 看 native_abi_version；abi_version 仅保留给旧消费者。\",\n");
    j.push_str("  \"name\": \"砾（Micro Core v0）语言卡\",\n");
    j.push_str(
        "  \"target\": \"STM32F103C8T6 固件内解释器（64 KB Flash / 20 KB RAM），宿主侧用 micc 编译为 .jmb\",\n",
    );
    j.push_str("  \"purpose\": \"给大模型与工具读的机器可读参考：语法、语义、内建 API、惯用法、错误与可编译示例。\",\n");
    j.push_str("  \"single_source\": {\n");
    j.push_str("    \"opcodes\": \"compiler/src/opcodes.rs\",\n");
    j.push_str("    \"bytecode_format\": \"compiler/src/writer.rs（MAGIC / VERSION）\",\n");
    j.push_str("    \"native_abi\": \"runtime/include/jm_native.h（JM_NATIVE_ABI_VERSION）\",\n");
    j.push_str("    \"keywords\": \"compiler/src/lexer.rs\",\n");
    j.push_str("    \"builtins\": \"runtime/src/jm_builtin.c\",\n");
    j.push_str("    \"native_modules\": \"compiler/src/native_spec.rs\",\n");
    j.push_str("    \"compile_errors\": \"compiler/src/error.rs\",\n");
    j.push_str("    \"runtime_errors\": \"runtime/src/jm_internal.h\",\n");
    j.push_str("    \"note\": \"本文件由 micc --langcard 生成，不要手改；micc --check-langcard 在 CI 里比对。\"\n");
    j.push_str("  },\n");

    // 关键字：直接读 lexer 的表，保证同源。
    let kws = lexer::keyword_table();
    j.push_str("  \"keywords\": [\n");
    let mut supported: Vec<&str> = Vec::new();
    let mut unsupported: Vec<&str> = Vec::new();
    for (w, k) in kws {
        if let lexer::Kw::Unsupported(_) = k {
            unsupported.push(w);
        } else {
            supported.push(w);
        }
    }
    supported.sort();
    unsupported.sort();
    for (i, w) in supported.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"word\": \"{}\", \"supported\": true}}{}\n",
            esc(w),
            if i + 1 < supported.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");
    j.push_str("  \"unsupported_keywords\": ");
    j.push_str(&json_str_array(&unsupported, "    "));
    j.push_str(",\n");

    // 词法规则
    j.push_str("  \"lexical\": [\n");
    let lexical: &[(&str, &str)] = &[
        ("缩进", "只认半角空格，4 个一级；Tab 或全角空格报 E109"),
        ("注释", "# 到行尾"),
        ("文本", "双引号、单引号、中文引号都可定界；支持 \\n \\t \\\\ \\\" \\'"),
        ("全角标点", "代码区的全角标点自动转半角（字符串内部不转）"),
        ("标识符", "字母或下划线开头，后接字母/数字/下划线"),
        ("整数", "十进制数字；带指数报 E102"),
        ("小数", "如 1.5、0.25；小数点后必须有数字，不支持指数（E102）"),
    ];
    for (i, (k, v)) in lexical.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"item\": \"{}\", \"rule\": \"{}\"}}{}\n",
            esc(k),
            esc(v),
            if i + 1 < lexical.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 语法
    j.push_str("  \"syntax\": [\n");
    for (i, s) in SYNTAX.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"construct\": \"{}\", \"form\": \"{}\", \"example\": \"{}\"}}{}\n",
            esc(s.construct),
            esc(s.form),
            esc(s.example),
            if i + 1 < SYNTAX.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");
    j.push_str("  \"grammar\": ");
    j.push_str(&json_str_array(GRAMMAR, "    "));
    j.push_str(",\n");

    // 运算符（来自 opcodes.rs）
    j.push_str("  \"operators\": {\n");
    j.push_str("    \"precedence\": [\"或\", \"与\", \"非\", \"== != < > <= >=\", \"+ -\", \"* / // %\", \"一元 -\", \"**\", \"调用/下标\"],\n");
    j.push_str("    \"binary\": [");
    let bins: Vec<String> = (0..=opcodes::BIN_POW)
        .map(|s| format!("\"{}\"", opcodes::binop_name(s)))
        .collect();
    j.push_str(&bins.join(", "));
    j.push_str("],\n");
    j.push_str("    \"compare\": [");
    let cmps: Vec<String> = (0..=opcodes::CMP_GE)
        .map(|s| format!("\"{}\"", opcodes::cmp_name(s)))
        .collect();
    j.push_str(&cmps.join(", "));
    j.push_str("],\n");
    j.push_str("    \"unary\": [\"-（取负，仅整数）\", \"非\"]\n");
    j.push_str("  },\n");

    // 语义
    j.push_str("  \"semantics\": [\n");
    for (i, s) in SEMANTICS.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"topic\": \"{}\", \"rule\": \"{}\"}}{}\n",
            esc(s.topic),
            esc(s.rule),
            if i + 1 < SEMANTICS.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 限制
    j.push_str("  \"limits\": [\n");
    for (i, s) in LIMITS.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"item\": \"{}\", \"value\": \"{}\"}}{}\n",
            esc(s.item),
            esc(s.value),
            if i + 1 < LIMITS.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 内建
    j.push_str("  \"builtins\": [\n");
    for (i, (id, name, min, max, doc)) in builtins.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"name\": \"{}\", \"id\": {}, \"min_args\": {}, \"max_args\": {}, \"doc\": \"{}\"}}{}\n",
            esc(name),
            id,
            min,
            max,
            esc(doc),
            if i + 1 < builtins.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 原生模块：与 C 侧 native 表同源于 compiler/src/native_spec.rs
    j.push_str("  \"native_modules\": [\n");
    for (i, m) in native_spec::MODULES.iter().enumerate() {
        j.push_str("    {\n");
        j.push_str(&format!("      \"module\": \"{}\",\n", esc(m.name)));
        j.push_str(&format!("      \"table\": \"{}\",\n", esc(m.table)));
        j.push_str("      \"entries\": [\n");
        for (k, e) in m.entries.iter().enumerate() {
            j.push_str(&format!(
                "        {{\"name\": \"{}\", \"min_args\": {}, \"max_args\": {}, \"doc\": \"{}\"}}{}\n",
                esc(e.name),
                e.min_args,
                e.max_args,
                esc(e.doc),
                if k + 1 < m.entries.len() { "," } else { "" }
            ));
        }
        j.push_str("      ]\n");
        j.push_str(&format!(
            "    }}{}\n",
            if i + 1 < native_spec::MODULES.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 语言层绿色任务：不走 native 表，单独给出
    j.push_str("  \"task_module\": {\n");
    j.push_str(&format!("    \"module\": \"{}\",\n", esc(vm_spec::TASK_MODULE)));
    j.push_str("    \"note\": \"根任务 + 最多 3 个子任务，协作式按槽轮转；子任务值栈 96 槽、调用帧 24、尝试 4 层\",\n");
    j.push_str("    \"functions\": [\n");
    for (i, f) in vm_spec::TASK_FNS.iter().enumerate() {
        let doc = TASK_FN_DOCS
            .iter()
            .find(|(n, _)| *n == f.name)
            .map(|(_, d)| *d)
            .ok_or_else(|| format!("任务函数 {} 缺少说明，请补 TASK_FN_DOCS", f.name))?;
        j.push_str(&format!(
            "      {{\"name\": \"{}\", \"nargs\": {}, \"opcode\": {}, \"doc\": \"{}\"}}{}\n",
            esc(f.name),
            f.nargs,
            f.opcode,
            esc(doc),
            if i + 1 < vm_spec::TASK_FNS.len() { "," } else { "" }
        ));
    }
    j.push_str("    ],\n");
    j.push_str("    \"wait_functions\": [\n");
    for (i, w) in vm_spec::WAIT_FNS.iter().enumerate() {
        let doc = WAIT_FN_DOCS
            .iter()
            .find(|(n, _)| *n == w.name)
            .map(|(_, d)| *d)
            .ok_or_else(|| format!("等待函数 {} 缺少说明，请补 WAIT_FN_DOCS", w.name))?;
        j.push_str(&format!(
            "      {{\"name\": \"{}\", \"kind\": {}, \"timeout_args\": {}, \"doc\": \"{}\"}}{}\n",
            esc(w.name),
            w.kind,
            w.nargs,
            esc(doc),
            if i + 1 < vm_spec::WAIT_FNS.len() { "," } else { "" }
        ));
    }
    j.push_str("    ]\n");
    j.push_str("  },\n");

    // 操作码
    j.push_str("  \"opcodes\": [\n");
    let all_ops: Vec<u8> = (0..=opcodes::OP_TASK_WAIT).collect();
    for (i, op) in all_ops.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"value\": {}, \"name\": \"{}\", \"operand\": \"{}\"}}{}\n",
            op,
            opcodes::op_name(*op),
            operand_name(opcodes::operand(*op)),
            if i + 1 < all_ops.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 编译期错误
    j.push_str("  \"compile_errors\": [\n");
    let cc = crate::error::COMPILE_CODES;
    for (i, (code, meaning)) in cc.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"code\": \"{}\", \"meaning\": \"{}\"}}{}\n",
            code,
            esc(meaning),
            if i + 1 < cc.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 运行期错误
    j.push_str("  \"runtime_errors\": [\n");
    for (i, (name, value)) in runtime_errs.iter().enumerate() {
        let meaning = RUNTIME_ERROR_DOCS
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.meaning)
            .ok_or_else(|| {
                format!(
                    "运行期错误 {} 缺少说明，请在 compiler/src/langcard.rs 的 RUNTIME_ERROR_DOCS 里补上",
                    name
                )
            })?;
        j.push_str(&format!(
            "    {{\"code\": {}, \"name\": \"{}\", \"meaning\": \"{}\"}}{}\n",
            value,
            esc(name),
            esc(meaning),
            if i + 1 < runtime_errs.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 惯用法
    j.push_str("  \"idioms\": [\n");
    for (i, s) in IDIOMS.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"title\": \"{}\", \"snippet\": \"{}\"}}{}\n",
            esc(s.title),
            esc(s.snippet),
            if i + 1 < IDIOMS.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 常见错误
    j.push_str("  \"pitfalls\": [\n");
    for (i, s) in PITFALLS.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"title\": \"{}\", \"bad\": \"{}\", \"good\": \"{}\", \"why\": \"{}\"}}{}\n",
            esc(s.title),
            esc(s.bad),
            esc(s.good),
            esc(s.why),
            if i + 1 < PITFALLS.len() { "," } else { "" }
        ));
    }
    j.push_str("  ],\n");

    // 示例
    j.push_str("  \"examples\": [\n");
    for (i, s) in EXAMPLES.iter().enumerate() {
        j.push_str(&format!(
            "    {{\"id\": \"{}\", \"title\": \"{}\", \"source\": \"{}\", \"prints\": \"{}\"}}{}\n",
            esc(s.id),
            esc(s.title),
            esc(s.source),
            esc(s.prints),
            if i + 1 < EXAMPLES.len() { "," } else { "" }
        ));
    }
    j.push_str("  ]\n");
    j.push_str("}\n");
    Ok(j)
}

fn code_block(s: &str) -> String {
    let mut o = String::from("```砾\n");
    o.push_str(s);
    if !s.ends_with('\n') {
        o.push('\n');
    }
    o.push_str("```\n");
    o
}

/// 生成 `docs/语言卡.md`。
pub fn render_markdown() -> Result<String, String> {
    let builtins = builtin_table()?;
    let runtime_errs = c_runtime_errors()?;
    let native_abi = c_native_abi_version()?;
    let magic = std::str::from_utf8(&MAGIC)
        .map_err(|_| "writer.rs 里的 JMB 魔数不是合法 UTF-8".to_string())?;
    let kws = lexer::keyword_table();

    let mut m = String::new();
    m.push_str("# 砾（Micro Core v0）语言卡\n\n");
    m.push_str("> 本文件由 `micc --langcard` 生成，不要手改。\n");
    m.push_str("> 机器可读版本：[spec/langcard.json](../spec/langcard.json)。\n");
    m.push_str("> 规范来源：`compiler/src/opcodes.rs`、`compiler/src/lexer.rs`、`runtime/src/jm_builtin.c`、`compiler/src/native_spec.rs`、`compiler/src/error.rs`、`runtime/src/jm_internal.h`。\n>\n");
    m.push_str("> 改了任一来源而没重新生成，`micc --check-langcard` 会失败。\n\n");

    m.push_str("## 概览\n\n");
    m.push_str("| 项 | 值 |\n|---|---|\n");
    m.push_str(&format!("| 语言版本 | {} |\n", LANG_VERSION));
    m.push_str(&format!(
        "| 字节码格式 | JMB v{}（魔数 `{}`） |\n",
        VERSION, magic
    ));
    m.push_str(&format!(
        "| Native ABI | v{}（`JM_NATIVE_ABI_VERSION`） |\n",
        native_abi
    ));
    m.push_str("| 编译 | `micc 源码.jsh -o 输出.jmb` |\n");
    m.push_str("| 运行 | STM32F103C8T6 固件内解释器；脚本堆固件 6 KB |\n\n");
    m.push_str("> 机器可读字段：`bytecode_magic` / `bytecode_version` / `native_abi_version`；`abi_version` 已弃用，仅为旧消费者保留（值等于魔数）。\n\n");
    m.push_str("砾是**中文关键字、缩进式、整数优先**的小语言。它跑在 20 KB RAM 的 MCU 上，整数是 29 位、小数是 Q12 定点；类没有继承，`最后` 和通用模块系统也没有，异常用显式 `抛出` / `尝试` / `捕获`；闭包按值捕获且捕获变量只读；板级硬件能力通过 `导入 模块` 在编译期引入。\n\n");

    m.push_str("## 关键字\n\n");
    m.push_str("可用：");
    let supported: Vec<&str> = kws
        .iter()
        .filter(|(_, k)| !matches!(k, lexer::Kw::Unsupported(_)))
        .map(|(w, _)| *w)
        .collect();
    m.push_str(&supported.join("、"));
    m.push_str("。\n\n保留但不支持：");
    let unsupported: Vec<&str> = kws
        .iter()
        .filter(|(_, k)| matches!(k, lexer::Kw::Unsupported(_)))
        .map(|(w, _)| *w)
        .collect();
    m.push_str(&unsupported.join("、"));
    m.push_str("。这些是 v0 明确拒绝的关键字（E103）。\n\n");

    m.push_str("## 词法规则\n\n");
    m.push_str("| 项目 | 规则 |\n|---|---|\n");
    let lexical: &[(&str, &str)] = &[
        ("缩进", "只认半角空格，4 个一级；Tab 或全角空格报 E109"),
        ("注释", "`#` 到行尾"),
        ("文本", "双引号、单引号、中文引号都可定界；支持 \\n \\t \\\\ \\\" \\'"),
        ("全角标点", "代码区的全角标点自动转半角（字符串内部不转）"),
        ("标识符", "字母或下划线开头，后接字母/数字/下划线"),
        ("整数", "十进制；带指数报 E102"),
        ("小数", "如 1.5、0.25；小数点后必须有数字，不支持指数（E102）"),
    ];
    for (k, v) in lexical {
        m.push_str(&format!("| {} | {} |\n", k, v));
    }
    m.push('\n');

    m.push_str("## 语法速查\n\n");
    m.push_str("| 构造 | 写法 | 例子 |\n|---|---|---|\n");
    for s in SYNTAX {
        m.push_str(&format!(
            "| {} | `{}` | `{}` |\n",
            s.construct,
            s.form.replace('|', "\\|"),
            s.example.replace('\n', "<br>").replace('|', "\\|")
        ));
    }
    m.push('\n');

    m.push_str("### 产生式\n\n");
    m.push_str("```text\n");
    for g in GRAMMAR {
        m.push_str(g);
        m.push('\n');
    }
    m.push_str("```\n\n");

    m.push_str("## 运算符\n\n");
    m.push_str("优先级从低到高：`或` → `与` → `非` → 比较 → `+ -` → `* / // %` → 一元 `-` → `**` → 调用/下标。\n\n");
    m.push_str("- 二元：`+ - * / // % **`\n");
    m.push_str("- 比较：`== != < > <= >=`（可链式，如 `0 <= x < 10`）\n");
    m.push_str("- 一元：`-`（仅整数）、`非`\n\n");

    m.push_str("## 语义要点\n\n");
    m.push_str("| 主题 | 规则 |\n|---|---|\n");
    for s in SEMANTICS {
        m.push_str(&format!("| {} | {} |\n", s.topic, s.rule));
    }
    m.push('\n');

    m.push_str("## 硬限制\n\n");
    m.push_str("| 项目 | 上限 |\n|---|---|\n");
    for s in LIMITS {
        m.push_str(&format!("| {} | {} |\n", s.item, s.value));
    }
    m.push('\n');

    m.push_str("## 内建 API\n\n");
    m.push_str("内建函数直接可用，不需要 `导入`；`导入` 只用于板级原生模块（见下一节）。\n\n");
    m.push_str("| 名字 | id | 参数个数 | 说明 |\n|---|---|---|---|\n");
    for (id, name, min, max, doc) in &builtins {
        m.push_str(&format!("| `{}` | {} | {}..{} | {} |\n", name, id, min, max, doc));
    }
    m.push('\n');

    m.push_str("## 原生模块（板级）\n\n");
    m.push_str("`导入 模块` 是纯编译期声明，只允许写在模块顶层。导入后，模块里的原生函数名才能调用；名字拼错、参数个数不符、没导入就调用都会在编译期报错（E112/E113/E115），不会拖到运行时 E004。\n\n");
    for mod_spec in native_spec::MODULES {
        m.push_str(&format!(
            "### 模块 `{}`（原生表 `{}`）\n\n",
            mod_spec.name, mod_spec.table
        ));
        m.push_str("| 函数 | 参数个数 | 说明 |\n|---|---|---|\n");
        for e in mod_spec.entries {
            let arity = if e.min_args == e.max_args {
                format!("{}", e.min_args)
            } else {
                format!("{}..{}", e.min_args, e.max_args)
            };
            m.push_str(&format!("| `{}` | {} | {} |\n", e.name, arity, e.doc));
        }
        m.push('\n');
    }
    m.push_str("单一事实来源：`compiler/src/native_spec.rs`。`micc --emit-native-header` 生成 `runtime/include/jm_native_table.h`，板级与主机端口都展开同一份头；`micc --check-native` 在构建里比对，防止只改一侧。运行期仍由 `jm_native_register` 按名字绑定。\n\n");

    m.push_str("## 语言层任务（绿色）\n\n");
    m.push_str("`导入 任务` 后启动的是 VM 内部的协作式子任务，不走原生表：根任务之外最多 3 个子任务，按槽轮转，不抢占、无优先级。子任务值栈 96 槽、调用帧 24，**`尝试` 最多 4 层**（第 5 层报 E028）；根任务仍是 512 槽 / 32 帧 / 8 层。`删除(句柄)` 可取消未在运行的任务；子任务退出或删除后句柄/槽位暂不回收，`创建` 第 4 次报 E038。\n\n");
    m.push_str("| 函数 | 参数个数 | 说明 |\n|---|---|---|\n");
    for f in vm_spec::TASK_FNS {
        let doc = TASK_FN_DOCS
            .iter()
            .find(|(n, _)| *n == f.name)
            .map(|(_, d)| *d)
            .ok_or_else(|| format!("任务函数 {} 缺少说明，请补 TASK_FN_DOCS", f.name))?;
        m.push_str(&format!("| `{}` | {} | {} |\n", f.name, f.nargs, doc));
    }
    m.push('\n');
    m.push_str("带超时的等待会先探测一次，失败就只睡 1 tick 再重试，其他任务照常运行；到期仍不成功返回 `空`（接收）或 `假`（发送/信号量）。不带超时的形式仍是普通原生调用。\n\n");
    m.push_str("| 等待函数 | 带超时的实参个数 | 说明 |\n|---|---|---|\n");
    for w in vm_spec::WAIT_FNS {
        let doc = WAIT_FN_DOCS
            .iter()
            .find(|(n, _)| *n == w.name)
            .map(|(_, d)| *d)
            .ok_or_else(|| format!("等待函数 {} 缺少说明，请补 WAIT_FN_DOCS", w.name))?;
        m.push_str(&format!("| `{}` | {} | {} |\n", w.name, w.nargs, doc));
    }
    m.push('\n');
    m.push_str("单一事实来源：`compiler/src/vm_spec.rs`（名字与参数个数）和 `runtime/src/jm_vm.c`（语义）。\n\n");

    m.push_str("## 操作码\n\n");
    m.push_str("字节码由 `micc` 生成，运行时要与 `runtime/include/jm_opcodes.h` 逐条对齐。\n\n");
    m.push_str("| 值 | 名字 | 操作数 |\n|---|---|---|\n");
    for op in 0..=opcodes::OP_TASK_KILL {
        m.push_str(&format!(
            "| {} | `{}` | {} |\n",
            op,
            opcodes::op_name(op),
            operand_name(opcodes::operand(op))
        ));
    }
    m.push('\n');

    m.push_str("## 编译期错误码\n\n");
    m.push_str("| 码 | 含义 |\n|---|---|\n");
    for (code, meaning) in crate::error::COMPILE_CODES {
        m.push_str(&format!("| `{}` | {} |\n", code, meaning));
    }
    m.push('\n');

    m.push_str("## 运行期错误码\n\n");
    m.push_str("固件通过串口打印 `错误[码] 说明`（未捕获异常带抛出值的显示文本），码与 `jm_internal.h` 一致。\n\n");
    m.push_str("| 码 | 名称 | 含义 |\n|---|---|---|\n");
    for (name, value) in &runtime_errs {
        let meaning = RUNTIME_ERROR_DOCS
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.meaning)
            .unwrap_or("（缺说明）");
        m.push_str(&format!("| {} | `{}` | {} |\n", value, name, meaning));
    }
    m.push('\n');

    m.push_str("## 惯用法\n\n");
    for s in IDIOMS {
        m.push_str(&format!("### {}\n\n", s.title));
        m.push_str(&code_block(s.snippet));
        m.push('\n');
    }

    m.push_str("## 常见错误\n\n");
    for s in PITFALLS {
        m.push_str(&format!("### {}\n\n", s.title));
        m.push_str("错误写法：\n\n");
        m.push_str(&code_block(s.bad));
        m.push_str("\n正确写法：\n\n");
        m.push_str(&code_block(s.good));
        m.push_str(&format!("\n原因：{}\n\n", s.why));
    }

    m.push_str("## 可编译示例\n\n");
    for s in EXAMPLES {
        m.push_str(&format!("### {}（`{}`）\n\n", s.title, s.id));
        m.push_str(&code_block(s.source));
        m.push_str(&format!("\n输出：`{}`\n\n", s.prints));
    }

    Ok(m)
}

/// `--langcard`：写入两个入库文件。
pub fn write_files() -> Result<(), String> {
    let root = repo_root();
    let json_path = root.join("spec/langcard.json");
    let md_path = root.join("docs/语言卡.md");
    if let Some(p) = json_path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("创建目录失败：{}", e))?;
    }
    let json = render_json()?;
    let md = render_markdown()?;
    std::fs::write(&json_path, json).map_err(|e| format!("写不了 {}：{}", json_path.display(), e))?;
    std::fs::write(&md_path, md).map_err(|e| format!("写不了 {}：{}", md_path.display(), e))?;
    println!("已生成 {} 与 {}", md_path.display(), json_path.display());
    Ok(())
}

fn first_diff(expected: &str, actual: &str) -> String {
    for (i, (a, b)) in expected.lines().zip(actual.lines()).enumerate() {
        if a != b {
            return format!("第 {} 行不一致\n  期望：{}\n  实际：{}", i + 1, a, b);
        }
    }
    let (e, a) = (expected.lines().count(), actual.lines().count());
    format!("行数不一致：期望 {} 行，实际 {} 行", e, a)
}

/// `--check-langcard`：CI 防漂移。
pub fn check() -> Result<(), String> {
    let root = repo_root();
    let json_path = root.join("spec/langcard.json");
    let md_path = root.join("docs/语言卡.md");

    let want_json = render_json()?;
    let want_md = render_markdown()?;

    let got_json = read_or_fail(&json_path, "语言卡 JSON")?.replace("\r\n", "\n");
    if got_json != want_json {
        return Err(format!(
            "{} 与规范来源不一致：{}\n请运行 micc --langcard 重新生成",
            json_path.display(),
            first_diff(&want_json, &got_json)
        ));
    }
    let got_md = read_or_fail(&md_path, "语言卡 Markdown")?.replace("\r\n", "\n");
    if got_md != want_md {
        return Err(format!(
            "{} 与规范来源不一致：{}\n请运行 micc --langcard 重新生成",
            md_path.display(),
            first_diff(&want_md, &got_md)
        ));
    }

    // 卡内每个示例都必须能编过。
    for ex in EXAMPLES {
        crate::compile_source(ex.source)
            .map_err(|e| format!("示例 {} 编不过：{}", ex.id, e))?;
    }

    // 编译期错误码：源码里出现的码必须登记，登记的码必须被用到。
    let src_dir = root.join("compiler/src");
    let mut used: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(&src_dir)
        .map_err(|e| format!("读不了 {}：{}", src_dir.display(), e))?;
    for ent in entries {
        let ent = ent.map_err(|e| format!("读目录失败：{}", e))?;
        if ent.path().extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        // 本文件自身含错误码字面量清单，跳过它，避免自证。
        if ent.file_name().to_string_lossy() == "langcard.rs" {
            continue;
        }
        let text = read_or_fail(&ent.path(), "编译器源码")?;
        for code in [
            "E102", "E103", "E104", "E105", "E106", "E107", "E108", "E109", "E110", "E111",
            "E112", "E113", "E114", "E115", "E116", "E117", "E118", "E119", "E120", "E121",
            "E122", "E123", "E124", "E125", "E126", "E127", "E128", "E129", "E130", "E131",
            "E132", "E133", "E134", "E135", "E136", "E137", "E138",
        ] {
            if text.contains(&format!("\"{}\"", code)) && !used.iter().any(|u| u == code) {
                used.push(code.to_string());
            }
        }
    }
    for (code, _) in crate::error::COMPILE_CODES {
        if !used.iter().any(|u| u == code) {
            return Err(format!(
                "COMPILE_CODES 登记的 {} 在编译器源码里没有被使用，可能已过时",
                code
            ));
        }
    }
    for code in &used {
        if !crate::error::COMPILE_CODES.iter().any(|(c, _)| c == code) {
            return Err(format!(
                "编译器源码里用了未登记的编译错误码 {}，请补进 compiler/src/error.rs 的 COMPILE_CODES",
                code
            ));
        }
    }

    // 运行期错误码：头文件里的每个 E_* 都必须有说明。
    for (name, _) in c_runtime_errors()? {
        if !RUNTIME_ERROR_DOCS.iter().any(|d| d.name == name) {
            return Err(format!(
                "运行期错误 {} 缺少说明，请补进 compiler/src/langcard.rs 的 RUNTIME_ERROR_DOCS",
                name
            ));
        }
    }
    for doc in RUNTIME_ERROR_DOCS {
        if !c_runtime_errors()?.iter().any(|(n, _)| *n == doc.name) {
            return Err(format!(
                "RUNTIME_ERROR_DOCS 里的 {} 在 jm_internal.h 里已不存在",
                doc.name
            ));
        }
    }

    println!("语言卡通过校验：{} 与 {} 均与规范来源一致，{} 个示例可编译", json_path.display(), md_path.display(), EXAMPLES.len());
    Ok(())
}

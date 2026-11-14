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
    BuiltinDoc {
        name: "集合",
        min_args: 0,
        max_args: 0,
        doc: "创建空集合。元素限不可变值（整数/文本/空/真/假/小数），容器与实例报 E019。",
    },
    BuiltinDoc {
        name: "集合加入",
        min_args: 2,
        max_args: 2,
        doc: "集合加入(集合, 值)：新增返回 真，已存在返回 假（不报错）；打印为 `集合{...}`。",
    },
    BuiltinDoc {
        name: "集合有",
        min_args: 2,
        max_args: 2,
        doc: "集合有(集合, 值)：在集合里返回 真，否则 假。",
    },
    BuiltinDoc {
        name: "集合删除",
        min_args: 2,
        max_args: 2,
        doc: "集合删除(集合, 值)：删除并返回 真，不存在返回 假。",
    },
    BuiltinDoc {
        name: "字节",
        min_args: 1,
        max_args: 1,
        doc: "文本 → 原始 UTF-8 字节；整数列表 → 字节串（元素必须是 0..255），其他类型报 E010。",
    },
    BuiltinDoc {
        name: "字节文本",
        min_args: 1,
        max_args: 1,
        doc: "按 UTF-8 把字节拷回文本；不做合法性校验，非法序列由调用方负责。",
    },
    BuiltinDoc {
        name: "字节列表",
        min_args: 1,
        max_args: 1,
        doc: "字节 → 0..255 的整数列表。",
    },
    BuiltinDoc {
        name: "字节十六进制",
        min_args: 1,
        max_args: 1,
        doc: "字节 → 大写十六进制文本，字节之间一个空格，如 `00 FF 1A`；空字节返回空文本。",
    },
    BuiltinDoc {
        name: "十六进制字节",
        min_args: 1,
        max_args: 1,
        doc: "十六进制文本 → 字节；允许空格/制表/换行与大小写，位数必须是偶数，否则报 E010。",
    },
    BuiltinDoc {
        name: "字节取",
        min_args: 2,
        max_args: 2,
        doc: "字节取(字节, 下标)：返回 0..255 的整数；支持负下标（-1 是最后一字节），越界报 E014。",
    },
    BuiltinDoc {
        name: "子串",
        min_args: 2,
        max_args: 3,
        doc: "子串(文本, 起[, 止])：按码点、左闭右开；可负（-1 是最后一个），越界夹取，起 ≥ 止返回空文本；下标不是整数报 E013。",
    },
    BuiltinDoc {
        name: "查找",
        min_args: 2,
        max_args: 2,
        doc: "查找(文本, 子串) 返回首次出现的码点下标；查找(列表, 值) 返回首个相等元素的下标；找不到返回 -1。文本的空子串返回 0。",
    },
    BuiltinDoc {
        name: "替换",
        min_args: 3,
        max_args: 3,
        doc: "替换(文本, 旧, 新)：把所有不重叠的「旧」换成「新」；旧为空时原样返回。",
    },
    BuiltinDoc {
        name: "分割",
        min_args: 2,
        max_args: 2,
        doc: "分割(文本, 分隔符)：返回文本列表；连续分隔符产生空文本；分隔符为空时按码点逐个切。",
    },
    BuiltinDoc {
        name: "连接",
        min_args: 1,
        max_args: 2,
        doc: "连接(列表[, 分隔符])：把文本列表拼起来，分隔符默认空文本；列表元素不是文本报 E010。",
    },
    BuiltinDoc {
        name: "去空白",
        min_args: 1,
        max_args: 1,
        doc: "去掉两端的 ASCII 空白（空格、制表、回车、换行），中间不动。",
    },
    BuiltinDoc {
        name: "转大写",
        min_args: 1,
        max_args: 1,
        doc: "把 ASCII 小写字母转成大写，其他字符（含中文）原样保留。",
    },
    BuiltinDoc {
        name: "转小写",
        min_args: 1,
        max_args: 1,
        doc: "把 ASCII 大写字母转成小写，其他字符（含中文）原样保留。",
    },
    BuiltinDoc {
        name: "追加",
        min_args: 2,
        max_args: 2,
        doc: "追加(列表, 值)：原地在末尾加一个元素，返回 空。",
    },
    BuiltinDoc {
        name: "插入",
        min_args: 3,
        max_args: 3,
        doc: "插入(列表, 下标, 值)：原地插入到下标前；可负（-1 是最后一个之前），越界夹取，返回 空。",
    },
    BuiltinDoc {
        name: "移除",
        min_args: 2,
        max_args: 2,
        doc: "移除(列表, 下标)：移走并返回该元素；可负，越界报 E014，下标不是整数报 E013。",
    },
    BuiltinDoc {
        name: "切片",
        min_args: 2,
        max_args: 3,
        doc: "切片(列表, 起[, 止])：返回新列表，左闭右开、可负、越界夹取；元素按引用共享。",
    },
    BuiltinDoc {
        name: "反转",
        min_args: 1,
        max_args: 1,
        doc: "反转(列表)：返回元素顺序相反的新列表，不改原列表。",
    },
    BuiltinDoc {
        name: "排序",
        min_args: 1,
        max_args: 1,
        doc: "排序(列表)：返回升序新列表（稳定插入排序）；元素须全为整数/小数（按数值）或全为文本（按 UTF-8 字节序），混类型报 E010。",
    },
    BuiltinDoc {
        name: "键列表",
        min_args: 1,
        max_args: 1,
        doc: "键列表(字典)：按插入顺序返回全部键的新列表。",
    },
    BuiltinDoc {
        name: "包含",
        min_args: 2,
        max_args: 2,
        doc: "包含(字典, 键)：键存在返回 真，否则 假。",
    },
    BuiltinDoc {
        name: "合并",
        min_args: 2,
        max_args: 2,
        doc: "合并(字典, 字典)：返回新字典，先放第一个的键值再放第二个（同键以后者为准），两边都不改。",
    },
    BuiltinDoc {
        name: "移除键",
        min_args: 2,
        max_args: 2,
        doc: "移除键(字典, 键)：删掉并返回对应的值；键不存在返回 空。",
    },
    BuiltinDoc {
        name: "默认值",
        min_args: 3,
        max_args: 3,
        doc: "默认值(字典, 键, 默认)：键存在返回它的值，否则返回第三个参数。",
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
        construct: "匿名函数",
        form: "令 名字 = 函数(参数, ...): 表达式 / 缩进块",
        example: "令 平方 = 函数(x): x * x\n打印(平方(5))",
    },
    SyntaxRow {
        construct: "类型标注（--strict-type 校验）",
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
        construct: "断言",
        form: "断言 条件[, 消息]",
        example: "断言 n > 0, \"n 必须为正\"",
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
        form: "[最终] 类 名字 [继承 父类]:\n    令 字段 = 默认值\n    最终 函数 方法(...): ...\n    函数 初始化(参数): ...",
        example: "类 计数器:\n    令 值 = 0\n    最终 函数 读():\n        返回 本.值\n    函数 加(n):\n        本.值 += n\n\n最终 类 定值 继承 计数器:\n    函数 加倍():\n        返回 本.值 * 2",
    },
    SyntaxRow {
        construct: "私有",
        form: "[私有] 令 / 函数 / 类 / 枚举（只能写在模块顶层；micc pack 跨包时私有声明不可见）",
        example: "私有 函数 内部():\n    返回 1\n\n私有 类 缓存:\n    令 值 = 0",
    },
    SyntaxRow {
        construct: "新建 / 成员",
        form: "新建 类名(实参...)；跨包 新建 包名.类名(...)（micc pack）；对象.字段；对象.方法(...)；本（方法内）；超.方法(...)（类方法内调父类实现）",
        example: "令 c = 新建 计数器(5)\nc.加(3)\n打印(c.值)",
    },
    SyntaxRow {
        construct: "枚举",
        form: "枚举 名字:\n    成员\n    成员 = 显式整数值",
        example: "枚举 状态:\n    空闲\n    运行 = 10\n    暂停",
    },
    SyntaxRow {
        construct: "匹配",
        form: "匹配 表达式:\n    情形 整数 / 起..止 / 枚举.成员 / 逗号分隔多个: ...\n    情形 其他: ...",
        example: "匹配 颜色.绿:\n    情形 颜色.红, 颜色.蓝:\n        打印(\"冷\")\n    情形 其他:\n        打印(\"其他\")",
    },
    SyntaxRow {
        construct: "匹配表达式",
        form: "令 名字 = 匹配 表达式:\n    情形 模式: 表达式\n    情形 其他: 表达式",
        example: "令 档位 = 匹配 7:\n    情形 0: \"零\"\n    情形 1..9: \"个位\"\n    情形 其他: \"大数\"\n打印(档位)",
    },
    SyntaxRow {
        construct: "集合",
        form: "集合()；集合加入(集, 值)；集合有(集, 值)；集合删除(集, 值)",
        example: "令 s = 集合()\n集合加入(s, 1)\n集合加入(s, 1)\n打印(长度(s), s)",
    },
    SyntaxRow {
        construct: "字节",
        form: "字节(文本或整数列表)；字节文本/字节列表/字节十六进制/十六进制字节；字节取(字节, 下标)",
        example: "令 b = 字节(\"砾\")\n打印(长度(b), 字节十六进制(b))\n打印(字节取(b, 0))",
    },
    SyntaxRow {
        construct: "位运算",
        form: "a & b；a | b；a ^ b；a << n；a >> n；~a（仅整数）",
        example: "打印(5 & 3, 5 | 3, 5 ^ 3, ~0)\n打印(1 << 4, -8 >> 1)",
    },
    SyntaxRow {
        construct: "文本插值",
        form: "文本里的 {表达式} 会套 文本() 拼进去；字面花括号写 \\{ \\} 或 {{ }}",
        example: "令 n = 3\n打印(\"n = {n}\")",
    },
    SyntaxRow {
        construct: "文本函数",
        form: "子串(文本, 起[, 止])；查找(文本, 子串)；替换(文本, 旧, 新)；分割(文本, 分隔符)；连接(列表[, 分隔符])；去空白(文本)；转大写(文本)；转小写(文本)",
        example: "导入 文本\n令 s = \"你好，砾世界\"\n打印(子串(s, 3), 查找(s, \"砾\"), 替换(s, \"砾\", \"石\"))\n打印(分割(\"甲,乙\", \",\"), 连接([\"甲\", \"乙\"], \"-\"))",
    },
    SyntaxRow {
        construct: "列表函数",
        form: "追加(列表, 值)；插入(列表, 下标, 值)；移除(列表, 下标)；切片(列表, 起[, 止])；反转(列表)；排序(列表)；查找(列表, 值)",
        example: "导入 列表\n令 数 = [3, 1, 2]\n追加(数, 4)\n打印(排序(数), 切片(数, 1), 移除(数, 0))",
    },
    SyntaxRow {
        construct: "字典函数",
        form: "键列表(字典)；包含(字典, 键)；合并(字典, 字典)；移除键(字典, 键)；默认值(字典, 键, 默认)",
        example: "导入 字典\n令 人 = {\"名字\": \"砾\", \"版本\": 2}\n打印(键列表(人), 包含(人, \"名字\"))\n打印(默认值(人, \"作者\", \"未知\"))",
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
    "statement := import | private_decl | class | enum | let | assign | if | while | repeat | foreach | match | funcdef | return | break | continue | assert | throw | try | expr_stmt",
    "private_decl := '私有' ( let | funcdef | class | enum )",
    "class := [ '私有' ] [ '最终' ] '类' IDENT [ '继承' IDENT ] ':' NEWLINE INDENT { class_member } DEDENT",
    "class_member := '令' IDENT '=' expr NEWLINE | funcdef | '最终' funcdef",
    "super_call := '超' '.' IDENT '(' [ expr { ',' expr } ] ')'",
    "enum := [ '私有' ] '枚举' IDENT ':' NEWLINE INDENT { IDENT [ '=' INT ] NEWLINE } DEDENT",
    "match := '匹配' expr ':' NEWLINE INDENT { '情形' patterns block } DEDENT",
    "match_expr := '匹配' expr ':' NEWLINE INDENT { '情形' patterns ':' expr NEWLINE } DEDENT",
    "patterns := pattern { ',' pattern }",
    "pattern := INT [ '..' INT ] | IDENT '.' IDENT | IDENT '.' IDENT '.' IDENT | '其他'",
    "import := '导入' IDENT",
    "let := '令' IDENT '=' expr",
    "assign := lvalue '=' expr | lvalue AUGOP expr",
    "lvalue := IDENT | postfix '[' expr ']' | postfix '.' IDENT",
    "if := '如果' expr block { '否则如果' expr block } [ '否则' block ]",
    "while := '当' expr block",
    "repeat := '循环' expr [ '次' ] block",
    "foreach := '遍历' IDENT '在' expr block",
    "funcdef := '函数' IDENT params [ '->' IDENT { '.' IDENT } ] func_body",
    "lambda := '函数' params [ '->' IDENT { '.' IDENT } ] func_body",
    "params := '(' [ param { ',' param } ] ')'",
    "func_body := block | ':' expr",
    "param := IDENT [ ':' IDENT { '.' IDENT } ]",
    "return := '返回' [ expr ]",
    "assert := '断言' expr [ ',' expr ]",
    "throw := '抛出' expr",
    "try := '尝试' block '捕获' [ IDENT ] block",
    "block := ':' NEWLINE INDENT { statement } DEDENT",
    "expr := or",
    "or := and { '或' and }",
    "and := not { '与' not }",
    "not := '非' not | compare",
    "compare := bor { ('=='|'!='|'<'|'>'|'<='|'>=') bor }",
    "bor := bxor { '|' bxor }",
    "bxor := band { '^' band }",
    "band := shift { '&' shift }",
    "shift := add { ('<<'|'>>') add }",
    "add := mul { ('+'|'-') mul }",
    "mul := unary { ('*'|'/'|'//'|'%') unary }",
    "unary := ('-'|'~') unary | power",
    "power := postfix [ '**' unary ]",
    "postfix := primary { '(' args ')' | '[' expr ']' | '.' IDENT [ '(' args ')' ] }",
    "primary := INT | STR | '真' | '假' | '空' | IDENT | '本' | '新建' IDENT { '.' IDENT } '(' args ')' | '(' expr ')' | list | dict | lambda | match_expr",
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
    SemanticRow { topic: "标准库模块", rule: "`导入 文本` / `导入 列表` / `导入 字典` 后，对应函数名才进入作用域：文本（子串/查找/替换/分割/连接/去空白/转大写/转小写）、列表（追加/插入/移除/切片/反转/排序/查找）、字典（键列表/包含/合并/移除键/默认值）。未导入就使用报 E112；`查找` 同时属于文本与列表，导入任一即可。用户自己声明的同名全局优先，不会被模块规则拦住。" },
    SemanticRow { topic: "绿色任务", rule: "`导入 任务` 后用 `创建(函数)` 起一个无参子任务（根任务之外最多 3 个），`让出()` 主动换出，`延时(毫秒)` 睡到指定时刻；`删除(句柄)` 可取消一个未在运行的任务。调度是协作式、按槽轮转，不抢占、无优先级。子任务值栈 96 槽、调用帧 24、`尝试` 最多 4 层（第 5 层报 E028）；根任务仍是 512 槽 / 32 帧 / 8 层。子任务退出的槽位暂不复用。" },
    SemanticRow { topic: "任务等待", rule: "带超时的 `队列接收(队, 毫秒)`、`队列发送(队, 值, 毫秒)`、`信号量等待(信号量, 毫秒)` 会先探测一次，失败就只睡 1 tick 再重试，其他任务照常运行；到期仍不成功返回 `空`（接收）或 `假`（发送/信号量）。不带超时的形式仍是普通原生调用。" },
    SemanticRow { topic: "作用域", rule: "函数内的 `令` 是局部槽；内层函数读取外层局部变量时会按值捕获（定义时快照、只读）；未在函数内声明而同名的读取会落到模块级全局。" },
    SemanticRow { topic: "函数", rule: "模块级函数与闭包都是一等值：可以赋值、传参、作为返回值；实参按值传递；形参与实参个数必须一致；调用层数上限 32。" },
    SemanticRow { topic: "可见性", rule: "模块顶层的 `令/函数/类/枚举` **默认公开**；在声明前加 `私有` 就只在包内可见，跨包引用报 E128。`私有` 只能写在模块顶层（类体或函数体里报 E144），不能重复（E144）；私有名字写进 `[导出] 名字/类型` 也报 E144。不写 `[导出]` 时所有公开名字与类型都可跨包；写了 `[导出]` 就按清单过滤。类方法/字段没有单独的公开/私有修饰，随类一起可见。" },
    SemanticRow { topic: "类型标注", rule: "`函数 名(参数: 类型) -> 类型` 的标注默认**只记录、不检查**，行为与无标注完全一致。加 `--strict-type`（`micc pack` 同）后启用 v1 校验：① 类型名必须是 整数/小数/文本/布尔/空/列表/字典/字节/集合/范围/函数，或已定义的类/枚举；`micc pack` 里还可以写 `包名.类名`（对方须在 [导出] 类型 里列出），否则 E143；② 在能静态看出实参类型时（字面量、`新建 C`、列表/字典字面量、匿名函数、枚举成员、`字节(...)`/`集合()`/`范围(...)`，以及带返回标注的用户函数调用）检查直接函数调用、`本.方法(...)`、`超.方法(...)` 与 `新建 C(...)` 的实参；③ 检查 `返回` 值。兼容规则：整数可当小数，`空` 可给引用类型，子类实例可给父类标注，枚举值可给整数标注。变量、字段、下标、内建返回值与“所有分支都返回”等复杂情况**不推断、不报错**。严格模式只影响编译期，不改字节码。" },
    SemanticRow { topic: "闭包", rule: "内层函数读取外层局部变量即按值捕获（定义那一刻的快照，之后外层再改也不影响）；捕获变量只读（赋值报 E117），不能在函数里捕获自己的名字（E117）；单个函数捕获上限 255（E116）。" },
    SemanticRow { topic: "匿名函数", rule: "`函数(参数): 表达式`（单行）或 `函数(参数):` + 缩进块，本身就是表达式，可以赋给变量、传参、返回、立刻调用 `(函数(x): x + 1)(5)`。它与具名闭包完全同一套机制：按值捕获外层局部变量、捕获变量只读、同样受 255 个捕获上限约束；没有名字，所以不能递归调用自己（要递归就用具名函数）。函数体单行写法也可用在具名函数上：`函数 平方(x): x * x`。" },
    SemanticRow { topic: "字符串", rule: "单引号、双引号、中文引号都可定界；反斜杠转义支持 \\n \\t \\\\ \\\" \\'。文本按 UTF-8 存。字符串里的 `{表达式}` 是**文本插值**：编译期展开成 `+` 拼接，并对每个插值自动套 `文本()`（不引入运行期格式化）；字面花括号写 `\\{` `\\}`（或 `{{` `}}`），单独的 `}` 按字面处理。插值表达式里不要出现右花括号。" },
    SemanticRow { topic: "文本库", rule: "文本函数全部按 **UTF-8 码点**处理、返回新文本，不改原值。`子串(s, 起[, 止])` 左闭右开、可负、越界夹取、起 ≥ 止为空；`查找(s, 子串)` 返回首次出现的码点下标，找不到 -1，空子串返回 0；`替换(s, 旧, 新)` 替换所有不重叠出现，旧为空原样返回；`分割(s, 分隔符)` 返回文本列表，连续分隔符产生空文本，分隔符为空按码点切；`连接(列表[, 分隔符])` 只拼接文本元素（否则 E010）；`去空白` 只去两端 ASCII 空白；`转大写/转小写` 只转 ASCII 字母，中文原样。参数类型不符报 E010，子串下标不是整数报 E013，参数个数不符报 E012。" },
    SemanticRow { topic: "列表库", rule: "`追加(列表, 值)` 原地末尾追加；`插入(列表, 下标, 值)` 原地插入到下标前（可负、越界夹取）；两者返回 空。`移除(列表, 下标)` 移走并返回该元素（可负，越界 E014，下标非整数 E013）；名字用「移除」是为了避开任务模块的 `删除(句柄)`。`切片(列表, 起[, 止])` 与 `反转(列表)`、`排序(列表)` 都返回**新列表**，元素按引用共享，不改原列表。`排序` 用稳定插入排序，元素须全为整数/小数（按数值）或全为文本（按 UTF-8 字节序），混类型报 E010。`查找(列表, 值)` 返回首个相等元素的下标，找不到 -1。" },
    SemanticRow { topic: "字典库", rule: "`键列表(字典)` 按插入顺序返回键的新列表；`包含(字典, 键)` 返回 真/假；`合并(字典, 字典)` 返回**新字典**（同键以后一个为准，两边都不改）；`移除键(字典, 键)` 删掉并返回值，键不存在返回 空（名字用「移除键」是为了避开任务模块的 `删除`）；`默认值(字典, 键, 默认)` 键存在返回值、否则返回默认。键仍限不可变值，容器/实例当键走 E019。" },
    SemanticRow { topic: "字典键", rule: "键必须是整数/文本/空/真/假这类不可变值；列表、字典、函数、实例不能当键。" },
    SemanticRow { topic: "集合", rule: "`集合()` 创建无序去重容器，用 `集合加入/集合有/集合删除` 操作；元素限不可变值（整数/文本/空/真/假/小数），容器、集合与实例报 E019。内部是线性数组：`长度` 可用、`遍历` 按插入顺序、**不支持下标**。打印为 `集合{1, 2}`；相等比较按身份。" },
    SemanticRow { topic: "字节", rule: "字节是不可变**原始字节串**，与文本不同：`长度`、`遍历`、下标都按字节而不是 UTF-8 码点。由 `字节(文本)` 或 `字节(整数列表)` 构造（无字面量，不改 JMB）；`字节取(字节, 下标)` 返回 0..255 整数，`字节文本` 按 UTF-8 拷回文本、**不做合法性校验**。`+` 支持字节+字节拼接；相等按内容逐字节比较；打印为 `字节<00 FF 1A>`。列表元素越界、十六进制非法都报 E010。" },
    SemanticRow { topic: "位运算", rule: "`& | ^ << >> ~` 只接受整数（小数/文本/容器报 E010）。按 **32 位补码**计算，结果必须落在 29 位范围 `-268435456..268435455`，否则报 E009——实际上只有 `<<` 会溢出，`& | ^ ~` 不会。移位次数必须是 `0..31`，负数或 ≥32 报 E010；`>>` 是**算术右移**（`-8 >> 1 = -4`），不提供逻辑右移。`~x` 在编译期降级为 `x ^ -1`。位运算整体高于比较：`a & b == c` 等于 `(a & b) == c`。" },
    SemanticRow { topic: "类", rule: "`类` 只能写在模块顶层，类名全局唯一；类体只允许字段（`令 名 = 默认值`，默认值必填且不能引用 `本`）与方法。`类 子 继承 父:` 是**单继承**：父类必须写在同一模块里（可以写在子类之后），字段按「父类字段在前」扁平继承、沿父链不能重名；方法同名即覆写，运行期沿父链查找，父方法里的 `本.方法(...)` 会调到子类覆写版本。子类构造先自动执行父类初始化（`LOAD_LOCAL 0 / CALL_INIT 父 0 / POP`），再跑本类字段默认值与 `初始化`；因此**父类 `初始化` 必须零形参**（E140）。`新建 C(...)` 有 `初始化` 时实参个数必须与它一致，否则只能零实参；在 `micc pack` 里，依赖包的类可以写 `新建 包名.类名(...)`（默认公开；标 `私有 类` 则不可跨包；对方写了 `[导出] 类型` 时还要在清单里列出），但**跨包继承不支持**（父类仍要求同一模块，E140）；`初始化` 里禁止 `返回`，构造结果固定是实例本身。方法内 `本` 是只读绑定（不能 `本 = ...`），`本.字段` 读写与 `对象.字段` 相同。`超.方法(...)` 在编译期解析到父类实现（沿父链找同名方法）并静态调用，不再按实例类派发；`超` 只能这样调用，不能当值、不能赋值，也不能写 `超.初始化`（父构造已自动执行）。`最终 类` 不能被继承；类里的 `最终 函数 方法` 不能被任何子类覆写，但可以正常继承和用 `超` 调用；`初始化` 不能标记最终。`最终` 只做编译期检查，不做静态分派优化。未声明字段/方法报 E035/E037，对非实例取成员报 E036；`本.未知` 与 `本.方法` 当值取是编译期 E122/E123。继承相关错误 E140，`超` 用法错误 E141，`最终` 用法错误 E142，字段总数（含继承）超过 255 报 E105。" },
    SemanticRow { topic: "枚举", rule: "`枚举 名字:` 下每行一个成员，可以写显式值 `成员 = 10`（可负）；没写时取**上一个成员的取值 +1**，第一个默认 0，所以 `空闲 / 运行 = 10 / 暂停` 得到 0 / 10 / 11。成员写作 `枚举名.成员`，编译期直接折成整数，不产生运行期对象、不进名字表。枚举名只能写在模块顶层，不能与顶层变量/函数/类、内建函数或原生函数重名（E139）；成员名不能重复、取值也不能重复（E139），取值超出 29 位整数范围报 E110。变量与参数也不能与枚举重名。枚举默认**公开**（与 令/函数/类 一致）；要包内私有就写 `私有 枚举`，跨包引用报 E128。跨包读枚举写 `包名.枚举名.成员`（含 `匹配` 情形）；如果对方写了 `[导出] 类型`，枚举还需在清单里列出。合并时依赖包的枚举会改名为 `包名::枚举名`。不同包可以同名。" },
    SemanticRow { topic: "匹配", rule: "`匹配 表达式:` 后跟若干 `情形` 分支，模式可以是整数字面量、闭区间 `起..止`（两端都含）、枚举成员，或逗号分隔的多个模式；可选的 `情形 其他:` 必须最后。主体只求值一次，按声明顺序用 `==` 与两次边界比较判定，命中即执行该分支再跳出。语句形式无返回值、不要求覆盖所有取值；分支值与区间不能重复或重叠（E139）。" },
    SemanticRow { topic: "匹配表达式", rule: "`匹配` 也可以当表达式：每个分支体是单行表达式，必须有 `情形 其他:` 兜底；命中分支的值放进临时槽后压栈，可直接赋给变量、`返回` 或参与外层表达式。能静态看出的字面量分支必须同类型（整数/小数/文本/布尔/空，混用报 E139），非字面量分支不做静态检查。因为要换行缩进，它只能写在语句级表达式位置（`令`、`返回` 的右边等），不能塞进括号里。" },
    SemanticRow { topic: "实例", rule: "实例是普通堆对象：可进列表/字典值、可 `抛出` 并被 `捕获`、可被闭包捕获（含捕获 `本`）；相等比较按身份；`长度`/下标/遍历不适用；native 只能把实例当入参透传，不能构造或返回。" },
    SemanticRow { topic: "迭代", rule: "可遍历：列表、文本（按字符）、字典（按键）、范围。" },
    SemanticRow { topic: "断言", rule: "`断言 条件[, 消息]`：条件为真时什么都不做；为假时抛出消息（省略消息则抛文本 `\"断言失败\"`）。等价于 `如果 非 条件: 抛出 ...`，因此可被 `尝试`/`捕获` 捕获，未捕获时报 `错误[E027] 未捕获的异常：<消息>`。条件与消息只在需要时求值。编译时加 `--no-assert`（`micc pack` 同）会把整句消除：条件和消息都不求值、不生成字节码。" },
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
    LimitRow { item: "类", value: "255 个；每类字段 255 个（含继承）、方法 255 个" },
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
        title: "只剩 `用` 不支持；方法仍是“调用”不是值",
        bad: "用 甲\n令 f = c.读",
        good: "最终 类 计数器:\n    令 值 = 0\n    函数 读():\n        返回 本.值\n令 c = 新建 计数器()\n打印(c.读())",
        why: "`最终 类` / `最终 函数` 已经支持（编译期检查继承与覆写），但 `用` 仍是 v0 明确拒绝的关键字（E103）。方法只能 `对象.方法(...)` 调用；把方法当值取（`令 f = c.读`）拿不到方法，`本.读` 会直接报 E123，普通对象则运行期报 E035。",
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
        id: "lambda",
        title: "匿名函数与立即调用",
        source: "令 平方 = 函数(x): x * x\n打印(平方(5))\n\n函数 应用(f, x):\n    返回 f(x)\n打印(应用(函数(v): v * 10, 6))\n\n令 加 = 函数(x): 函数(y): x + y\n打印(加(2)(3))\n打印((函数(x): x + 1)(41))",
        prints: "25\n60\n5\n42",
    },
    Example {
        id: "exception",
        title: "抛出与捕获",
        source: "函数 取数(n):\n    如果 n < 0:\n        抛出 \"负数\"\n    返回 n\n\n尝试:\n    打印(取数(-1))\n捕获 原因:\n    打印(\"捕获\", 原因)",
        prints: "捕获 负数",
    },
    Example {
        id: "assert",
        title: "断言与断言失败",
        source: "函数 倒数(n):\n    断言 n != 0, \"不能为零\"\n    返回 10 / n\n\n尝试:\n    打印(倒数(2))\n    打印(倒数(0))\n捕获 原因:\n    打印(\"捕获\", 原因)",
        prints: "5\n捕获 不能为零",
    },
    Example {
        id: "interp",
        title: "文本插值",
        source: "令 名字 = \"砾\"\n令 版本 = 2\n打印(\"你好，{名字}！v{版本}\")\n打印(\"1 + 2 = {1 + 2}\")\n打印(\"花括号：\\{ \\}\")",
        prints: "你好，砾！v2\n1 + 2 = 3\n花括号：{ }",
    },
    Example {
        id: "text-lib",
        title: "文本函数",
        source: "导入 文本\n令 s = \"你好，砾世界\"\n打印(子串(s, 0, 2), 子串(s, 3), 子串(s, -2))\n打印(查找(s, \"砾\"), 查找(s, \"石\"))\n打印(替换(s, \"砾\", \"石\"))\n打印(分割(\"甲,乙\", \",\"), 连接([\"甲\", \"乙\"], \"-\"))\n打印(\"[\" + 去空白(\"  你 好  \") + \"]\")\n打印(转大写(\"abc砾\"), 转小写(\"ABC砾\"))",
        prints: "你好 砾世界 世界\n3 -1\n你好，石世界\n['甲', '乙'] 甲-乙\n[你 好]\nABC砾 abc砾",
    },
    Example {
        id: "list-lib",
        title: "列表函数",
        source: "导入 列表\n令 数 = [3, 1, 2]\n追加(数, 4)\n插入(数, 0, 9)\n打印(数, 排序(数))\n打印(切片(数, 1, 3), 切片(数, -2), 切片(数, 1, 100))\n打印(移除(数, 0), 数)\n打印(反转(数), 数)\n打印(排序([3, 1.5, 2]), 排序([\"丙\", \"甲\", \"乙\"]))\n打印(查找([10, 20, 30], 20), 查找([10, 20], 99))",
        prints: "[9, 3, 1, 2, 4] [1, 2, 3, 4, 9]\n[3, 1] [2, 4] [3, 1, 2, 4]\n9 [3, 1, 2, 4]\n[4, 2, 1, 3] [3, 1, 2, 4]\n[1.5, 2, 3] ['丙', '乙', '甲']\n1 -1",
    },
    Example {
        id: "dict-lib",
        title: "字典函数",
        source: "导入 字典\n令 人 = {\"名字\": \"砾\", \"版本\": 2}\n打印(键列表(人))\n打印(包含(人, \"名字\"), 包含(人, \"作者\"))\n打印(默认值(人, \"版本\", 0), 默认值(人, \"作者\", \"未知\"))\n打印(移除键(人, \"版本\"), 人)\n打印(移除键(人, \"版本\"))\n令 合成 = 合并(人, {\"作者\": \"基石\", \"版本\": 3})\n打印(合成, 人)",
        prints: "['名字', '版本']\n真 假\n2 未知\n2 {'名字': '砾'}\n空\n{'名字': '砾', '作者': '基石', '版本': 3} {'名字': '砾'}",
    },
    Example {
        id: "board-led",
        title: "板级点灯（导入原生模块）",
        source: "导入 引脚\n引脚写(13, 0)\n等待毫秒(400)\n引脚写(13, 1)",
        prints: "（板上执行，无主机输出）",
    },
    Example {
        id: "enum-match",
        title: "枚举与匹配",
        source: "枚举 颜色:\n    红\n    绿\n    蓝\n\n令 c = 颜色.绿\n匹配 c:\n    情形 颜色.红:\n        打印(\"红\")\n    情形 颜色.绿:\n        打印(\"绿\")\n    情形 其他:\n        打印(\"其他\")\n打印(颜色.蓝)",
        prints: "绿\n2",
    },
    Example {
        id: "match-ranges",
        title: "匹配区间、多值与表达式",
        source: "函数 等级(分):\n    匹配 分:\n        情形 90..100:\n            返回 \"优\"\n        情形 60..89:\n            返回 \"及格\"\n        情形 其他:\n            返回 \"不及格\"\n\n函数 档位(n):\n    返回 匹配 n:\n        情形 0: \"零\"\n        情形 1, 2: \"很少\"\n        情形 3..9: \"一些\"\n        情形 其他: \"很多\"\n\n打印(等级(95), 等级(70), 等级(30))\n打印(档位(0), 档位(2), 档位(7), 档位(42))",
        prints: "优 及格 不及格\n零 很少 一些 很多",
    },
    Example {
        id: "enum-values",
        title: "枚举显式取值",
        source: "枚举 状态:\n    空闲\n    运行 = 10\n    暂停\n    停止 = -1\n\n打印(状态.空闲, 状态.运行, 状态.暂停, 状态.停止)\n匹配 状态.暂停:\n    情形 状态.运行, 状态.暂停:\n        打印(\"进行中\")\n    情形 其他:\n        打印(\"未知\")",
        prints: "0 10 11 -1\n进行中",
    },
    Example {
        id: "set",
        title: "集合去重",
        source: "令 s = 集合()\n集合加入(s, 1)\n集合加入(s, 2)\n集合加入(s, 1)\n打印(长度(s))\n打印(s)\n打印(集合有(s, 2), 集合删除(s, 1), 长度(s))",
        prints: "2\n集合{1, 2}\n真 真 1",
    },
    Example {
        id: "bytes",
        title: "字节与十六进制",
        source: "令 b = 字节(\"砾\")\n打印(长度(b))\n打印(字节十六进制(b))\n打印(字节取(b, 0))\n打印(字节([1, 2]) + 字节([255]))\n打印(十六进制字节(\"00 ff\"))",
        prints: "3\nE7 A0 BE\n231\n字节<01 02 FF>\n字节<00 FF>",
    },
    Example {
        id: "types",
        title: "类型标注（--strict-type）",
        source: "函数 面积(宽: 整数, 高: 整数) -> 整数:\n    返回 宽 * 高\n\n类 计数器:\n    令 值 = 0\n    函数 初始化(起点: 整数):\n        本.值 = 起点\n\n令 c = 新建 计数器(3)\n打印(面积(3, 4), c.值)",
        prints: "12 3",
    },
    Example {
        id: "final",
        title: "最终：禁止继承与覆写",
        source: "类 基础:\n    令 名字 = \"基础\"\n    最终 函数 版本():\n        返回 \"v1\"\n    函数 介绍():\n        返回 本.名字 + \":\" + 本.版本()\n\n类 扩展 继承 基础:\n    令 标签 = \"扩展\"\n    函数 介绍():\n        返回 超.介绍() + \"+\" + 本.标签\n\n最终 类 点:\n    令 x = 0\n\n令 e = 新建 扩展()\n打印(e.介绍())\n令 p = 新建 点()\n打印(p.x)",
        prints: "基础:v1+扩展\n0",
    },
    Example {
        id: "super",
        title: "超：显式调父类实现",
        source: "类 父:\n    令 名字 = \"父\"\n    函数 读():\n        返回 \"父读\"\n\n类 子 继承 父:\n    函数 读():\n        返回 \"子读+\" + 超.读()\n\n令 c = 新建 子()\n打印(c.读())",
        prints: "子读+父读",
    },
    Example {
        id: "inherit",
        title: "单继承",
        source: "类 动物:\n    令 名字 = \"动物\"\n    函数 叫声():\n        返回 \"……\"\n    函数 介绍():\n        返回 本.名字 + \"：\" + 本.叫声()\n\n类 狗 继承 动物:\n    令 品种 = \"中华田园犬\"\n    函数 叫声():\n        返回 \"汪汪\"\n\n令 d = 新建 狗()\n打印(d.介绍())\n打印(d.品种, d.名字)",
        prints: "动物：汪汪\n中华田园犬 动物",
    },
    Example {
        id: "bits",
        title: "位运算",
        source: "打印(5 & 3, 5 | 3, 5 ^ 3)\n打印(~0, ~5)\n打印(1 << 4, -8 >> 1)\n打印(5 & 3 == 1)",
        prints: "1 7 6\n-1 -6\n16 -4\n真",
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
    j.push_str("    \"precedence\": [\"或\", \"与\", \"非\", \"== != < > <= >=\", \"|\", \"^\", \"&\", \"<< >>\", \"+ -\", \"* / // %\", \"一元 - ~\", \"**\", \"调用/下标\"],\n");
    j.push_str("    \"binary\": [");
    let bins: Vec<String> = (0..=opcodes::BIN_SHR)
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
    j.push_str("    \"unary\": [\"-（取负，仅整数）\", \"非\", \"~（按位取反，仅整数）\"]\n");
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
    m.push_str("砾是**中文关键字、缩进式、整数优先**的小语言。它跑在 20 KB RAM 的 MCU 上，整数是 29 位、小数是 Q12 定点；类支持单继承（`继承`）、`超` 显式调父类实现与 `最终` 禁止继承/覆写，`最后`、`用` 和通用模块系统还没有，异常用显式 `抛出` / `尝试` / `捕获`；闭包按值捕获且捕获变量只读；板级硬件能力通过 `导入 模块` 在编译期引入。\n\n");

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
    m.push_str("优先级从低到高：`或` → `与` → `非` → 比较 → `|` → `^` → `&` → `<< >>` → `+ -` → `* / // %` → 一元 `- ~` → `**` → 调用/下标。\n\n");
    m.push_str("- 二元：`+ - * / // % **`、位运算 `& | ^ << >>`\n");
    m.push_str("- 比较：`== != < > <= >=`（可链式，如 `0 <= x < 10`）\n");
    m.push_str("- 一元：`-`（仅整数）、`非`、`~`（按位取反，仅整数）\n\n");

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
            "E132", "E133", "E134", "E135", "E136", "E137", "E138", "E139", "E140", "E141",
            "E142", "E143", "E144",
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

use std::fmt;

/// 编译期错误：错误码 + 行列 + 中文说明。
#[derive(Debug, Clone)]
pub struct CompileError {
    pub code: &'static str,
    pub line: u32,
    pub col: u32,
    pub msg: String,
    pub hint: Option<String>,
}

impl CompileError {
    pub fn new(code: &'static str, line: u32, col: u32, msg: impl Into<String>) -> Self {
        Self {
            code,
            line,
            col,
            msg: msg.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} 错误[{}] {}", self.line, self.col, self.code, self.msg)?;
        if let Some(h) = &self.hint {
            write!(f, "\n  提示：{}", h)?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}

pub type Result<T> = std::result::Result<T, CompileError>;

/// 编译期错误码清单 —— 语言卡从这里生成，`micc --check-langcard` 会与源码里的字面量互查。
pub const COMPILE_CODES: &[(&str, &str)] = &[
    ("E102", "数字字面量格式错误：不支持指数形式，小数点后必须有数字"),
    ("E103", "使用了 v0 不支持的关键字（类/继承/尝试/匹配等）"),
    ("E104", "局部变量超过 255 个"),
    ("E105", "名字/常量/列表元素/字典项超过 255 个"),
    ("E106", "函数超过 255 个"),
    ("E107", "不支持参数默认值"),
    ("E108", "不支持属性访问「.」"),
    ("E109", "语法错误：缺少符号/换行/缩进错乱/字符串未闭合/中断继续位置不对"),
    ("E110", "整数超出 29 位范围（-268435456 .. 268435455），或小数超出 Q12 值域（约 ±524288）"),
    ("E111", "单个函数或 .jmb 超过 64 KB"),
    ("E112", "导入的模块不存在或不可用（v0 有 引脚、ADC、定时器）"),
    ("E113", "调用了原生函数但没有先导入它所属的模块"),
    ("E114", "实参超过 255 个"),
    ("E115", "原生函数用法错误：名字拼错、参数个数不符，或当值/左值使用"),
    ("E116", "闭包捕获的变量超过 255 个"),
    ("E117", "闭包捕获的变量只读：不能赋值，也不能在函数里捕获自己的名字"),
];

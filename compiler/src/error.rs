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

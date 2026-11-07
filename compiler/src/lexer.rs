use crate::error::{CompileError, Result};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kw {
    Ling,
    If,
    Elif,
    Else,
    While,
    Loop,
    Times,
    ForEach,
    In,
    Break,
    Continue,
    Func,
    Return,
    True,
    False,
    Nil,
    And,
    Or,
    Not,
    Import,
    /// JISHI 有、Micro v0 明确不支持的关键字
    Unsupported(&'static str),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Tok {
    Newline,
    Indent,
    Dedent,
    Eof,
    Ident(String),
    Int(i64),
    Str(String),
    Kw(Kw),
    Op(&'static str),
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
}

/// 全角 → 半角（只作用于代码，不进字符串内部）。
fn norm(c: char) -> char {
    match c {
        '：' => ':',
        '，' => ',',
        '（' => '(',
        '）' => ')',
        '［' => '[',
        '］' => ']',
        '【' => '(',
        '】' => ')',
        '｛' => '{',
        '｝' => '}',
        '＝' => '=',
        '＋' => '+',
        '－' => '-',
        '＊' => '*',
        '／' => '/',
        '％' => '%',
        '＜' => '<',
        '＞' => '>',
        '！' => '!',
        '＿' => '_',
        '．' => '.',
        '＃' => '#',
        '　' => ' ',
        _ => c,
    }
}

/// 关键字表 —— 词法器与语言卡生成共用的唯一清单。
pub const KEYWORDS: &[(&str, Kw)] = &[
    ("令", Kw::Ling),
    ("如果", Kw::If),
    ("否则如果", Kw::Elif),
    ("否则", Kw::Else),
    ("当", Kw::While),
    ("循环", Kw::Loop),
    ("次", Kw::Times),
    ("遍历", Kw::ForEach),
    ("在", Kw::In),
    ("中断", Kw::Break),
    ("继续", Kw::Continue),
    ("函数", Kw::Func),
    ("返回", Kw::Return),
    ("真", Kw::True),
    ("假", Kw::False),
    ("空", Kw::Nil),
    ("与", Kw::And),
    ("或", Kw::Or),
    ("非", Kw::Not),
    ("导入", Kw::Import),
    ("类", Kw::Unsupported("类")),
    ("继承", Kw::Unsupported("继承")),
    ("新建", Kw::Unsupported("新建")),
    ("尝试", Kw::Unsupported("尝试")),
    ("捕获", Kw::Unsupported("捕获")),
    ("最终", Kw::Unsupported("最终")),
    ("抛出", Kw::Unsupported("抛出")),
    ("匹配", Kw::Unsupported("匹配")),
    ("情形", Kw::Unsupported("情形")),
    ("枚举", Kw::Unsupported("枚举")),
    ("用", Kw::Unsupported("用")),
    ("断言", Kw::Unsupported("断言")),
    ("超", Kw::Unsupported("超")),
];

pub fn keyword_table() -> &'static [(&'static str, Kw)] {
    KEYWORDS
}

fn keyword(s: &str) -> Option<Kw> {
    KEYWORDS
        .iter()
        .find(|(w, _)| *w == s)
        .map(|(_, k)| *k)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

struct Lexer {
    tokens: Vec<Token>,
    indents: Vec<usize>,
    depth: usize,
    line: u32,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>> {
    let mut lx = Lexer {
        tokens: Vec::new(),
        indents: vec![0],
        depth: 0,
        line: 1,
    };
    lx.run(src)?;
    Ok(lx.tokens)
}

impl Lexer {
    fn run(&mut self, src: &str) -> Result<()> {
        for (idx, raw) in src.split('\n').enumerate() {
            let line_no = idx as u32 + 1;
            self.line = line_no;
            let line = raw.trim_end_matches('\r');
            let chars: Vec<char> = line.chars().collect();

            let mut start = 0usize;
            if self.depth == 0 {
                // 量缩进（只认半角空格）
                while start < chars.len() {
                    match chars[start] {
                        ' ' => start += 1,
                        '\t' => {
                            return Err(CompileError::new(
                                "E109",
                                line_no,
                                start as u32 + 1,
                                "缩进里不能用 Tab",
                            )
                            .with_hint("用 4 个半角空格"))
                        }
                        '　' => {
                            return Err(CompileError::new(
                                "E109",
                                line_no,
                                start as u32 + 1,
                                "缩进里不能用全角空格",
                            )
                            .with_hint("用 4 个半角空格"))
                        }
                        _ => break,
                    }
                }
                let rest_is_blank = start >= chars.len() || chars[start] == '#';
                if rest_is_blank {
                    continue;
                }
                self.handle_indent(start, line_no)?;
            }

            self.tokenize_line(&chars, start, line_no)?;

            if self.depth == 0 {
                self.tokens.push(Token {
                    tok: Tok::Newline,
                    line: line_no,
                    col: chars.len() as u32 + 1,
                });
            }
        }

        while self.indents.len() > 1 {
            self.indents.pop();
            self.tokens.push(Token {
                tok: Tok::Dedent,
                line: self.line,
                col: 1,
            });
        }
        self.tokens.push(Token {
            tok: Tok::Eof,
            line: self.line,
            col: 1,
        });
        Ok(())
    }

    fn handle_indent(&mut self, width: usize, line: u32) -> Result<()> {
        let cur = *self.indents.last().unwrap();
        if width > cur {
            self.indents.push(width);
            self.tokens.push(Token {
                tok: Tok::Indent,
                line,
                col: 1,
            });
        } else if width < cur {
            while self.indents.len() > 1 && *self.indents.last().unwrap() > width {
                self.indents.pop();
                self.tokens.push(Token {
                    tok: Tok::Dedent,
                    line,
                    col: 1,
                });
            }
            if *self.indents.last().unwrap() != width {
                return Err(CompileError::new(
                    "E109",
                    line,
                    1,
                    "缩进对不上：这一行没有对应的外层缩进",
                ));
            }
        }
        Ok(())
    }

    fn tokenize_line(&mut self, chars: &[char], start: usize, line: u32) -> Result<()> {
        let mut i = start;
        while i < chars.len() {
            let c = norm(chars[i]);
            if c == ' ' {
                i += 1;
                continue;
            }
            if c == '#' {
                break;
            }
            let col = i as u32 + 1;
            if c.is_ascii_digit() {
                let (val, next) = self.read_number(chars, i, line, col)?;
                self.tokens.push(Token {
                    tok: Tok::Int(val),
                    line,
                    col,
                });
                i = next;
                continue;
            }
            if is_ident_start(c) {
                let (name, next) = self.read_ident(chars, i);
                let tok = match keyword(&name) {
                    Some(k) => Tok::Kw(k),
                    None => Tok::Ident(name),
                };
                self.tokens.push(Token { tok, line, col });
                i = next;
                continue;
            }
            if matches!(c, '"' | '\'' | '“' | '”') {
                let (s, next) = self.read_string(chars, i, line, col)?;
                self.tokens.push(Token {
                    tok: Tok::Str(s),
                    line,
                    col,
                });
                i = next;
                continue;
            }

            // 运算符：先长后短
            let rest: String = chars[i..].iter().map(|x| norm(*x)).collect();
            let mut matched: Option<(&'static str, usize)> = None;
            for op in [
                "//=", "**=", "**", "//", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "->",
            ] {
                if rest.starts_with(op) {
                    matched = Some((op, op.chars().count()));
                    break;
                }
            }
            if let Some((op, n)) = matched {
                self.tokens.push(Token {
                    tok: Tok::Op(op),
                    line,
                    col,
                });
                i += n;
                continue;
            }
            let single: Option<&'static str> = match c {
                '+' => Some("+"),
                '-' => Some("-"),
                '*' => Some("*"),
                '/' => Some("/"),
                '%' => Some("%"),
                '=' => Some("="),
                '<' => Some("<"),
                '>' => Some(">"),
                '(' => Some("("),
                ')' => Some(")"),
                '[' => Some("["),
                ']' => Some("]"),
                '{' => Some("{"),
                '}' => Some("}"),
                ',' => Some(","),
                ':' => Some(":"),
                '.' => Some("."),
                '!' => Some("!"),
                _ => None,
            };
            match single {
                Some(op) => {
                    if op == "(" || op == "[" || op == "{" {
                        self.depth += 1;
                    } else if op == ")" || op == "]" || op == "}" {
                        self.depth = self.depth.saturating_sub(1);
                    }
                    self.tokens.push(Token {
                        tok: Tok::Op(op),
                        line,
                        col,
                    });
                    i += 1;
                }
                None => {
                    return Err(CompileError::new(
                        "E109",
                        line,
                        col,
                        format!("不认识的字符「{}」", chars[i]),
                    ))
                }
            }
        }
        Ok(())
    }

    fn read_number(
        &self,
        chars: &[char],
        start: usize,
        line: u32,
        col: u32,
    ) -> Result<(i64, usize)> {
        let mut i = start;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i < chars.len() {
            let n = norm(chars[i]);
            if (n == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
                || n == '.'
                || n == 'e'
                || n == 'E'
            {
                return Err(CompileError::new(
                    "E102",
                    line,
                    col,
                    "Micro Core v0 不支持小数",
                )
                .with_hint("先用整数；小数在 JM5 加入"));
            }
        }
        let text: String = chars[start..i].iter().collect();
        match text.parse::<i64>() {
            Ok(v) => Ok((v, i)),
            Err(_) => Err(CompileError::new("E110", line, col, "整数太大")),
        }
    }

    fn read_ident(&self, chars: &[char], start: usize) -> (String, usize) {
        let mut i = start;
        while i < chars.len() && is_ident_continue(norm(chars[i])) {
            i += 1;
        }
        (chars[start..i].iter().map(|c| norm(*c)).collect(), i)
    }

    fn read_string(
        &self,
        chars: &[char],
        start: usize,
        line: u32,
        col: u32,
    ) -> Result<(String, usize)> {
        let open = norm(chars[start]);
        let mut i = start + 1;
        let mut out = String::new();
        loop {
            if i >= chars.len() {
                return Err(CompileError::new("E109", line, col, "字符串没有闭合"));
            }
            let c = norm(chars[i]);
            if (open == '"' && c == '"')
                || (open == '\'' && c == '\'')
                || (open == '“' && (c == '”' || c == '“'))
                || (open == '”' && (c == '”' || c == '“'))
            {
                return Ok((out, i + 1));
            }
            if c == '\\' {
                i += 1;
                if i >= chars.len() {
                    return Err(CompileError::new("E109", line, col, "字符串没有闭合"));
                }
                let e = chars[i];
                out.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    '"' => '"',
                    '\'' => '\'',
                    other => other,
                });
                i += 1;
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
    }
}

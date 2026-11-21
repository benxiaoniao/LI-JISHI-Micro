//! 砾源码的最小格式化：缩进归一为 4 空格 + 运算符两侧留空格。
//!
//! 与 `editors/vscode/src/core.ts` 的 `formatSource` 同一套规则（LI037 起由 LSP 统一提供，
//! 编辑器不再自己实现）。纯函数：`format_source(format_source(x)) == format_source(x)`。

#[derive(Clone, PartialEq)]
enum Kind {
    Word,
    Num,
    Str,
    Op,
    Punct,
    Com,
}

#[derive(Clone)]
struct Tok {
    kind: Kind,
    text: String,
    /// 注释/左括号前原有的空格数，格式化时原样保留。
    gap: Option<usize>,
}

const MULTI_OPS: &[&str] = &[
    "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "**", "//", "<<", ">>", "->", "??",
    "?.", "?[",
];
const PUNCT_CHARS: &str = "()[]{},.:：，、；";
const NO_SPACE_BEFORE: &str = ")]}.,:：，、；";

const LIST_KEYWORDS: &[&str] = &[
    "返回", "令", "在", "导入", "创建", "新建", "抛出", "捕获", "如果", "当", "循环", "遍历",
    "否则", "类", "函数", "匹配", "情形",
];
const EXPR_KEYWORDS: &[&str] = &[
    "返回", "令", "在", "如果", "当", "循环", "遍历", "否则", "抛出", "捕获", "创建", "新建",
    "打印", "类", "函数", "导入",
];

fn is_space(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\u{3000}'
}

fn tokenize_line(line: &str) -> Vec<Tok> {
    let chars: Vec<char> = line.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if is_space(ch) {
            i += 1;
            continue;
        }
        if ch == '#' {
            let mut gap = 0usize;
            let mut k = i;
            while k > 0 && is_space(chars[k - 1]) {
                gap += 1;
                k -= 1;
            }
            let text: String = chars[i..].iter().collect();
            toks.push(Tok {
                kind: Kind::Com,
                text,
                gap: Some(gap.max(1)),
            });
            break;
        }
        if ch == '"' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '"' {
                j += if chars[j] == '\\' { 2 } else { 1 };
            }
            j = (j + 1).min(chars.len());
            let text: String = chars[i..j].iter().collect();
            toks.push(Tok {
                kind: Kind::Str,
                text,
                gap: None,
            });
            i = j;
            continue;
        }
        if ch == '“' || ch == '‘' {
            let close = if ch == '“' { '”' } else { '’' };
            let mut j = i + 1;
            while j < chars.len() && chars[j] != close {
                j += 1;
            }
            j = (j + 1).min(chars.len());
            let text: String = chars[i..j].iter().collect();
            toks.push(Tok {
                kind: Kind::Str,
                text,
                gap: None,
            });
            i = j;
            continue;
        }
        if ch.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '.' {
                if j + 1 < chars.len() && chars[j + 1].is_ascii_digit() {
                    j += 2;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                }
            }
            let text: String = chars[i..j].iter().collect();
            toks.push(Tok {
                kind: Kind::Num,
                text,
                gap: None,
            });
            i = j;
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let text: String = chars[i..j].iter().collect();
            toks.push(Tok {
                kind: Kind::Word,
                text,
                gap: None,
            });
            i = j;
            continue;
        }
        if ch == '(' {
            let mut gap = 0usize;
            let mut k = i;
            while k > 0 && is_space(chars[k - 1]) {
                gap += 1;
                k -= 1;
            }
            toks.push(Tok {
                kind: Kind::Punct,
                text: "(".to_string(),
                gap: Some(gap),
            });
            i += 1;
            continue;
        }
        let rest: String = chars[i..].iter().collect();
        if let Some(op) = MULTI_OPS.iter().find(|op| rest.starts_with(*op)) {
            toks.push(Tok {
                kind: Kind::Op,
                text: (*op).to_string(),
                gap: None,
            });
            i += op.chars().count();
            continue;
        }
        if "+-*/%<>=~".contains(ch) {
            toks.push(Tok {
                kind: Kind::Op,
                text: ch.to_string(),
                gap: None,
            });
            i += 1;
            continue;
        }
        if PUNCT_CHARS.contains(ch) {
            toks.push(Tok {
                kind: Kind::Punct,
                text: ch.to_string(),
                gap: None,
            });
            i += 1;
            continue;
        }
        // 认不出的字符原样保留（宁可不动，也不要吞掉用户代码）
        toks.push(Tok {
            kind: Kind::Punct,
            text: ch.to_string(),
            gap: None,
        });
        i += 1;
    }
    toks
}

fn is_value_end(tok: &Tok) -> bool {
    if tok.kind == Kind::Word && EXPR_KEYWORDS.contains(&tok.text.as_str()) {
        return false;
    }
    tok.kind == Kind::Word
        || tok.kind == Kind::Num
        || tok.kind == Kind::Str
        || (tok.kind == Kind::Punct && (tok.text == ")" || tok.text == "]" || tok.text == "}"))
}

fn need_space(prev: &Tok, prev_unary: bool, tok: &Tok) -> bool {
    if tok.kind == Kind::Com {
        return false; // 注释前的空格按原样保留
    }
    if tok.kind == Kind::Op && (tok.text == "?." || tok.text == "?[") {
        return false;
    }
    if prev.kind == Kind::Op && (prev.text == "?." || prev.text == "?[") {
        return false;
    }
    if tok.kind == Kind::Punct && tok.text.chars().all(|c| NO_SPACE_BEFORE.contains(c)) {
        return false;
    }
    if tok.kind == Kind::Punct && tok.text == "(" {
        if prev.kind == Kind::Op || prev.text == "," || prev.text == ":" || prev.text == "：" {
            return true;
        }
        // 名字与 `(` 之间：按原样保留
        return prev.kind == Kind::Word && tok.gap.unwrap_or(0) > 0;
    }
    if tok.kind == Kind::Punct && tok.text == "[" {
        if prev.kind == Kind::Word {
            return LIST_KEYWORDS.contains(&prev.text.as_str());
        }
        if prev.kind == Kind::Op || prev.text == "," || prev.text == ":" || prev.text == "：" {
            return true;
        }
        return false;
    }
    if prev.kind == Kind::Op {
        return !prev_unary;
    }
    if prev.kind == Kind::Punct {
        if prev.text == "(" || prev.text == "[" || prev.text == "{" || prev.text == "." {
            return false;
        }
        return true;
    }
    if tok.kind == Kind::Op {
        if tok.text != "-" && tok.text != "+" {
            return true;
        }
        if is_value_end(prev) {
            return true;
        }
        return prev.kind == Kind::Word;
    }
    true
}

fn format_tokens(toks: &[Tok]) -> String {
    let mut out = String::new();
    let mut prev: Option<&Tok> = None;
    let mut prev_unary = false;
    for tok in toks {
        if let Some(p) = prev {
            if tok.kind == Kind::Com {
                out.push_str(&" ".repeat(tok.gap.unwrap_or(1)));
            } else if need_space(p, prev_unary, tok) {
                out.push(' ');
            }
        }
        out.push_str(&tok.text);
        prev_unary = tok.kind == Kind::Op
            && (tok.text == "-" || tok.text == "+" || tok.text == "~")
            && !(prev.map(is_value_end).unwrap_or(false));
        prev = Some(tok);
    }
    out
}

fn count_indent(lead: &str) -> usize {
    lead.chars()
        .map(|c| if c == '\t' { 4 } else if c == '\u{3000}' { 2 } else { 1 })
        .sum()
}

/// 最小格式化（与编辑器旧实现逐字符一致）。
pub fn format_source(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<usize> = vec![0];
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let stripped = raw.trim_end_matches(|c| is_space(c));
        if stripped.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let lead_len = raw.chars().take_while(|&c| is_space(c)).count();
        let lead: String = raw.chars().take(lead_len).collect();
        let orig = count_indent(&lead);
        while stack.len() > 1 && orig < *stack.last().unwrap() {
            stack.pop();
        }
        if orig > *stack.last().unwrap() {
            stack.push(orig);
        }
        let level = stack.len() - 1;
        let mut line = "    ".repeat(level);
        line.push_str(&format_tokens(&tokenize_line(stripped.trim())));
        out.push(line);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::format_source;

    #[test]
    fn basic() {
        let src = "函数 甲():\n  令 x=1+2\n";
        assert_eq!(format_source(src), "函数 甲():\n    令 x = 1 + 2\n");
    }

    #[test]
    fn idempotent() {
        let src = "匹配 [1, 2]:\n    情形 [甲, 乙]:\n        打印(甲, 乙)\n    情形 其他:\n        打印(\"其他\")\n";
        assert_eq!(format_source(src), src);
        assert_eq!(format_source(&format_source(src)), format_source(src));
    }
}

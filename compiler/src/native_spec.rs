//! 板级原生函数表的编译期规格。
//!
//! 这是「导入 模块」与 C 侧 `JmNativeEntry` 表的**唯一事实来源**：
//! - `micc` 编译 `.jsh` 时据此做名字存在性、导入状态与参数个数的静态检查；
//! - `micc --emit-native-header` 据此生成 `firmware/stm32f103/f103_native_table.h`，
//!   C 侧不再手写名字表（名字、参数个数、C 函数名都从同一份规格来）；
//! - `micc --check-native` 校验入库的生成头与规格一致，防止有人只改一侧。
//!
//! 运行期仍然只按名字绑定：`.jmb` 里只有名字，`jm_native_bind_name` 在 `jm_run`
//! 时把它接到登记过的 native 表项。编译期检查与运行期分派因此不会各有一份名字表。

use std::collections::HashSet;
use std::path::PathBuf;

pub struct NativeEntrySpec {
    pub name: &'static str,
    pub min_args: u8,
    pub max_args: u8,
    pub c_fn: &'static str,
    pub doc: &'static str,
}

pub struct NativeModuleSpec {
    /// `导入` 后面写的模块名。
    pub name: &'static str,
    /// 运行期登记的表名（`jm_native_register` 的那张表）。
    pub table: &'static str,
    /// 生成头文件里宏的前缀，例如 `F103_NATIVE`。
    pub c_prefix: &'static str,
    pub entries: &'static [NativeEntrySpec],
}

pub const MODULES: &[NativeModuleSpec] = &[NativeModuleSpec {
    name: "引脚",
    table: "f103_board",
    c_prefix: "F103_NATIVE",
    entries: &[
        NativeEntrySpec {
            name: "引脚写",
            min_args: 2,
            max_args: 2,
            c_fn: "n_pin_write",
            doc: "向指定引脚写电平；v0 只支持引脚 13（PC13），电平 0/1。PC13 低电平点亮。",
        },
        NativeEntrySpec {
            name: "引脚读",
            min_args: 1,
            max_args: 1,
            c_fn: "n_pin_read",
            doc: "读指定引脚的输入寄存器值（0/1）；v0 只支持引脚 13（PC13）。",
        },
        NativeEntrySpec {
            name: "引脚翻转",
            min_args: 1,
            max_args: 1,
            c_fn: "n_pin_toggle",
            doc: "读当前电平再取反；v0 只支持引脚 13（PC13）。",
        },
        NativeEntrySpec {
            name: "串口写",
            min_args: 1,
            max_args: 1,
            c_fn: "n_uart_write",
            doc: "直接写 USART1（115200 8N1），不换行、不加前缀。",
        },
        NativeEntrySpec {
            name: "等待毫秒",
            min_args: 1,
            max_args: 1,
            c_fn: "n_delay",
            doc: "忙等 0–60000 毫秒；SysTick 中断仍在跑，调度器不参与。",
        },
    ],
}];

pub const BOARD_HEADER_PATH: &str = "firmware/stm32f103/f103_native_table.h";
const BOARD_TABLE: &str = "f103_board";

pub fn module(name: &str) -> Option<&'static NativeModuleSpec> {
    MODULES.iter().find(|m| m.name == name)
}

pub fn entry(name: &str) -> Option<(&'static NativeModuleSpec, &'static NativeEntrySpec)> {
    for m in MODULES {
        for e in m.entries {
            if e.name == name {
                return Some((m, e));
            }
        }
    }
    None
}

pub fn module_names() -> String {
    MODULES
        .iter()
        .map(|m| m.name)
        .collect::<Vec<_>>()
        .join("、")
}

/// 在已导入的模块里找与 `name` 只差一个字的函数，用于拼错提示。
pub fn suggest(
    name: &str,
    imported: &HashSet<String>,
) -> Option<(&'static NativeModuleSpec, &'static NativeEntrySpec)> {
    if name.chars().count() < 3 {
        return None;
    }
    let mut best: Option<(usize, &'static NativeModuleSpec, &'static NativeEntrySpec)> = None;
    for m in MODULES {
        if !m.entries.iter().any(|e| imported.contains(e.name)) {
            continue;
        }
        for e in m.entries {
            let d = edit_distance(name, e.name);
            if d <= 1 && best.as_ref().map(|(bd, _, _)| d < *bd).unwrap_or(true) {
                best = Some((d, m, e));
            }
        }
    }
    best.map(|(_, m, e)| (m, e))
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn validate() -> Result<(), String> {
    let mut module_names_seen = HashSet::new();
    let mut entry_names_seen = HashSet::new();
    for m in MODULES {
        if !module_names_seen.insert(m.name) {
            return Err(format!("模块名「{}」重复", m.name));
        }
        if m.entries.is_empty() {
            return Err(format!("模块「{}」没有任何原生函数", m.name));
        }
        for e in m.entries {
            if !entry_names_seen.insert(e.name) {
                return Err(format!("原生函数名「{}」在多个模块里重复", e.name));
            }
            if e.c_fn.is_empty() {
                return Err(format!("原生函数「{}」缺少 C 函数名", e.name));
            }
            if e.min_args > e.max_args || e.max_args > 8 {
                return Err(format!(
                    "原生函数「{}」的参数个数范围不合法：{}..{}",
                    e.name, e.min_args, e.max_args
                ));
            }
        }
    }
    Ok(())
}

/// 生成板级原生表头（`firmware/stm32f103/f103_native_table.h`）。
pub fn render_c_header() -> Result<String, String> {
    validate()?;
    let m = MODULES
        .iter()
        .find(|m| m.table == BOARD_TABLE)
        .ok_or_else(|| format!("规格里没有表 {}", BOARD_TABLE))?;

    let guard = format!("{}_TABLE_H", m.c_prefix);
    let mut s = String::new();
    s.push_str("/* 本文件由 `micc --emit-native-header` 从 compiler/src/native_spec.rs 生成，不要手改。 */\n");
    s.push_str(&format!(
        "/* 原生表 {}：{} 项 */\n",
        m.table,
        m.entries.len()
    ));
    s.push_str(&format!("#ifndef {}\n", guard));
    s.push_str(&format!("#define {}\n\n", guard));
    s.push_str(&format!(
        "#define {}_TABLE_NAME \"{}\"\n",
        m.c_prefix, m.table
    ));
    s.push_str(&format!(
        "#define {}_ENTRY_COUNT {}u\n\n",
        m.c_prefix,
        m.entries.len()
    ));
    s.push_str("/* 在 JmNativeEntry 数组初始化处展开；名字、参数个数、C 函数名都来自同一份规格。 */\n");
    s.push_str(&format!("#define {}_TABLE_INIT \\\n", m.c_prefix));
    let n = m.entries.len();
    for (i, e) in m.entries.iter().enumerate() {
        s.push_str(&format!(
            "    {{\"{}\", {}, {}u, {}u, 0u, 0u}}{}\n",
            e.name,
            e.c_fn,
            e.min_args,
            e.max_args,
            if i + 1 < n { ", \\" } else { "" }
        ));
    }
    s.push_str(&format!("\n#endif /* {} */\n", guard));
    Ok(s)
}

fn header_path() -> PathBuf {
    crate::langcard::repo_root().join(BOARD_HEADER_PATH)
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

/// `--check-native`：CI 防漂移。
pub fn check() -> Result<(), String> {
    let path = header_path();
    let want = render_c_header()?;
    let got = std::fs::read_to_string(&path)
        .map_err(|e| format!("读不了原生表头 {}：{}", path.display(), e))?
        .replace("\r\n", "\n");
    if got != want {
        return Err(format!(
            "{} 与 compiler/src/native_spec.rs 不一致：{}\n请运行 `micc --emit-native-header > {}` 重新生成",
            path.display(),
            first_diff(&want, &got),
            BOARD_HEADER_PATH
        ));
    }
    println!("原生表头通过校验：{} 与规格一致", path.display());
    Ok(())
}

//! 语言层任务/等待函数的编译期规格。
//!
//! 这些名字不走 Native ABI：`创建/让出/延时` 在 VM 绿色调度器里完成，
//! `队列接收/队列发送/信号量等待` 的**带超时形式**降到 `OP_TASK_WAIT`（探测一次、
//! 挂起、到期重试），不带超时的形式仍是普通原生调用。见 docs/设计-语言层任务.md §3.4。

use crate::opcodes::{OP_TASK_SLEEP, OP_TASK_SPAWN, OP_TASK_YIELD};

/// `导入 任务` 的模块名。
pub const TASK_MODULE: &str = "任务";

pub struct TaskFnSpec {
    pub name: &'static str,
    pub nargs: u8,
    pub opcode: u8,
}

/// 任务模块里的三个名字：只能直接调用，不能当值。
pub const TASK_FNS: &[TaskFnSpec] = &[
    TaskFnSpec {
        name: "创建",
        nargs: 1,
        opcode: OP_TASK_SPAWN,
    },
    TaskFnSpec {
        name: "让出",
        nargs: 0,
        opcode: OP_TASK_YIELD,
    },
    TaskFnSpec {
        name: "延时",
        nargs: 1,
        opcode: OP_TASK_SLEEP,
    },
];

pub struct WaitFnSpec {
    pub name: &'static str,
    /// `OP_TASK_WAIT` 的立即数：0 队列接收 / 1 队列发送 / 2 信号量等待。
    pub kind: u8,
    /// 带超时形式的总实参个数。
    pub nargs: u8,
}

/// 带超时的等待调用：编译器降到 `OP_TASK_WAIT`。
/// 不带超时的形式（`队列接收(q)` / `队列发送(q, v)` / `信号量等待(s)`）仍是普通原生调用。
pub const WAIT_FNS: &[WaitFnSpec] = &[
    WaitFnSpec {
        name: "队列接收",
        kind: 0,
        nargs: 2,
    },
    WaitFnSpec {
        name: "队列发送",
        kind: 1,
        nargs: 3,
    },
    WaitFnSpec {
        name: "信号量等待",
        kind: 2,
        nargs: 2,
    },
];

pub fn task_entry(name: &str) -> Option<&'static TaskFnSpec> {
    TASK_FNS.iter().find(|f| f.name == name)
}

pub fn wait_entry(name: &str) -> Option<&'static WaitFnSpec> {
    WAIT_FNS.iter().find(|f| f.name == name)
}

/// 任务模块的函数名不能当变量/参数/被赋值。
pub fn reserved(name: &str) -> bool {
    task_entry(name).is_some()
}

pub fn module_names() -> String {
    TASK_MODULE.to_string()
}

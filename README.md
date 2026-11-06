# 砾（LI）

> **基石语言（JISHI）面向单片机的分支。**
> 仓库名：`LI-JISHI-Micro` · 语言名：**砾** · 许可证：[MIT-0](LICENSE)

砾保留基石的中文关键字与缩进写法，但把「值、指令、内存、错误」全部重新设计成能在
**STM32F103C8T6（Cortex-M3，64KB Flash / 20KB SRAM）** 上活下来的尺寸。

```
PC 上：  .jsh 源码 ──micc──▶  .jmb 微字节码
芯片上：.jmb ──libjishim（C99）──▶ 执行
之后：  在同一台 VM 上长出任务与调度 —— 一个「用砾写」的 FreeRTOS 形态内核
```

完整规划见 [docs/路径图.md](docs/路径图.md)，字节码契约见 [docs/微字节码规格.md](docs/微字节码规格.md)。

---

## 现在能跑什么

已经跑通 **JM0–JM5**（立项 → 字节码契约 → 编译器 → 虚拟机 → 端到端 → 容器/文本）：

| 能力 | 状态 |
|------|------|
| `令` / 赋值 / 复合赋值（`+=` 等） | ✅ |
| 整数算术 `+ - * / // % **`、比较（含链式）、`与/或/非` | ✅ |
| `如果 / 否则如果 / 否则`、`当`、`循环 N 次`、`遍历`、`中断/继续` | ✅ |
| `函数` / `返回` / 递归 | ✅ |
| 列表、字典、文本（UTF-8 码点计数）、下标读写 | ✅ |
| 内建：`打印 文本 整数 长度 范围 类型 总和 最大 最小 绝对值` | ✅ |
| 引用计数堆 + 静态块分配器 + 压力用例（1500 次分配） | ✅ |
| 小数 / 闭包 / 类 / 异常 / 属性访问 | ❌ v0 明确报错 |

示例见 [examples/](examples/)；回归基准见 [tests/expected/](tests/expected/)。
`tools\build.ps1` 会跑 **6 个输出用例 + 6 个错误用例**（错误用例专门验证「差异显式报错」）。

---

## 快速开始

需要：Rust（构建编译器）、一个主机 C 编译器（构建 `jmrun`）。
ARM 工具链只在交叉编译时用。

```powershell
# 1) 构建 + 跑回归（一条命令）
powershell -ExecutionPolicy Bypass -File tools\build.ps1

# 2) 单独编译/运行
compiler\target\release\micc.exe examples\你好.jsh -o build\你好.jmb
build\jmrun.exe build\你好.jmb

# 3) 看反汇编
compiler\target\release\micc.exe examples\斐波那契.jsh --dump
```

工具链路径通过环境变量覆盖（见 [tools/toolchains.ps1](tools/toolchains.ps1)）：

| 变量 | 用途 |
|------|------|
| `JM_HOST_GCC` | 主机 C 编译器（默认自动在 PATH 里找 `gcc`） |
| `JM_ARM_GCC` | `arm-none-eabi-gcc` |
| `JM_ARM_SIZE` | `arm-none-eabi-size` |

---

## 目录

```
compiler/          Rust 编译器 micc
  src/lexer.rs       词法（全角归一化 / 缩进 / 中文标识符）
  src/parser.rs      语法（砾 Core 子集）
  src/codegen.rs     作用域分析与代码生成
  src/opcodes.rs     操作码唯一事实来源（导出 C 头文件）
  src/writer.rs      .jmb 写出与反汇编
runtime/           C99 运行时
  include/           jishim.h（公共 ABI）、jm_opcodes.h（生成）
  src/               vm / heap / builtin / loader / error
  port/host/         PC 端口（stdout）
examples/          砾 源码示例
tests/expected/    回归基准输出
tools/             构建、测试、工具链定位
docs/              路径图、字节码规格
```

---

## 实测资源（Cortex-M3，`-Os`）

用 `arm-none-eabi-gcc 13.3.1 -Os -mcpu=cortex-m3 -mthumb` 编译运行时核心：

| 项目 | 字节 |
|------|------|
| `jm_vm.o` | 6 604 |
| `jm_builtin.o` | 2 999 |
| `jm_heap.o` | 1 020 |
| `jm_error.o` | 1 056 |
| `jm_loader.o` | 732 |
| **Flash 合计** | **12 411（约 12.1 KB）** |
| 静态堆（bss） | 12 288 |
| VM 状态（栈/帧/全局/常量缓存，bss） | 4 344 |
| 错误文案缓冲区（bss） | 128 |
| **RAM(bss) 合计** | **16 760（约 16.4 KB）** |

F103C8T6 的 64KB Flash / 20KB SRAM 能装下；剩余约 **3.6KB** 留给 C 栈与中断。
装不下的不是「语言太大」，而是「任务内核还没写」——这正是 JM6 之后的事。

---

## 与基石的关系

- **语法同源**：中文关键字、缩进、全角标点归一化、显示规则都对齐基石。
- **语义是子集**：基石有、砾 v0 没有的写法（小数、闭包、类、异常、属性访问）**编译期报专门错误码**，不静默降级。
- **实现不共享**：砾有自己的 `.jmb` 与 C 虚拟机，不复用基石那套 24 字节指令 / 宿主回调模型。

---

## 许可证

**MIT-0（MIT No Attribution）**：可商用、可修改、可再分发，**不要求保留版权声明**。
完整原文见 [LICENSE](LICENSE)。

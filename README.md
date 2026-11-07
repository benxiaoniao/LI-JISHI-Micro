# 砾（LI）

> **基石语言（JISHI）面向单片机的分支。**
> 仓库名：`LI-JISHI-Micro` · 语言名：**砾** · 许可证：[MIT-0](LICENSE)

砾保留基石的中文关键字与缩进写法，但把「值、指令、内存、错误」全部重新设计成能在
**STM32F103C8T6（Cortex-M3，64 KB Flash / 20 KB SRAM）** 上活下来的尺寸：

```
PC 上：  .jsh 源码 ──micc──▶  .jmb 微字节码
芯片上：.jmb ──libjishim（C99）──▶ 虚拟机 ──▶ JMK 任务内核
```

本文面向第一次拿到仓库的人：先说什么能用，再说怎么构建、怎么烧、怎么看串口，最后列目录与限制。
设计取舍见 [docs/路径图.md](docs/路径图.md)，字节码契约见 [docs/微字节码规格.md](docs/微字节码规格.md)。

---

## 1. 现在有什么

| 组成 | 内容 |
|------|------|
| 编译器 `micc` | Rust 实现，`.jsh → .jmb`；词法（中文标识符 / 全角归一化 / 4 空格缩进）、语法、作用域、代码生成、反汇编（`--dump`） |
| 字节码 | `.jmb`（JMB v2），变长指令 1/2/3 字节，4 字节值 + 29 位整数；小数是 Q12 定点堆对象；闭包是带捕获表的堆对象 |
| 运行时 `libjishim` | C99、零依赖：`jm_loader`（读包）/ `jm_heap`（静态堆 + 引用计数）/ `jm_vm`（栈机）/ `jm_builtin`（内建）/ `jm_native`（原生绑定）/ `jm_error`（错误码） |
| 语言能力 | `令`/赋值/复合赋值、`如果/否则如果/否则`、`当`、`循环 N 次`、`遍历`、`中断/继续`、`函数/返回/递归`、闭包（值捕获、只读）；列表、字典、文本（UTF-8 码点）、小数（Q12 定点） |
| 内建 | `打印 文本 整数 小数 长度 范围 类型 总和 最大 最小 绝对值` |
| JMK 内核 | 抢占式固定优先级调度（PendSV 切换）、SysTick 1 kHz、6 个用户任务 + 空闲任务、4 档优先级、动态栈池（默认 3200 B）或静态 6×512 B、信号量、消息队列、毫秒延时/让出 |
| 板级原生表 | `引脚`（5 项）、`ADC`（2 项）、`定时器`（3 项）；由 `compiler/src/native_spec.rs` 单一事实来源生成 `f103_native_table.h` |
| 语言卡 | [docs/语言卡.md](docs/语言卡.md) + [spec/langcard.json](spec/langcard.json)，由操作码/原生表/内建/错误码渲染；`--check-langcard` 防漂移 |
| 规范校验 | `--check-native`、`--check-langcard` 进构建链路，头文件或语言卡落后于规范即失败 |
| 可复现工具链 | `tools/toolchains/toolchain.lock.json` + `tools/fetch-toolchain.ps1`，按锁文件哈希安装 |

上板验证记录在 [docs/verification/](docs/verification/)：ADC 零点（PA0 接 GND）、
ADC 满量程（PA0 接 3V3）、以及第四轮读到 4094 触发 E023 的故障与修复对照。

---

## 2. 构建

需要：Rust（构建 `micc`）、主机 C 编译器（构建 `jmrun`）、`arm-none-eabi-gcc`（构建固件）。

```powershell
# 一次装好 ARM 工具链（按锁文件哈希校验；默认装到 ASCII 缓存目录）
powershell -ExecutionPolicy Bypass -File tools\fetch-toolchain.ps1
```

工具链解析顺序与路径覆盖见 [tools/toolchains.ps1](tools/toolchains.ps1) 与 [docs/工具链.md](docs/工具链.md)：
包内 `tools/toolchains/<名字>/bin` → 外部缓存（`%LOCALAPPDATA%\jishi\toolchains`）→
环境变量 `JM_HOST_GCC` / `JM_ARM_GCC` / `JM_ARM_SIZE` → `PATH`。
仓库路径含中文时，工具链必须放外部缓存（MinGW 版 gcc/ld 用 ANSI 文件 API，中文目录下会找不到 crt/lib）。

### 2.1 主机构建 + 回归

```powershell
powershell -ExecutionPolicy Bypass -File tools\build.ps1
```

脚本依次做：构建 `micc` → 校验原生表头 → 生成 `runtime/include/jm_opcodes.h` →
构建 `build\jmrun.exe` → 编译 `examples\*.jsh` → 跑三套回归
（[test.ps1](tools/test.ps1) 输出用例、[test-errors.ps1](tools/test-errors.ps1) 错误码用例、
[test-native-import.ps1](tools/test-native-import.ps1) 原生导入用例）。
只想构建不跑回归时加 `-SkipTests`。

单独编译/运行/反汇编：

```powershell
compiler\target\release\micc.exe examples\你好.jsh -o build\你好.jmb
build\jmrun.exe build\你好.jmb
compiler\target\release\micc.exe examples\斐波那契.jsh --dump
```

### 2.2 固件构建

```powershell
powershell -ExecutionPolicy Bypass -File tools\build-firmware.ps1 -Example 你好
powershell -ExecutionPolicy Bypass -File tools\build-firmware.ps1 -Example board\ADC采样
```

`-Example` 是相对 `examples\` 的路径（不带 `.jsh`）。产物在 `build\stm32\`：
`app.jmb`、`jm_blob.c`、`jishim.elf/.bin/.hex/.map`。
脚本会内嵌字节码、交叉编译运行时 + 端口 + JMK 内核，并打印 Flash/RAM 占用。

### 2.3 规范同步校验

语言卡与原生表头都是生成物，改完规范来源要重新生成，再让校验通过：

```powershell
compiler\target\release\micc.exe --langcard          # 生成 docs\语言卡.md 与 spec\langcard.json
compiler\target\release\micc.exe --check-langcard    # 校验语言卡没漂移
compiler\target\release\micc.exe --check-native      # 校验 f103_native_table.h 没漂移
compiler\target\release\micc.exe --emit-header       # 重新导出 runtime\include\jm_opcodes.h
compiler\target\release\micc.exe --emit-native-header
```

规范来源：操作码在 [compiler/src/opcodes.rs](compiler/src/opcodes.rs)，板级原生函数在
[compiler/src/native_spec.rs](compiler/src/native_spec.rs)，内建与错误码在运行时 C 侧。

---

## 3. 烧录与串口

### 3.1 SWD 烧录

```powershell
powershell -ExecutionPolicy Bypass -File tools\flash-firmware.ps1          # 只预览命令
powershell -ExecutionPolicy Bypass -File tools\flash-firmware.ps1 -Apply   # 实际写入
```

约束：**只写 + 校验 + 软复位**，不整片擦除、不动选项字节、不开 GDB Server。
需要擦除的扇区由下载过程按需处理。脚本里的 `STM32_Programmer_CLI.exe` 路径是当前机器的
STM32CubeIDE 插件路径；换机器时改 [tools/flash-firmware.ps1](tools/flash-firmware.ps1) 顶部的 `$cli`。

### 3.2 接线

| 板子 | USB 转串口（CH340） |
|------|---------------------|
| PA9（USART1_TX） | RXD |
| PA10（USART1_RX） | TXD |
| GND | GND（必须共地） |

串口参数固定 **115200 8N1**，输出为 UTF-8 中文。
其他板载资源：PC13 = LED（低电平点亮）；PA0–PA7 = ADC1 通道 0–7（12 位，VDDA = 3.3 V）。

### 3.3 抓串口

```powershell
powershell -ExecutionPolicy Bypass -File tools\capture-serial.ps1 -Port COM3 -Seconds 60
```

行为：先打开端口开始录制 → 观察 2 秒并打印实际收到的字节数（收到 0 字节会明确告警）→
提示按复位 → 持续落盘到 `build\uart\uart-<时间戳>.log`（共享读，录制中也能打开）→
每 10 秒打印进度 → 收 0 字节时退出码 2。

脚本是同步阻塞式读取，**必须在前台或工具托管的常驻后台运行**。
不要用 `Start-Process` 起子进程：子进程会随宿主终端被回收，表现是端口开了、
只写了驱动缓冲区里的旧数据，看起来在录其实没有听众。

---

## 4. 目录结构

```
compiler/                  Rust 编译器 micc
  src/lexer.rs               词法：全角归一化 / 缩进 / 中文标识符
  src/parser.rs              语法（砾 Core 子集）
  src/codegen.rs             作用域分析与代码生成
  src/opcodes.rs             操作码唯一事实来源（导出 jm_opcodes.h）
  src/native_spec.rs         板级原生表唯一事实来源（导出 f103_native_table.h）
  src/langcard.rs            语言卡生成器
  src/error.rs               编译期错误码与文案
  src/writer.rs              .jmb 写出与反汇编
runtime/                   C99 运行时（不依赖 HAL，可移植）
  include/                   jishim.h / jm_native.h / jm_port.h / jm_opcodes.h（生成）
  src/                       vm / heap / builtin / loader / native / error
  port/host/                 PC 端口（stdout）与 jmrun 入口
firmware/stm32f103/        板级固件
  startup_stm32f103c8.s      启动与向量表
  stm32f103c8.ld             64 KB Flash / 20 KB RAM 链接脚本
  jmk.c / jmk.h / jmk_port_cm3.s   JMK 任务内核
  stm32f103_port.c / .h      USART1 / SysTick / GPIO / ADC1 / TIM2
  f103_native.c / f103_native_table.h  原生函数实现与生成表
  main.c                     启动横幅、注册原生表、跑内嵌字节码
examples/                  砾源码示例（board/ 下是上板示例）
tests/expected/            主机回归基准输出
tests/errors/              编译期 / 运行期错误用例
tools/                     构建、测试、烧录、串口、工具链定位
spec/langcard.json         语言卡机器可读版本（生成物）
docs/                      路径图、字节码规格、原生函数表、调度模型、语言卡、工具链、验证日志
```

## 5. 常用脚本

| 脚本 | 用途 |
|------|------|
| [tools/build.ps1](tools/build.ps1) | 主机构建 + 三套回归（`-SkipTests` 跳过回归） |
| [tools/build-firmware.ps1](tools/build-firmware.ps1) | 交叉编译固件（`-Example 名字` 切换示例） |
| [tools/flash-firmware.ps1](tools/flash-firmware.ps1) | SWD 烧录（默认预览，`-Apply` 才写） |
| [tools/capture-serial.ps1](tools/capture-serial.ps1) | 串口录制到 `build\uart\`（`-Port` / `-Seconds` / `-Out`） |
| [tools/test.ps1](tools/test.ps1) | 输出回归（逐字节对比 `tests\expected`） |
| [tools/test-errors.ps1](tools/test-errors.ps1) | 错误码回归（E004 / E007 / E009 / E013 / E023 / E102 / E103 / E117） |
| [tools/test-native-import.ps1](tools/test-native-import.ps1) | 原生导入回归（引脚 / ADC / 定时器各三类错误 + 正确用例） |
| [tools/toolchains.ps1](tools/toolchains.ps1) | 工具链定位与版本自检 |
| [tools/fetch-toolchain.ps1](tools/fetch-toolchain.ps1) | 按锁文件下载并校验 ARM 工具链 |

---

## 6. 当前状态与已知限制

**资源预算**：64 KB Flash / 20 KB SRAM。含 JMK 内核、ADC/定时器驱动与闭包运行时的当前固件约
**24.5 KB Flash（37.4%）**、**14.5 KB RAM（70.9%）**；RAM 余量已不大，再加功能前先看 `jishim.map`。

**已上板验证**：USART1 输出、PC13 点灯、JMK 调度与栈池、ADC 零点（PA0 接 GND 读 0 / 0 mV）、
ADC 满量程（PA0 接 3V3 读 4095 / 3300 mV，末位 4094 属正常抖动）、TIM2 1 kHz 计数。
原始日志见 [docs/verification/](docs/verification/)。

**还没验证或做不到的**：

- ADC 中间量程线性度未测。蓝板上没有板载电位器；需外接电位器，或用两个等值电阻分压后再测。
- PWM 方波不能直接当 ADC 源：ADC 采的是瞬时电压，方波会读到 0 或满码。需要 RC 滤波成直流，或改用电阻分压。
- ADC 是 12 位、单次阻塞转换、239.5 周期采样（ADC 时钟 12 MHz），不适合高速连续采样。
- TIM2 是 16 位 1 kHz 计数器，约 65.5 秒回绕；跨回绕的差值要自己处理。
- 整数只有 29 位（±268,435,455）；小数是 Q12 定点（值域约 ±524288，精度 1/4096），
  不是软浮点。闭包按值捕获、捕获变量只读；类、异常、属性访问在编译期显式报错。
- 闭包是引用计数的堆对象；闭包与列表/字典互相引用成环时不会回收（没有 GC），
  反复创建会耗尽堆并报 `E017`。详见语言卡与 [examples/闭包.jsh](examples/闭包.jsh)。
- 整数/整数的 `/` 是**真除法**：结果必须整除，否则运行期报 `E023`。只要有一个操作数是小数，
  `/` 就返回小数，例如 `原始值 / 4095.0 * 3300` 得到 `3299.1943`，不再需要 `//`。
  纯整数缩放仍可用 `原始值 * 3300 // 4095`；需要四舍五入时写成 `(原始值 * 3300 + 2047) // 4095`。
- 小数值域不是无限的：`4094 * 3300.0` 会得到 13,510,200 并报 `E009`，换算时先除后乘。
  打印固定 4 位小数、去尾零但至少保留 1 位（`5.0`、`3299.1943`）。
  详见语言卡「常见错误」与 [examples/小数.jsh](examples/小数.jsh)。

---

## 7. 与基石的关系

- **语法同源**：中文关键字、缩进、全角标点归一化、显示规则都对齐基石。
- **语义是子集**：基石有、砾 v0 没有的写法在**编译期报专门错误码**，不静默降级。
- **实现不共享**：砾有自己的 `.jmb` 与 C 虚拟机，不复用基石那套 24 字节指令 / 宿主回调模型。

## 许可证

**MIT-0（MIT No Attribution）**：可商用、可修改、可再分发，**不要求保留版权声明**。
完整原文见 [LICENSE](LICENSE)。

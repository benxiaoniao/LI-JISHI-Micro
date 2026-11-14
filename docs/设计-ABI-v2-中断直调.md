# ABI v2 设计：中断直调 native

> 状态：**已实现并上板验证（2026-11-14）**。主机 C 回归、五套回归、`--check-native`、`--check-langcard`、固件交叉编译与上板直调均通过。
> 关联：路径图 LI019；[原生函数表.md](原生函数表.md) §7.2 / §10；[设计-原生表容量与ABI.md](设计-原生表容量与ABI.md) §3.1 / §5
> 结论：`JM_NATIVE_ABI_VERSION` 已升到 2，新增 `JM_NATIVE_ISR_SAFE` 与 `jm_native_isr_resolve/call`；普通任务路径不变，ISR 直调只放行 `引脚读` / `引脚写` / `引脚翻转` / `定时器计数` / `PWM占空比`。

---

## 0 一页结论

| 项 | 结论 |
|---|---|
| 收益 | ISR 内直接执行一个已审计的 native 条目，省掉 `jmk_queue_send_from_isr` → 唤醒任务 → PendSV 切换 → 任务里再调 native 的链路。已知 PendSV 切换体实测 **193 周期**（同优先级 A→B，2026-11-08，量程含插桩、不含异常入/出栈）；队列/IPC 还有自己的临界区、入队与唤醒开销。 |
| 代价 | ABI 版本 1 → 2；所有包必须重打（`原生ABI` 精确相等，E129）；每个 ISR-safe 条目要逐条审计；ISR 里没有 VM 错误通道；要补主机与上板回归；**不做 v1/v2 共存**。 |
| 不能做的事 | ISR 里仍然**不能跑字节码、不能分配、不能阻塞、不能碰 VM 栈**。直调只适合「写一个寄存器 / 读一个寄存器」级别的动作；要把数据送回 `.jsh`，仍然走队列。 |
| 决策 | **已升 v2 并实现（2026-11-14）**。原因是用户明确要求按本文实施，而不是继续等触发条件；迁移成本由编译器/运行时/板级生态内部消化（当前外部包为 0）。 |

---

## 1 现状

### 1.1 v1 的 ISR 链路

```
EXTI 边沿
  → exti_serve()                 清挂起位、计数（port 层）
  → irq_post_from_isr()         板级回调，只做一次入队
  → jmk_queue_send_from_isr()   PRIMASK 临界区、写队列、可能 ipc_wake + PENDSVSET
  → （ISR 退出）
  → PendSV                      保存/恢复任务上下文
  → vm 任务恢复
  → .jsh 队列接收 → 调 native → 真正做事
```

代码位置：

- `firmware/stm32f103/stm32f103_port.c` 的 `exti_serve` / `EXTI_IRQHandler`；
- `firmware/stm32f103/f103_native.c` 的 `irq_post_from_isr`；
- `firmware/stm32f103/jmk.c` 的 `jmk_queue_send_from_isr`；
- `runtime/src/jm_native.c` 的 `jm_native_call`。

延迟组成：ISR 入口 + `crit_enter` + 队列写入 + 可能的 `ready_push`/`PENDSVSET` + ISR 退出 + PendSV 切换 + 任务恢复 + `.jsh` 队列接收 + native 分派。**端到端 ISR 延迟未实测**（没有逻辑分析仪/示波器）；唯一可引用的实测是 PendSV 切换体 193 周期（不含硬件异常帧），见 [调度模型.md](调度模型.md) §4.1。

另一个副作用：队列满时 `jmk_queue_send_from_isr` 返回 `-4`，事件被丢弃；`g_exti_count` 仍然增加，所以「计数」与「收到的消息数」会对不上。

### 1.2 为什么 v1 禁止

`docs/原生函数表.md` §7.2 的三条理由仍然成立：

1. native 可能分配（静态堆没有临界区保护）；
2. native 可能访问外设总线，耗时不可预算；
3. VM 的值栈/帧栈是单线程结构，ISR 里调用会破坏它。

v1 的 `JmNativeEntry.flags` / `reserved` 必须为 0；`port_exti_set_hook` 是板级 C 层的口子，不是 native 表的一部分。

---

## 2 目标与非目标

### 2.1 目标

1. 允许 C 侧 ISR 同步调用一个**显式标记为 ISR-safe** 的 native 表项；
2. 不经过 VM、不经过队列、不唤醒任务；
3. 保持「native 表名字/参数个数的唯一事实来源」仍然是 `compiler/src/native_spec.rs`；
4. 普通任务上下文的 native 调用行为完全不变；
5. `.jmb` 格式不变（不加操作码、不改名字表）。

### 2.2 非目标

- 不在 ISR 里执行 `.jsh` 字节码；
- 不在 ISR 里分配/释放堆、创建文本/列表/字典；
- 不在 ISR 里阻塞（等信号量、收队列、延时）；
- 不在 ISR 里抛错/捕获/设置 VM 错误状态；
- v2.0 不提供语言层「把某个 native 绑成中断回调」的语法；
- 不做 native 表热插拔/卸载；
- 不做 v1/v2 共存。

---

## 3 方案比较

| 方案 | 做法 | 优点 | 缺点 | 结论 |
|---|---|---|---|---|
| A. 复用 `JmNativeFn` + flags + ISR 分派 API | 同一张表、同一个函数指针；`jm_native_isr_call` 只放行 `JM_NATIVE_ISR_SAFE` 项；`st == NULL` 表示 ISR 上下文 | 单一事实来源；不复制实现；普通调用路径不变；第三方模块也能声明 | 每个安全项要分支处理 `st == NULL`；flags 是「信任声明」，无法机器验证行为 | **采用（触发后）** |
| B. 独立的 ISR 函数签名/独立表 | 新增 `JmNativeIsrFn`、独立登记 | 类型上强制分离 | 双份表、双份分派、双份 C 实现；与「唯一事实来源」冲突 | 不做 |
| C. 只保留 C hook | `port_exti_set_hook` + 直接调 C 端口函数 | 零 ABI 成本；板级可控 | 第三方 native 模块无法复用；没有表项级校验 | **当前采用（不升 ABI）** |

设计上采用 A；但**当前决策采用 C**。原因见 §12。

---

## 4 ABI v2 契约

### 4.1 版本与 flags

```c
#define JM_NATIVE_ABI_VERSION 2u

/* flags 只允许这一位；其余位必须为 0 */
#define JM_NATIVE_ISR_SAFE 0x01u
```

- `JmNativeEntry` 布局不变（`name / fn / min_args / max_args / flags / reserved`）。
- `flags & ~JM_NATIVE_ISR_SAFE` 非 0 → 登记失败。
- `reserved` 必须为 0。
- `JM_NATIVE_ISR_SAFE` 是**声明**，不是证明：登记层只能校验位，不能校验函数行为；行为审计靠代码评审与 §5 清单。

### 4.2 新增 API

```c
/* 任务上下文：按名字解析一个 ISR-safe 表项，拿到内部 id。
 * 返回 0 成功；-1 名字不存在；-2 存在但不是 ISR-safe；-4 参数为空。 */
int jm_native_isr_resolve(const char *name, uint8_t *id);

/* ISR 上下文：直接调用。
 * id     ：jm_native_isr_resolve 得到的 id
 * args   ：C 侧构造的值数组；借用，调用期间有效；nargs==0 时可为 NULL
 * nargs  ：实参个数
 * out    ：返回值；可为 NULL（不需要结果时）
 * 返回   ：0 成功；负数见下表；正数 30..34 是 native 自己返回的 JM_NE_*。 */
int jm_native_isr_call(uint8_t id, const JmValue *args, uint8_t nargs,
                       JmValue *out);
```

ABI 层返回码：

| 码 | 值 | 含义 |
|---|---|---|
| `JM_NATIVE_ISR_OK` | 0 | 成功 |
| `JM_NATIVE_ISR_E_ID` | -1 | id 非法/未登记 |
| `JM_NATIVE_ISR_E_NOT_SAFE` | -2 | 表项没有 `JM_NATIVE_ISR_SAFE` |
| `JM_NATIVE_ISR_E_ARGC` | -3 | 实参个数不在 `[min_args, max_args]` |
| `JM_NATIVE_ISR_E_PARAM` | -4 | `args == NULL` 但 `nargs != 0`，或其他调用方错误 |
| `JM_NE_PARAM..JM_NE_NOMEM` | 30..34 | native 自己返回的错误码（没有 detail） |

关键约束：

1. `jm_native_isr_call` **不碰 `JmState`**：不 `jm_fail`、不写 `has_err`、不分配、不 retain/release。
2. 传给 `fn` 的 `st` 固定为 `NULL`；ISR-safe 函数必须识别这个上下文。
3. `jm_native_isr_call` 可在任务上下文调用（便于主机测试），但任务上下文应优先用普通 `jm_native_call`。
4. 是否真的在 ISR 里，ABI 不做检查；调用方负责。

### 4.3 值约定

ISR-safe 表项只允许：

- 入参：整数、布尔、空；**禁止文本、列表、字典、实例、闭包**；
- 出参：整数、布尔、空；**禁止文本/容器**；
- 允许的辅助函数：`jm_value_kind` / `jm_value_to_int` / `jm_value_get_int` / `jm_value_int` / `jm_value_bool` / `jm_value_nil`；
- **禁止**：`jm_value_make_text`、`jm_value_text`、`jm_value_retain`、`jm_value_release`、`jm_native_fail`。

### 4.4 与 v1 分派的关系

- 普通调用 `jm_native_call` 完全不变；`flags` 不影响它。
- 同一个表项、同一个 `fn` 指针同时服务两条路径；不复制实现。
- `st == NULL` 是唯一的上下文信号；ISR-safe 函数按它分支。
- `.jmb` 不变：不加操作码、不改名字表、不改 VM；编译器不生成任何「ISR 调用」。

---

## 5 ISR-safe 准入规则

### 5.1 硬性检查清单

一个表项要标 `JM_NATIVE_ISR_SAFE`，必须同时满足：

1. **无分配**：不碰堆、不 `make_text`、不 retain/release；
2. **无阻塞**：不等信号量/队列、不延时、不 `WFI`；
3. **无 VM 状态**：不 `jm_native_fail`、不写 `JmState`、不抛错；
4. **无文本/容器**：入参和出参只限 int/bool/nil；
5. **执行时间有界**：不遍历无界数据、不跑长循环；
6. **可重入**：任务里正在执行同一个/其他 native 时被本 ISR 打断，仍然正确；
7. **可嵌套**：更高优先级 ISR 再打断本 ISR，仍然正确（v2.0 的允许项都满足）；
8. **无惰性初始化**：所需硬件必须由任务上下文预先配置好；未配置时返回 `JM_NE_STATE`；
9. **共享状态只做原子访问**：单个对齐的 8/16/32 位读或写；需要多步时，要么禁止，要么用临界区并明确延迟代价；
10. **不调用会重配外设的 `port_*`**：只允许固定寄存器读/写。

### 5.2 v2.0 允许子集（建议）

| 表项 | 入参 | 前置条件 | 共享状态与竞争 | 结论 |
|---|---|---|---|---|
| `引脚读` | 引脚 | 无 | 读 IDR | 允许 |
| `定时器计数` | 无 | `定时器初始化` 已调用 | 32 位对齐读 `g_ms` | 允许 |
| `PWM占空比` | 千分比 | `PWM启动` 已由任务完成 | 单次写 `TIM4_CCRx`；读 `g_pwm_on/g_pwm_ch/g_pwm_arr1` | 允许；任务同时改频率时允许一个周期的占空比偏差 |
| `引脚写` | 引脚, 电平 | 该脚已由任务上下文配置为 GPIO 输出 | 写 BSRR/BRR；写 `g_pin_out[pin]` 单字节 | **需要 fast path**，见 §5.3 |
| `引脚翻转` | 引脚 | 同上 | 读 `g_pin_out` + 写 BSRR/BRR | **需要 fast path**，见 §5.3 |

明确**不允许**：

- `串口写`（阻塞、共享 USART）；
- `等待毫秒`、`定时器延时`（阻塞）；
- `ADC读` / `ADC初始化`（转换耗时、共享 ADC）；
- 所有 `队列` / `信号量` / `中断绑定` / `中断解除`（内核对象、状态机）；
- 所有 `I2C` / `SPI`（总线事务、阻塞、共享外设）；
- `PWM启动` / `PWM停止`（多寄存器 + 状态机）；
- `引脚写` / `引脚翻转` 在**未预配置**时（会走 `gpio_set_mode`，多寄存器读改写）。

### 5.3 GPIO fast path 设计

现状 `port_gpio_write` 每次调用都执行 `gpio_set_mode(pin, 0x2)`，这是对 CRL/CRH 的读改写，不能在 ISR 里与任务并发。改法：

```c
/* 只读判断：CRL/CRH 的 4 位 nibble 是否为通用推挽输出 */
int port_gpio_is_output(uint32_t pin);

/* ISR 快路径：不配置引脚，只写 BSRR/BRR 与 g_pin_out */
void port_gpio_write_fast(uint32_t pin, int level);
void port_gpio_toggle_fast(uint32_t pin);
```

`n_pin_write` / `n_pin_toggle` 的分支：

```c
if (st == NULL) {                       /* ISR 上下文 */
    if (!port_gpio_is_output(pin)) {
        return JM_NE_STATE;             /* 没有 jm_native_fail，直接返回错误码 */
    }
    port_gpio_write_fast(pin, level);   /* 或 toggle_fast */
    return JM_NATIVE_OK;
}
/* 任务上下文：保持现有 port_gpio_write / toggle */
```

`g_pin_out` 是 48 B 的 `uint8_t` 数组；单字节读写原子。任务与 ISR 同时写同一脚时，以最后一次写为准；这是可接受的「最后写入者胜」语义，不保证顺序。

### 5.4 PWM 的竞争边界

- `PWM占空比` 只写 CCR；写一个 16 位寄存器是原子的。
- 任务同时 `PWM停止`：ISR 可能写一个已经停止的定时器 CCR，无硬件效果；下次 `PWM启动` 会重新写 CCR。
- 任务同时 `PWM启动` 改 `ARR`：ISR 可能用旧的 `g_pwm_arr1` 算 CCR，允许一个周期的占空比偏差；下一条指令后自动恢复。
- 如果产品不能接受这个偏差，就不要把 `PWM占空比` 标 ISR-safe，改用队列路径。

---

## 6 重入边界

### 6.1 任务被随时打断

任务可能在以下位置被打断：

- 字节码指令之间；
- 普通 native 函数内部（例如 `I2C写` 正在等总线）；
- `jm_native_call` 的参数检查/返回写回期间。

因此 ISR-safe 函数必须与**所有普通 native** 共存：

- 不能碰普通 native 正在使用的非原子状态；
- 不能假设「任务没在跑 native」；
- 不能依赖 `JmState` 的任何字段。

### 6.2 嵌套 ISR

Cortex-M3 允许更高优先级中断打断当前 ISR。v2.0 的允许项都是单寄存器读/写，嵌套安全；后续新增项必须单独证明：

- 优先级分层与「同层不嵌套、跨层抢占、最大深度 3」的完整规则见 [设计-中断嵌套与优先级.md](设计-中断嵌套与优先级.md)（LI020）。ISR-safe 直调固定放在**普通层 0x40**，紧急层 0x10 不使用。

- 如果使用静态多步状态，必须用 `PRIMASK` 临界区，或把该项从允许列表移除；
- 临界区会拉长 ISR 延迟，必须在文档里给出最坏路径。

### 6.3 不做什么

- 不允许在普通 native 里调用 `jm_native_isr_call`（除非调用方自己保证没有持有资源）；
- 不允许 ISR-safe 函数调用 `jm_run` / `jm_native_call` / 内核等待 API；
- 不允许 ISR-safe 函数做惰性初始化；
- 不允许 ISR-safe 函数修改 `g_entries` / 登记表。

---

## 7 错误语义

### 7.1 没有 VM 错误通道

ISR 里没有 `JmState`，因此：

- 不能 `jm_native_fail`；
- 不能写 `jm_last_error`；
- 不能 `抛出`/`捕获`；
- 错误不会终止 VM。

ISR-safe 函数失败时直接返回 `JM_NE_*`（30..34），`jm_native_isr_call` 原样返回。detail 文本不可用；需要排查时由板级代码自己计数/置标志。

### 7.2 板级错误计数样例

```c
static volatile uint32_t g_isr_native_errors;

static void isr_action(void)
{
    JmValue args[1];
    args[0] = jm_value_int(13);
    int rc = jm_native_isr_call(g_toggle_id, args, 1, 0);
    if (rc != 0) {
        g_isr_native_errors++;          /* 只计数，不打印（串口不能在 ISR 里阻塞） */
    }
}
```

任务上下文可以再暴露一个只读 native（例如 `中断直调错误计数()`）把计数器读给 `.jsh`；这属于板级实现，不进入 ABI v2 的必需项。

### 7.3 错误分类

| 情况 | 返回 | 处理 |
|---|---|---|
| 表项未标 ISR-safe | `-2` | 编程错误；登记/评审阶段就该发现 |
| id 非法 | `-1` | 解析后没保存好，或表被重新登记（不支持） |
| 实参个数不符 | `-3` | 调用方 bug |
| 参数类型/取值不符 | `30`/`31` | native 内部检查 |
| 前置条件不满足 | `33`（`JM_NE_STATE`） | 任务上下文先做初始化 |
| 硬件失败 | `32` | 板级记录/重试策略由调用方定 |

---

## 8 波及面与迁移

| 文件 | 要改什么 | 是否破坏 |
|---|---|---|
| `compiler/src/native_spec.rs` | `NATIVE_ABI_VERSION = 2`；ISR-safe 集中清单 `ISR_SAFE_ENTRIES`；头文件生成 flags 位 | 是（ABI 版本） |
| `runtime/include/jm_native.h` | 版本、`JM_NATIVE_ISR_SAFE`、两个新 API、返回码 | 是 |
| `runtime/src/jm_native.c` | flags 校验、`jm_native_isr_resolve/call` | 否（普通分派不变） |
| `runtime/include/jm_native_table.h` | 由 `--emit-native-header` 重新生成 | 生成物 |
| `firmware/stm32f103/f103_native.c` | 给允许项加 `isr_safe`；GPIO fast path；ISR 样例 | 是（表 flags） |
| `runtime/port/host/jm_host_native.c` | 同一份宏；fast path 的主机替身 | 是（表 flags） |
| `compiler/src/pack_manifest.rs` | `原生ABI` 缺省 2 | 是（旧包缺省=1 会被 E129） |
| `compiler/src/pack.rs` | E129 文案写清「v2 要求，请重打」 | 是 |
| `compiler/src/langcard.rs` / `spec/langcard.json` / `docs/语言卡.md` | `native_abi_version` 重生成 | 否（消费者只读） |
| `tools/test-packages.ps1` | 用 `原生ABI = 2` 造 E129 的夹具要改成 `3` | 否 |
| `tools/test-native-import.ps1` | 名字/参数不变，预期全绿 | 否 |
| 新增主机 C 回归 | 见 §9.1 | — |
| `docs/原生函数表.md`、`docs/设计-原生表容量与ABI.md` | 版本、flags、ISR 契约 | 否 |

**迁移策略**：一次性切换，不做 v1/v2 共存。`.jmb` 不需要重编（字节码不变），但所有源码包要重新 `micc pack`，或手改 `砾包.toml` 的 `原生ABI = 2`。

---

## 9 验证计划

### 9.1 主机 C 回归（不上板）

新增一个独立的小 C 测试（可放在 `runtime/port/host` 或 `tools` 下的 C 夹具，由 `build.ps1` 调用）：

1. 登记一张表：一个 `JM_NATIVE_ISR_SAFE` 项、一个普通项、一个 `flags` 非法项；
2. `jm_native_isr_resolve("安全项", &id)` 返回 0；解析普通项返回 -2；解析不存在返回 -1；
3. `jm_native_isr_call` 安全项成功、`out` 正确、`st == NULL`；
4. 普通项调用返回 -2，且函数体没有被执行；
5. 实参个数越界返回 -3；
6. 安全项在未预配置时返回 33；
7. 普通 `jm_native_call` 对两项都能正常调用（普通路径不受 flags 影响）；
8. 任务上下文直接调 `jm_native_isr_call` 也能工作（便于测试，不要求上下文检查）。

### 9.2 固件回归（上板）

用现有 EXTI 夹具体系，新增一个「直调」样例：

- 绑定一路软件触发中断；ISR 里 `jm_native_isr_call(id, args, 1, 0)` 调 `引脚翻转`；
- 任务侧只做 `中断计数` 与计数校验，不再 `队列接收`；
- 触发 N 次（建议 10 000 次），检查：
  - 无 HardFault；
  - 计数严格等于触发次数；
  - `g_isr_native_errors == 0`；
  - 任务仍在运行（串口心跳不中断）。

### 9.3 延迟测量

- 用 DWT `CYCCNT`，在 ISR 内量两段：
  - A：`jmk_queue_send_from_isr` 调用前后；
  - B：`jm_native_isr_call` 调用前后。
- 另记 PendSV 次数：直调模式不应因这次动作增加 PendSV。
- 报告口径必须写清：**只量 ISR 内函数体，不含异常入/出栈与端到端任务唤醒**；端到端仍需示波器/逻辑分析仪（LI029）。
- 不预设「必须快多少」；先拿到两组数，再决定是否值得。
- **实测（2026-11-14，上板）**：直调 `avg=326 / max=330` 周期，队列发送 `avg=143 / max=145` 周期（4 次样本，含插桩）。直调函数体本身比一次队列入队更贵（GPIO 前置判断 + BSRR 写 + `g_pin_out`），但队列路径还另有 PendSV 切换 193 周期与任务侧接收/分派，二者相加已超过直调；端到端仍未测。

### 9.4 体积

- 实现后用 `build-firmware.ps1 -Example 类` 与 `-Example 外部中断` 实测 Flash/RAM；
- 不推算；如果增量超过 Flash 余量（当前 11 284 B），先砍允许项或只保留 C hook 方案。

### 9.5 全量回归

- `test.ps1`、`test-errors.ps1`、`test-debug.ps1`、`test-packages.ps1`、`test-native-import.ps1`；
- `micc --check-native`、`micc --check-langcard`；
- 固件交叉编译。

### 9.6 实施结果（2026-11-14）

| 项 | 结果 |
|---|---|
| 主机 C 回归 | 新增 [tests/native-abi-v2.c](../tests/native-abi-v2.c)，`build.ps1` 步骤 5/6 编译并运行；**21 项全过**（登记 flags/reserved、resolve/call、st == NULL、-1/-2/-3/-4、正数 33 透传、out 可为 NULL、普通分派不受 flags 影响） |
| 五套回归 | `test` 58/58、`test-errors` 96/96、`test-debug` 54/54、`test-packages` 69/69、`test-native-import` 37/37 |
| 工具链校验 | `micc --check-native`、`micc --check-langcard` 均通过（语言卡 25 个示例可编译） |
| 固件体积（`-Example 类`） | FLASH **54 428 B** / RAM **17 404 B**；v1 基线 54 252 B / 17 404 B → **+176 B Flash，RAM 不变** |
| 直调样例编译（`-Example 外部中断 -Define "-DJM_ISR_DIRECT_DEMO=1"`） | 通过，含 DWT 计时的版本 FLASH 55 128 B / RAM 17 436 B；样例默认关闭，仅用于上板核对 |
| 上板（2026-11-14，外部中断 + 直调样例） | 7 张表登记成功；`[直调] 成功 4 次，失败 0 次`；4 条 EXTI 事件按 0/1/0/1 收齐，`PA0/PA1 计数 = 2/2`，解除后不再触发；无 HardFault，后续 LED/IPC 任务持续运行；`vm` 栈峰值 416 B / 1024 B |
| ISR 周期（DWT `CYCCNT`，4 次软件触发） | 直调 `avg=326 / max=330`；同一 ISR 内队列发送 `avg=143 / max=145`。量程**只含 ISR 内函数体**，含两次 `CYCCNT` 读与计数插桩、未扣除，不含异常入/出栈、PendSV 切换与任务侧接收 |
| 未覆盖 | 10 000 次连续触发压力、真实外部边沿、端到端示波器/逻辑分析仪延迟（LI029）仍未做 |

---

## 10 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| 第三方模块虚报 ISR-safe | ISR 里分配/阻塞/破坏状态，偶发 HardFault | `flags` 是信任声明；准入清单 + 代码评审；不允许第三方自行登记未审计表 |
| 共享状态竞争 | 偶发丢写/读到半更新值 | 只允许单寄存器原子操作；GPIO 配置只在任务上下文做；复杂项不进允许列表 |
| ABI 升版 | 所有包需重打；旧二进制包被 E129 拒绝 | 一次性切换；E129 文案写清；当前外部包为 0，迁移成本最低 |
| ISR 延迟变长 | 高优先级中断被拉长，影响实时性 | 允许项执行时间有界；禁止循环/总线/分配；逐项测量最坏周期 |
| 错误不可见 | ISR 失败只返回码，没有 detail | 板级错误计数器 + 任务上下文读取；调试期用 SWD |
| 工具链漂移 | 头文件/flags/语言卡不一致 | `native_spec.rs` 单一来源；`--check-native` + `--check-langcard` |
| 误用语言层 | `.jsh` 以为能在 ISR 里跑逻辑 | 明确非目标；v2.0 不提供语言语法 |

---

## 11 触发后的实施顺序

1. `native_spec.rs`：版本 2、`ISR_SAFE_ENTRIES` 集中清单、头文件生成；
2. `jm_native.h`：flags、两个 API、返回码；
3. `jm_native.c`：登记校验、resolve/call；
4. 端口层：GPIO `is_output` / `write_fast` / `toggle_fast`；`f103_native.c` 与 `jm_host_native.c` 的 flags；
5. `pack_manifest.rs` / `pack.rs` / 语言卡：ABI 2 与 E129；
6. 主机 C 回归 + 固件直调样例；
7. 五套回归 + 体积/周期实测；
8. 文档：本文转「已实现」，同步 `原生函数表.md`、`设计-原生表容量与ABI.md`、语言卡、README。

---

## 12 决策记录

### 12.1 已实施（2026-11-14）

**已升 ABI v2 并按 §11 顺序实现；LI019 从「按需触发」转为「已实现」。**

原先「暂不升」的四条理由仍然成立，只是这次由用户明确要求实施而覆盖：

1. 板级 ISR 已经有 `port_exti_set_hook`，直接调 C 端口函数即可完成「ISR 里翻转一个引脚 / 更新一个寄存器」，不需要 native 表；
2. 现有唯一的使用者是板级固件自身，没有第三方 native 模块，ABI 表带来的「统一校验/可移植」收益暂时为零；
3. 升版会让所有包重打，并引入 flags 信任、共享状态审计、ISR 错误语义等长期维护面；
4. 当前没有证据表明队列链路是产品瓶颈；在没有测量数据前，先不为「可能省一次投递」付 ABI 成本。

实施时保留了 C hook 路径（`port_exti_set_hook` 仍默认投递队列），ISR 直调是**新增可选路径**，不是替换。

### 12.2 原触发条件（已不再阻塞实施）

1. 出现**第三方 native 模块**，且它必须在 ISR 里同步执行，不能由板级 C hook 代劳；
2. `.jsh` 需要按名字选择/绑定一个 ISR 回调（语言层需求），且明确不要求在 ISR 里跑字节码；
3. 有实测证据表明「中断→队列→任务→native」的延迟或丢事件率已经影响产品指标；
4. 需要把 ISR-safe 子集作为对外发布契约（例如给别人用的板级包）。

### 12.3 触发前的替代做法

- ISR 里只做「清挂起、计数、投递」；
- 需要立即动作的，在 ISR 里直接调 C 端口函数（`port_gpio_write_fast` 之类的内部函数可以先在 C 层加，不必进 ABI）；
- 数据仍然走 `jmk_queue_send_from_isr` → 任务 → native；
- 需要观测丢事件时，同时读 `队列数量` 与 `中断计数`（现有 EXT I 文档已给出这个方法）。

---

## 13 附：与现有文档的关系

- [原生函数表.md](原生函数表.md) §7.2 / §10 已同步为 v2 契约：`flags` 只允许 `JM_NATIVE_ISR_SAFE`，ISR 直调只走 `jm_native_isr_call`。
- [设计-原生表容量与ABI.md](设计-原生表容量与ABI.md) §3.1 把「增加中断上下文可调用 flags 约定」列为 ABI 破坏项；本次正是该破坏项的一次性切换。
- 本文不改变 `.jmb`、JMB 版本、操作码与 VM；当前 `JM_NATIVE_ABI_VERSION` 为 **2**。

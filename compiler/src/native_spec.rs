//! 板级原生函数表的编译期规格。
//!
//! 这是「导入 模块」与 C 侧 `JmNativeEntry` 表的**唯一事实来源**：
//! - `micc` 编译 `.jsh` 时据此做名字存在性、导入状态与参数个数的静态检查；
//! - `micc --emit-native-header` 据此生成 `runtime/include/jm_native_table.h`，
//!   C 侧不再手写名字表（名字、参数个数、C 函数名都从同一份规格来）；
//!   板级（`f103_native.c`）与主机端口（`jm_host_native.c`）展开同一份宏；
//! - `micc --check-native` 校验入库的生成头与规格一致，防止有人只改一侧。
//!
//! 运行期仍然只按名字绑定：`.jmb` 里只有名字，`jm_native_bind_name` 在 `jm_run`
//! 时把它接到登记过的 native 表项。编译期检查与运行期分派因此不会各有一份名字表。

use std::collections::HashSet;
use std::path::PathBuf;

/// Native ABI 版本，与 `runtime/include/jm_native.h` 的 `JM_NATIVE_ABI_VERSION` 一致。
/// 语言卡与打包都校验这一处；`micc --check-langcard` 会比对两边是否漂移。
pub const NATIVE_ABI_VERSION: u8 = 1;

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

pub const MODULES: &[NativeModuleSpec] = &[
    NativeModuleSpec {
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
    },
    NativeModuleSpec {
        name: "ADC",
        table: "f103_adc",
        c_prefix: "F103_ADC",
        entries: &[
            NativeEntrySpec {
                name: "ADC初始化",
                min_args: 0,
                max_args: 0,
                c_fn: "n_adc_init",
                doc: "上电并校准 ADC1；PA0–PA7 对应通道 0–7，参考电压 VDDA（3.3V）。",
            },
            NativeEntrySpec {
                name: "ADC读",
                min_args: 1,
                max_args: 1,
                c_fn: "n_adc_read",
                doc: "对指定通道（0–7，对应 PA0–PA7）做一次单次转换，返回 0–4095（12 位）。",
            },
        ],
    },
    NativeModuleSpec {
        name: "定时器",
        table: "f103_timer",
        c_prefix: "F103_TIMER",
        entries: &[
            NativeEntrySpec {
                name: "定时器初始化",
                min_args: 0,
                max_args: 0,
                c_fn: "n_timer_init",
                doc: "启动毫秒基准（SysTick 1 kHz 计数）；TIM2 另配为 1 MHz 自由运行。不调用也能延时/计数，但请按约定先初始化。",
            },
            NativeEntrySpec {
                name: "定时器延时",
                min_args: 1,
                max_args: 1,
                c_fn: "n_timer_delay",
                doc: "按 1 ms 实时计数忙等 0–60000 毫秒；需先调用 定时器初始化。",
            },
            NativeEntrySpec {
                name: "定时器计数",
                min_args: 0,
                max_args: 0,
                c_fn: "n_timer_count",
                doc: "返回当前毫秒计数（0–65535，约 65.5 秒回绕，取自 SysTick 1 ms 计数）。",
            },
            NativeEntrySpec {
                name: "PWM启动",
                min_args: 3,
                max_args: 3,
                c_fn: "n_pwm_start",
                doc: "PWM启动(引脚, 频率赫兹, 占空比千分比)；引脚 22–25（PB6–PB9，对应 TIM4_CH1–CH4），频率 1–10000 Hz，占空比 0–1000（千分比，0=常低，1000=常高）。板上用 TIM4，不占 TIM2 毫秒基准；PC13 没有定时器通道，不能 PWM。",
            },
            NativeEntrySpec {
                name: "PWM占空比",
                min_args: 1,
                max_args: 1,
                c_fn: "n_pwm_set_duty",
                doc: "PWM占空比(千分比)，范围 0–1000；需先调用 PWM启动，未启动报 E033。改占空比不改变频率。",
            },
            NativeEntrySpec {
                name: "PWM停止",
                min_args: 0,
                max_args: 0,
                c_fn: "n_pwm_stop",
                doc: "关闭 PWM 输出并把引脚拉低；需先调用 PWM启动，未启动报 E033。",
            },
        ],
    },
    NativeModuleSpec {
        name: "队列",
        table: "ipc_queue",
        c_prefix: "JM_QUEUE",
        entries: &[
            NativeEntrySpec {
                name: "队列创建",
                min_args: 1,
                max_args: 1,
                c_fn: "n_queue_create",
                doc: "队列创建(槽数)，槽数 1–8；返回整数句柄（1 起）。队列项固定是 32 位整数。",
            },
            NativeEntrySpec {
                name: "队列发送",
                min_args: 2,
                max_args: 3,
                c_fn: "n_queue_send",
                doc: "队列发送(句柄, 整数[, 超时毫秒])；成功返回 真，队列满且超时返回 假。超时默认 0（不等待），范围 0–60000。",
            },
            NativeEntrySpec {
                name: "队列接收",
                min_args: 1,
                max_args: 2,
                c_fn: "n_queue_recv",
                doc: "队列接收(句柄[, 超时毫秒])；有数据返回整数，超时返回 空。超时默认 0。",
            },
            NativeEntrySpec {
                name: "队列数量",
                min_args: 1,
                max_args: 1,
                c_fn: "n_queue_count",
                doc: "返回队列里当前的整数条数。",
            },
            NativeEntrySpec {
                name: "队列销毁",
                min_args: 1,
                max_args: 1,
                c_fn: "n_queue_destroy",
                doc: "队列销毁(句柄)；成功释放该队列控制块并返回 真，句柄无效或已销毁返回 假。正在等待它的任务按“未成功”返回（发送/接收表现为超时）。槽位缓冲归调用方，不随队列释放。",
            },
        ],
    },
    NativeModuleSpec {
        name: "信号量",
        table: "ipc_sem",
        c_prefix: "JM_SEM",
        entries: &[
            NativeEntrySpec {
                name: "信号量创建",
                min_args: 1,
                max_args: 2,
                c_fn: "n_sem_create",
                doc: "信号量创建(初值[, 上限])；上限默认 1（二值信号量），两者都在 0–255；返回整数句柄（1 起）。",
            },
            NativeEntrySpec {
                name: "信号量等待",
                min_args: 1,
                max_args: 2,
                c_fn: "n_sem_wait",
                doc: "信号量等待(句柄[, 超时毫秒])；计数大于 0 时减一并返回 真，超时返回 假。超时默认 0。",
            },
            NativeEntrySpec {
                name: "信号量给出",
                min_args: 1,
                max_args: 1,
                c_fn: "n_sem_post",
                doc: "计数加一或唤醒一个等待者；已到上限返回 假。",
            },
        ],
    },
    NativeModuleSpec {
        name: "外部中断",
        table: "f103_exti",
        c_prefix: "JM_IRQ",
        entries: &[
            NativeEntrySpec {
                name: "中断绑定",
                min_args: 2,
                max_args: 3,
                c_fn: "n_irq_bind",
                doc: "中断绑定(引脚, 队列句柄[, 边沿])；引脚 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15），EXTI 线号 = 引脚 % 16；线 0–4 走 IRQ6–10，线 5–9 走 IRQ23，线 10–15 走 IRQ40。边沿 0=上升（默认）/1=下降/2=双沿。同一条线只能选一个端口，换绑会解绑同线的旧引脚。中断里只把引脚号投进队列，不执行砾代码。",
            },
            NativeEntrySpec {
                name: "中断触发",
                min_args: 1,
                max_args: 1,
                c_fn: "n_irq_trigger",
                doc: "中断触发(引脚)，软件触发该 EXTI 线一次（板上写 SWIER，主机直接模拟投递）；已绑定返回 真，未绑定返回 假。引脚编码同 中断绑定。",
            },
            NativeEntrySpec {
                name: "中断计数",
                min_args: 1,
                max_args: 1,
                c_fn: "n_irq_count",
                doc: "返回该引脚自绑定以来进入中断服务函数的次数。引脚编码同 中断绑定。",
            },
            NativeEntrySpec {
                name: "中断解除",
                min_args: 1,
                max_args: 1,
                c_fn: "n_irq_unbind",
                doc: "取消该引脚的绑定并关闭这条 EXTI 线（同组还有别的线在用时只关本线），之后 中断触发 返回 假。引脚编码同 中断绑定。",
            },
        ],
    },
    NativeModuleSpec {
        name: "总线",
        table: "f103_bus",
        c_prefix: "F103_BUS",
        entries: &[
            NativeEntrySpec {
                name: "I2C初始化",
                min_args: 3,
                max_args: 3,
                c_fn: "n_i2c_init",
                doc: "I2C初始化(SCL引脚, SDA引脚, 速率千赫兹)；引脚编码 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15）。SCL=26 / SDA=27（PB10/PB11）走硬件 I2C2；其它组合退回软件位翻转（开漏输出，依赖外部上拉）。速率 10–400 kHz，硬件按 APB1 时钟算 CCR/TRISE。返回 真=硬件通道，假=软件模拟。",
            },
            NativeEntrySpec {
                name: "I2C写",
                min_args: 3,
                max_args: 3,
                c_fn: "n_i2c_write",
                doc: "I2C写(设备地址, 控制字节, 数据字节)；一次完整写事务：START + 地址(写) + 控制 + 数据 + STOP。地址 0–127，控制/数据 0–255。SSD1306 用 控制 0x00 发命令、0x40 发显存数据。",
            },
            NativeEntrySpec {
                name: "I2C写块",
                min_args: 3,
                max_args: 8,
                c_fn: "n_i2c_write_block",
                doc: "I2C写块(设备地址, 控制字节, 数据1[, 数据2 … 数据5])；在同一次事务里连续写 1–5 个数据字节（参数上限 8 个），适合初始化命令段与成段显存写入。参数语义同 I2C写。",
            },
            NativeEntrySpec {
                name: "I2C探测",
                min_args: 1,
                max_args: 1,
                c_fn: "n_i2c_probe",
                doc: "I2C探测(设备地址)：只发地址并等应答，返回 真/假，用于扫描 0x00–0x7F 找设备。主机端口对 0x3C 返回 真（模拟 SSD1306）。",
            },
        ],
    },
];

pub const NATIVE_HEADER_PATH: &str = "runtime/include/jm_native_table.h";

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

/// 生成原生表头（`runtime/include/jm_native_table.h`）。
/// 每个模块生成独立的一组宏：`<前缀>_TABLE_NAME` / `_ENTRY_COUNT` / `_TABLE_INIT`。
/// 板级与主机端口都包含这一份头文件，只展开各自需要的表。
pub fn render_c_header() -> Result<String, String> {
    validate()?;

    let mut s = String::new();
    s.push_str("/* 本文件由 `micc --emit-native-header` 从 compiler/src/native_spec.rs 生成，不要手改。 */\n");
    for m in MODULES {
        let guard = format!("{}_TABLE_H", m.c_prefix);
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
        s.push_str(&format!("\n#endif /* {} */\n\n", guard));
    }
    Ok(s)
}

fn header_path() -> PathBuf {
    crate::langcard::repo_root().join(NATIVE_HEADER_PATH)
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
            NATIVE_HEADER_PATH
        ));
    }
    println!("原生表头通过校验：{} 与规格一致", path.display());
    Ok(())
}

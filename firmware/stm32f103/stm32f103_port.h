#ifndef STM32F103_PORT_H
#define STM32F103_PORT_H

#include <stdint.h>

/*
 * 砾（LI）STM32F103C8T6 板级端口
 * USART1：PA9 = TX，PA10 = RX，115200 8N1
 * 板载 LED：PC13（低电平点亮）
 * 时基：SysTick 1 ms
 */

/* 一次性初始化：时钟 → UART → 中断优先级 → SysTick → LED */
void port_init(void);

/* ---- 中断优先级（Cortex-M3，4 位抢占优先级，数值越小越急） ----
 * 分层与规则见 docs/设计-中断嵌套与优先级.md：
 *   故障 0x00 / 紧急 0x10 / 普通 0x40 / 慢速 0x80 / 内核 0xFF。
 * 外部 IRQ 只允许 0x10 / 0x40 / 0x80；0x00 与 0xFF 留给故障与内核向量。 */
#define PORT_PRIO_FAULT     0x00u
#define PORT_PRIO_CRITICAL  0x10u
#define PORT_PRIO_NORMAL    0x40u
#define PORT_PRIO_SLOW      0x80u
#define PORT_PRIO_KERNEL    0xFFu

/* 受控写入：irq 为 NVIC 中断号（0..67）；返回 0 成功，-1 非法 irq 或非法层。 */
int port_irq_set_priority(uint32_t irq, uint8_t prio);
/* 只读：返回高 4 位优先级；irq 越界返回 0xFF。 */
uint8_t port_irq_get_priority(uint32_t irq);

/* 调试构建：启动时打印内核向量优先级；绑定时打印 EXTI 优先级。 */
#ifndef JM_IRQ_DEBUG
#define JM_IRQ_DEBUG 0
#endif

/* 上板自检构建：用 PA0(0x40) 与 PA1(0x10) 验证跨层抢占，详见设计文档 §8.3。 */
#ifndef JM_IRQ_NEST_DEMO
#define JM_IRQ_NEST_DEMO 0
#endif
#if JM_IRQ_NEST_DEMO
void port_irq_nest_demo_run(void);
#endif

/* MSP 哨兵水位：启动时用 0xA5 填满主栈未使用区，任务上下文扫描峰值。
 * 需要配合 JM_IRQ_NEST_DEMO=1 才有 10 000 次嵌套压力。 */
#ifndef JM_MSP_WATERMARK
#define JM_MSP_WATERMARK 0
#endif
#if JM_MSP_WATERMARK
void port_msp_watermark_init(void);
uint32_t port_msp_watermark_used(void);
uint32_t port_msp_watermark_total(void);
#endif

/* 故障现场测试：故意执行 UDF，验证 HardFault 打印后停机。默认关闭。 */
#ifndef JM_FAULT_DEMO
#define JM_FAULT_DEMO 0
#endif
#if JM_FAULT_DEMO
void port_fault_demo_trigger(void);
#endif

/* 当前系统时钟（Hz） */
uint32_t port_sysclk(void);

/* 简单日志输出（不依赖 printf） */
void port_log(const char *s);
void port_log_u32(uint32_t v);
void port_log_hex(uint32_t v);

/* 运行时间（毫秒） */
uint32_t port_uptime_ms(void);

/* 板载 LED */
void port_led_set(int on);
void port_led_toggle(void);

/* 板级 GPIO：引脚编码 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15）。
 * 写会把该脚配成推挽输出 2 MHz；读只读输入寄存器、不改模式；翻转按最后写入的电平取反。
 * PC13 是板载 LED（低电平点亮），走同一套接口。 */
void port_gpio_write(uint32_t pin, int level);
int  port_gpio_read(uint32_t pin);
int  port_gpio_toggle(uint32_t pin);
/* ISR 快路径：要求调用方已由任务上下文把该脚配成推挽输出；port_gpio_is_output 只读判断。 */
int  port_gpio_is_output(uint32_t pin);
void port_gpio_write_fast(uint32_t pin, int level);
void port_gpio_toggle_fast(uint32_t pin);

/* 忙等延时（不带调度，供 native 函数使用） */
void port_delay_ms(uint32_t ms);

/* ADC1：PA0–PA7 = 通道 0–7，12 位，参考电压 VDDA（3.3V） */
void port_adc_init(void);
/* 返回 0 = 成功；-1 = 未初始化或通道越界 */
int  port_adc_read(uint32_t channel, uint32_t *raw);

/* TIM2：1 kHz 自由运行计数器（毫秒级，16 位，约 65.5 秒回绕） */
void port_timer_init(void);
/* 返回 0 = 成功；-1 = 未初始化 */
int  port_timer_delay_ms(uint32_t ms);
int  port_timer_count(uint32_t *ms);

/* TIM4 PWM：引脚编码 22–25 = PB6–PB9 = TIM4_CH1–CH4。
 * 频率 1–10000 Hz，占空比 0–1000（千分比）。占用 TIM4，不影响 TIM2 毫秒基准。
 * 返回 0 = 成功；-1 = 参数越界 / 未启动。 */
int  port_pwm_start(uint32_t pin, uint32_t freq_hz, uint32_t duty_permille);
int  port_pwm_set_duty(uint32_t duty_permille);
int  port_pwm_stop(void);

/* I2C：SCL/SDA 引脚编码 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15）。
 * 26/27（PB10/PB11）走硬件 I2C2（APB1，72 MHz 主频时 36 MHz）；其它组合走软件位翻转，
 * 引脚配成开漏输出，必须有外部上拉（OLED 模块自带 4.7k）。速率 10–400 kHz。
 * port_i2c_init 返回 1 = 硬件通道，0 = 软件模拟，-1 = 参数越界/初始化失败。
 * port_i2c_probe 返回 1 = 有 ACK，0 = 无 ACK，-1 = 未初始化/地址越界。 */
int  port_i2c_init(uint32_t scl_pin, uint32_t sda_pin, uint32_t speed_khz);
int  port_i2c_write(uint32_t addr, uint32_t ctrl, const uint32_t *data, uint32_t len);
int  port_i2c_probe(uint32_t addr);

/* SPI：引脚编码同上。5/6/7（PA5/PA6/PA7 = SCK/MISO/MOSI）走硬件 SPI1（主机、模式 0、
 * MSB 先行、NSS 软件），分频只能是 2/4/8/16/32/64/128/256；其它组合走软件位翻转。
 * port_spi_init 返回 1 = 硬件 SPI1，0 = 软件模拟，-1 = 参数越界/初始化失败。
 * port_spi_transfer 返回 0 = 成功（收到字节写 *got），-1 = 未初始化/总线超时。 */
int  port_spi_init(uint32_t sck_pin, uint32_t miso_pin, uint32_t mosi_pin,
                   uint32_t divider);
int  port_spi_transfer(uint32_t byte, uint32_t *got);
int  port_spi_write(const uint32_t *data, uint32_t len);

/* EXTI：PA0–PA4 对应 EXTI0–4，边沿 0=上升 1=下降 2=双沿。
 * 引脚会被改成下拉输入；中断里只做计数与 hook 回调，hook 不许碰 VM。 */
void port_exti_bind(uint32_t pin, uint32_t edge);
void port_exti_unbind(uint32_t pin);
void port_exti_sw_trigger(uint32_t pin);
uint32_t port_exti_count(uint32_t pin);
void port_exti_set_hook(void (*hook)(uint32_t line));

#endif

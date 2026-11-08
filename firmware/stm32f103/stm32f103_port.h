#ifndef STM32F103_PORT_H
#define STM32F103_PORT_H

#include <stdint.h>

/*
 * 砾（LI）STM32F103C8T6 板级端口
 * USART1：PA9 = TX，PA10 = RX，115200 8N1
 * 板载 LED：PC13（低电平点亮）
 * 时基：SysTick 1 ms
 */

/* 一次性初始化：时钟 → UART → SysTick → LED */
void port_init(void);

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

/* 板级 GPIO（v0 只开放 PC13 一个输出脚） */
void port_gpio_write(uint32_t pin, int level);
int  port_gpio_read(uint32_t pin);

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

/* EXTI：PA0–PA4 对应 EXTI0–4，边沿 0=上升 1=下降 2=双沿。
 * 引脚会被改成下拉输入；中断里只做计数与 hook 回调，hook 不许碰 VM。 */
void port_exti_bind(uint32_t pin, uint32_t edge);
void port_exti_unbind(uint32_t pin);
void port_exti_sw_trigger(uint32_t pin);
uint32_t port_exti_count(uint32_t pin);
void port_exti_set_hook(void (*hook)(uint32_t line));

#endif

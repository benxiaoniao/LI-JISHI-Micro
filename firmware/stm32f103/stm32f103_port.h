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

#endif

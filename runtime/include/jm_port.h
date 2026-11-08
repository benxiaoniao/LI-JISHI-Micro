#ifndef JM_PORT_H
#define JM_PORT_H

#include <stdint.h>

/* 平台端口：运行时只通过这几个函数与外界打交道。 */

/* 输出一段文本（主机：stdout；STM32：UART）。 */
void jm_port_write(const char *s, uint32_t n);

/* 单调毫秒计数（绿色调度用它算截止时间与最近唤醒点）。 */
uint32_t jm_port_millis(void);

/* 睡眠指定毫秒（主机：Sleep/nanosleep；STM32：jmk_delay）。 */
void jm_port_sleep_ms(uint32_t ms);

/* 让出一次 CPU，给同级/低优先级的其它任务机会（主机：SwitchToThread；
 * STM32：jmk_yield）。绿色任务在没有别的就绪任务时用它。 */
void jm_port_yield(void);

#endif

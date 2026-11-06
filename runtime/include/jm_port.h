#ifndef JM_PORT_H
#define JM_PORT_H

#include <stdint.h>

/* 平台端口：运行时只通过这两个函数与外界打交道。 */

/* 输出一段文本（主机：stdout；STM32：UART）。 */
void jm_port_write(const char *s, uint32_t n);

/* 毫秒计数（任务调度用；v0 还未用到，先留接口）。 */
uint32_t jm_port_millis(void);

#endif

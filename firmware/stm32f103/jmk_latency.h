#ifndef JMK_LATENCY_H
#define JMK_LATENCY_H

#include <stdint.h>

/*
 * 延迟测量（LI029，仅 JM_LATENCY_DEMO=1 的取证构建）：
 *   - PendSV 入口/出口由 build-firmware.ps1 生成的插桩汇编调用；
 *   - SysTick / EXTI 在端口层调用；
 *   - 统计只累加 CYCCNT 差值，不在计时区内打印。
 * 生产固件不编译本文件，也不含任何 CYCCNT 读写。
 */

void jmk_lat_switch_enter(void);
void jmk_lat_switch_exit(void);
void jmk_lat_tick_enter(void);
void jmk_lat_tick_exit(void);
void jmk_lat_isr_enter(void);

/* 建立演示任务并启动调度器；不返回。 */
void jmk_latency_demo(void);

#endif

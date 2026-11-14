#ifndef F103_NATIVE_H
#define F103_NATIVE_H

/*
 * F103C8T6 板级原生函数表（Native ABI v2）
 * 表名、项数与表项内容由 compiler/src/native_spec.rs 生成（runtime/include/jm_native_table.h），
 * 这里不再手写名字表，避免与 micc 的编译期检查各存一份。
 * 覆盖：PC13 引脚写/读/翻转、USART1 写文本、忙等延时；另登记跨平台的「任务」表。
 */

#include "jm_native_table.h"

/* ISR 直调样例开关（默认关）：-DJM_ISR_DIRECT_DEMO=1 时，
 * EXTI 回调里用 jm_native_isr_call 直接翻转 PC13，同时保留原来的队列投递。 */
#ifndef JM_ISR_DIRECT_DEMO
#define JM_ISR_DIRECT_DEMO 0
#endif

int f103_native_register(void);

#if JM_ISR_DIRECT_DEMO
/* 任务上下文调用：解析 ISR-safe 的 引脚翻转，并把 PC13 预配置成推挽输出。 */
void f103_isr_direct_demo_init(void);
/* 任务上下文调用：打印直调成功/失败计数。 */
void f103_isr_direct_demo_report(void);
#endif

#endif

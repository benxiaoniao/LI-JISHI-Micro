#ifndef F103_NATIVE_H
#define F103_NATIVE_H

/*
 * F103C8T6 板级原生函数表（Native ABI v1）
 * 表名、项数与表项内容由 compiler/src/native_spec.rs 生成（runtime/include/jm_native_table.h），
 * 这里不再手写名字表，避免与 micc 的编译期检查各存一份。
 * 覆盖：PC13 引脚写/读/翻转、USART1 写文本、忙等延时；另登记跨平台的「任务」表。
 */

#include "jm_native_table.h"

int f103_native_register(void);

#endif

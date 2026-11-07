#ifndef F103_NATIVE_H
#define F103_NATIVE_H

/*
 * F103C8T6 板级原生函数表（Native ABI v1）
 * 表名：f103_board
 * 覆盖：PC13 引脚写/读/翻转、USART1 写文本、忙等延时
 */

#define F103_NATIVE_TABLE_NAME "f103_board"
#define F103_NATIVE_ENTRY_COUNT 5u

int f103_native_register(void);

#endif

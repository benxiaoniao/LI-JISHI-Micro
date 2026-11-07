/* 本文件由 `micc --emit-native-header` 从 compiler/src/native_spec.rs 生成，不要手改。 */
/* 原生表 f103_board：5 项 */
#ifndef F103_NATIVE_TABLE_H
#define F103_NATIVE_TABLE_H

#define F103_NATIVE_TABLE_NAME "f103_board"
#define F103_NATIVE_ENTRY_COUNT 5u

/* 在 JmNativeEntry 数组初始化处展开；名字、参数个数、C 函数名都来自同一份规格。 */
#define F103_NATIVE_TABLE_INIT \
    {"引脚写", n_pin_write, 2u, 2u, 0u, 0u}, \
    {"引脚读", n_pin_read, 1u, 1u, 0u, 0u}, \
    {"引脚翻转", n_pin_toggle, 1u, 1u, 0u, 0u}, \
    {"串口写", n_uart_write, 1u, 1u, 0u, 0u}, \
    {"等待毫秒", n_delay, 1u, 1u, 0u, 0u}

#endif /* F103_NATIVE_TABLE_H */

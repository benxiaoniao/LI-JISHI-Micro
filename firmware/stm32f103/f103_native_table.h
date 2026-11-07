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

/* 原生表 f103_adc：2 项 */
#ifndef F103_ADC_TABLE_H
#define F103_ADC_TABLE_H

#define F103_ADC_TABLE_NAME "f103_adc"
#define F103_ADC_ENTRY_COUNT 2u

/* 在 JmNativeEntry 数组初始化处展开；名字、参数个数、C 函数名都来自同一份规格。 */
#define F103_ADC_TABLE_INIT \
    {"ADC初始化", n_adc_init, 0u, 0u, 0u, 0u}, \
    {"ADC读", n_adc_read, 1u, 1u, 0u, 0u}

#endif /* F103_ADC_TABLE_H */

/* 原生表 f103_timer：3 项 */
#ifndef F103_TIMER_TABLE_H
#define F103_TIMER_TABLE_H

#define F103_TIMER_TABLE_NAME "f103_timer"
#define F103_TIMER_ENTRY_COUNT 3u

/* 在 JmNativeEntry 数组初始化处展开；名字、参数个数、C 函数名都来自同一份规格。 */
#define F103_TIMER_TABLE_INIT \
    {"定时器初始化", n_timer_init, 0u, 0u, 0u, 0u}, \
    {"定时器延时", n_timer_delay, 1u, 1u, 0u, 0u}, \
    {"定时器计数", n_timer_count, 0u, 0u, 0u, 0u}

#endif /* F103_TIMER_TABLE_H */


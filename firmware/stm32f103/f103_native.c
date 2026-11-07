#include "jm_internal.h"
#include "jm_native.h"
#include "jm_port.h"
#include "stm32f103_port.h"
#include "f103_native.h"

/*
 * F103C8T6 板级原生函数表。
 * 约束（与 docs/原生函数表.md 一致）：
 *   - 只在任务/主上下文调用，不得进 ISR
 *   - 不分配、不保存借用指针
 *   - PC13 低电平点亮板载 LED，所以「引脚写(13, 0)」= 亮
 */

#define BOARD_LED_PIN 13

static int arg_int(JmStateRef st, const JmValue *args, uint8_t nargs, int32_t *v,
                   const char *what) {
    if (nargs != 1) {
        return jm_native_fail(st, JM_NE_PARAM, what);
    }
    if (jm_value_to_int(args[0], v) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "参数必须是整数");
    }
    return 0;
}

static int n_pin_write(JmStateRef st, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    int32_t pin = 0;
    int32_t level = 0;
    if (nargs != 2) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚写需要两个参数：引脚, 电平");
    }
    if (jm_value_to_int(args[0], &pin) != 0 || jm_value_to_int(args[1], &level) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "引脚写：参数必须是整数");
    }
    if (pin != BOARD_LED_PIN) {
        return jm_native_fail(st, JM_NE_PARAM, "v0 只支持 PC13（引脚 13）");
    }
    if (level != 0 && level != 1) {
        return jm_native_fail(st, JM_NE_PARAM, "电平只能是 0 或 1");
    }
    port_gpio_write((uint32_t)pin, (int)level);
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_pin_read(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t pin = 0;
    if (arg_int(st, args, nargs, &pin, "引脚读需要一个参数：引脚") != 0) {
        return -1;
    }
    if (pin != BOARD_LED_PIN) {
        return jm_native_fail(st, JM_NE_PARAM, "v0 只支持 PC13（引脚 13）");
    }
    *out = jm_value_int(port_gpio_read((uint32_t)pin) ? 1 : 0);
    return JM_NATIVE_OK;
}

static int n_pin_toggle(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    int32_t pin = 0;
    if (arg_int(st, args, nargs, &pin, "引脚翻转需要一个参数：引脚") != 0) {
        return -1;
    }
    if (pin != BOARD_LED_PIN) {
        return jm_native_fail(st, JM_NE_PARAM, "v0 只支持 PC13（引脚 13）");
    }
    port_gpio_write((uint32_t)pin, port_gpio_read((uint32_t)pin) ? 0 : 1);
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_uart_write(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    const char *s = 0;
    uint32_t len = 0;
    if (nargs != 1) {
        return jm_native_fail(st, JM_NE_PARAM, "串口写需要一个参数：文本");
    }
    if (jm_value_text(args[0], &s, &len) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "串口写只接受文本");
    }
    jm_port_write(s, len);
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_delay(JmStateRef st, const JmValue *args, uint8_t nargs,
                   JmValue *out) {
    int32_t ms = 0;
    if (arg_int(st, args, nargs, &ms, "等待毫秒需要一个参数：毫秒") != 0) {
        return -1;
    }
    if (ms < 0 || ms > 60000) {
        return jm_native_fail(st, JM_NE_PARAM, "等待毫秒范围 0–60000");
    }
    port_delay_ms((uint32_t)ms);
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static const JmNativeEntry F103_TABLE[] = {
    {"引脚写",   n_pin_write, 2, 2, 0, 0},
    {"引脚读",   n_pin_read,  1, 1, 0, 0},
    {"引脚翻转", n_pin_toggle, 1, 1, 0, 0},
    {"串口写",   n_uart_write, 1, 1, 0, 0},
    {"等待毫秒", n_delay,     1, 1, 0, 0},
};

int f103_native_register(void) {
    return jm_native_register(F103_TABLE, (uint8_t)F103_NATIVE_ENTRY_COUNT);
}

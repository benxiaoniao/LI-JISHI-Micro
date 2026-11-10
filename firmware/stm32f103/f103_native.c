#include "jm_internal.h"
#include "jm_native.h"
#include "jm_port.h"
#include "stm32f103_port.h"
#include "f103_native.h"
#include "jmk.h"

/*
 * F103C8T6 板级原生函数表。
 * 约束（与 docs/原生函数表.md 一致）：
 *   - 只在任务/主上下文调用，不得进 ISR
 *   - 不分配、不保存借用指针
 *   - PC13 低电平点亮板载 LED，所以「引脚写(13, 0)」= 亮
 */

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
    if (pin < 0 || pin > 47) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "引脚编码 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15）");
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
    if (pin < 0 || pin > 47) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "引脚编码 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15）");
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
    if (pin < 0 || pin > 47) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "引脚编码 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15）");
    }
    (void)port_gpio_toggle((uint32_t)pin);
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

static int n_adc_init(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    (void)args;
    if (nargs != 0) {
        return jm_native_fail(st, JM_NE_PARAM, "ADC初始化不接受参数");
    }
    port_adc_init();
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_adc_read(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t channel = 0;
    uint32_t raw = 0;
    if (arg_int(st, args, nargs, &channel, "ADC读需要一个参数：通道") != 0) {
        return -1;
    }
    if (channel < 0 || channel > 7) {
        return jm_native_fail(st, JM_NE_PARAM, "ADC通道范围 0–7（PA0–PA7）");
    }
    if (port_adc_read((uint32_t)channel, &raw) != 0) {
        return jm_native_fail(st, JM_NE_STATE, "ADC 未初始化，先调用 ADC初始化()");
    }
    *out = jm_value_int((int32_t)raw);
    return JM_NATIVE_OK;
}

static int n_timer_init(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    (void)args;
    if (nargs != 0) {
        return jm_native_fail(st, JM_NE_PARAM, "定时器初始化不接受参数");
    }
    port_timer_init();
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_timer_delay(JmStateRef st, const JmValue *args, uint8_t nargs,
                         JmValue *out) {
    int32_t ms = 0;
    if (arg_int(st, args, nargs, &ms, "定时器延时需要一个参数：毫秒") != 0) {
        return -1;
    }
    if (ms < 0 || ms > 60000) {
        return jm_native_fail(st, JM_NE_PARAM, "定时器延时范围 0–60000");
    }
    if (port_timer_delay_ms((uint32_t)ms) != 0) {
        return jm_native_fail(st, JM_NE_STATE, "定时器未初始化，先调用 定时器初始化()");
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_timer_count(JmStateRef st, const JmValue *args, uint8_t nargs,
                         JmValue *out) {
    uint32_t ms = 0;
    (void)args;
    if (nargs != 0) {
        return jm_native_fail(st, JM_NE_PARAM, "定时器计数不接受参数");
    }
    if (port_timer_count(&ms) != 0) {
        return jm_native_fail(st, JM_NE_STATE, "定时器未初始化，先调用 定时器初始化()");
    }
    *out = jm_value_int((int32_t)ms);
    return JM_NATIVE_OK;
}

/* PWM 并入「定时器」表：TIM4，引脚 22–25 = PB6–PB9。PC13 没有定时器通道。 */
static int n_pwm_start(JmStateRef st, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    int32_t pin = 0;
    int32_t freq = 0;
    int32_t duty = 0;
    if (nargs != 3) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "PWM启动需要三个参数：引脚, 频率赫兹, 占空比千分比");
    }
    if (jm_value_to_int(args[0], &pin) != 0 ||
        jm_value_to_int(args[1], &freq) != 0 ||
        jm_value_to_int(args[2], &duty) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "PWM启动：参数必须是整数");
    }
    if (pin < 22 || pin > 25) {
        return jm_native_fail(st, JM_NE_PARAM, "PWM 引脚只能是 22–25（PB6–PB9）");
    }
    if (freq < 1 || freq > 10000) {
        return jm_native_fail(st, JM_NE_PARAM, "PWM 频率范围 1–10000 Hz");
    }
    if (duty < 0 || duty > 1000) {
        return jm_native_fail(st, JM_NE_PARAM, "PWM 占空比范围 0–1000（千分比）");
    }
    if (port_pwm_start((uint32_t)pin, (uint32_t)freq, (uint32_t)duty) != 0) {
        return jm_native_fail(st, JM_NE_HW, "PWM 启动失败");
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_pwm_set_duty(JmStateRef st, const JmValue *args, uint8_t nargs,
                          JmValue *out) {
    int32_t duty = 0;
    if (arg_int(st, args, nargs, &duty, "PWM占空比需要一个参数：千分比") != 0) {
        return -1;
    }
    if (duty < 0 || duty > 1000) {
        return jm_native_fail(st, JM_NE_PARAM, "PWM 占空比范围 0–1000（千分比）");
    }
    if (port_pwm_set_duty((uint32_t)duty) != 0) {
        return jm_native_fail(st, JM_NE_STATE, "PWM 未启动，先调用 PWM启动()");
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_pwm_stop(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    (void)args;
    if (nargs != 0) {
        return jm_native_fail(st, JM_NE_PARAM, "PWM停止不接受参数");
    }
    if (port_pwm_stop() != 0) {
        return jm_native_fail(st, JM_NE_STATE, "PWM 未启动，先调用 PWM启动()");
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

/* ---- 「总线」表：I2C（PB10/PB11 走硬件 I2C2，其它引脚走软件位翻转） ---- */

static uint8_t g_i2c_ready = 0u;

static int n_i2c_init(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t scl = 0;
    int32_t sda = 0;
    int32_t khz = 0;
    int rc;

    if (nargs != 3) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "I2C初始化需要三个参数：SCL引脚, SDA引脚, 速率千赫兹");
    }
    if (jm_value_to_int(args[0], &scl) != 0 ||
        jm_value_to_int(args[1], &sda) != 0 ||
        jm_value_to_int(args[2], &khz) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "I2C初始化：参数必须是整数");
    }
    if (scl < 0 || scl > 47 || sda < 0 || sda > 47) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "引脚编码 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15）");
    }
    if (scl == sda) {
        return jm_native_fail(st, JM_NE_PARAM, "SCL 与 SDA 不能是同一个引脚");
    }
    if (khz < 10 || khz > 400) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 速率范围 10–400 kHz");
    }

    rc = port_i2c_init((uint32_t)scl, (uint32_t)sda, (uint32_t)khz);
    if (rc < 0) {
        return jm_native_fail(st, JM_NE_HW, "I2C 初始化失败");
    }
    g_i2c_ready = 1u;
    *out = jm_value_bool(rc != 0);      /* 真 = 硬件 I2C2，假 = 软件位翻转 */
    return JM_NATIVE_OK;
}

static int n_i2c_arg_bytes(JmStateRef st, const JmValue *args, uint8_t first,
                           uint8_t nargs, uint32_t *buf, uint8_t *count) {
    uint8_t i;

    *count = 0u;
    for (i = first; i < nargs; i++) {
        int32_t v = 0;
        if (jm_value_to_int(args[i], &v) != 0) {
            return jm_native_fail(st, JM_NE_TYPE, "I2C 数据字节必须是整数");
        }
        if (v < 0 || v > 255) {
            return jm_native_fail(st, JM_NE_PARAM, "I2C 数据字节范围 0–255");
        }
        buf[*count] = (uint32_t)v;
        (*count)++;
    }
    return 0;
}

static int n_i2c_write(JmStateRef st, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    int32_t addr = 0;
    int32_t ctrl = 0;
    int32_t data_i = 0;
    uint32_t data;

    if (nargs != 3) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "I2C写需要三个参数：设备地址, 控制字节, 数据字节");
    }
    if (jm_value_to_int(args[0], &addr) != 0 ||
        jm_value_to_int(args[1], &ctrl) != 0 ||
        jm_value_to_int(args[2], &data_i) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "I2C写：参数必须是整数");
    }
    if (addr < 0 || addr > 127) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 设备地址范围 0–127");
    }
    if (ctrl < 0 || ctrl > 255 || data_i < 0 || data_i > 255) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 控制字节/数据字节范围 0–255");
    }
    if (g_i2c_ready == 0u) {
        return jm_native_fail(st, JM_NE_STATE, "I2C 未初始化，先调用 I2C初始化()");
    }
    data = (uint32_t)data_i;
    if (port_i2c_write((uint32_t)addr, (uint32_t)ctrl, &data, 1u) != 0) {
        return jm_native_fail(st, JM_NE_HW, "I2C 写失败（设备未应答或总线超时）");
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_i2c_write_block(JmStateRef st, const JmValue *args, uint8_t nargs,
                             JmValue *out) {
    int32_t addr = 0;
    int32_t ctrl_b = 0;
    uint32_t ctrl;
    uint32_t data[6];
    uint8_t count = 0u;
    int rc;

    if (nargs < 3 || nargs > 8) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "I2C写块需要 3–8 个参数：设备地址, 控制字节, 数据1[, 数据2 … 数据5]");
    }
    if (jm_value_to_int(args[0], &addr) != 0 ||
        jm_value_to_int(args[1], &ctrl_b) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "I2C写块：地址与控制字节必须是整数");
    }
    if (addr < 0 || addr > 127) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 设备地址范围 0–127");
    }
    if (ctrl_b < 0 || ctrl_b > 255) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 控制字节范围 0–255");
    }
    rc = n_i2c_arg_bytes(st, args, 2u, nargs, data, &count);
    if (rc != 0) {
        return -1;
    }
    if (g_i2c_ready == 0u) {
        return jm_native_fail(st, JM_NE_STATE, "I2C 未初始化，先调用 I2C初始化()");
    }
    ctrl = (uint32_t)ctrl_b;
    if (port_i2c_write((uint32_t)addr, ctrl, data, (uint32_t)count) != 0) {
        return jm_native_fail(st, JM_NE_HW, "I2C 写块失败（设备未应答或总线超时）");
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_i2c_probe(JmStateRef st, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    int32_t addr = 0;
    int rc;

    if (arg_int(st, args, nargs, &addr, "I2C探测需要一个参数：设备地址") != 0) {
        return -1;
    }
    if (addr < 0 || addr > 127) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 设备地址范围 0–127");
    }
    if (g_i2c_ready == 0u) {
        return jm_native_fail(st, JM_NE_STATE, "I2C 未初始化，先调用 I2C初始化()");
    }
    rc = port_i2c_probe((uint32_t)addr);
    if (rc < 0) {
        return jm_native_fail(st, JM_NE_HW, "I2C 探测失败");
    }
    *out = jm_value_bool(rc != 0);
    return JM_NATIVE_OK;
}

/* ---- 「总线」表续：SPI（PA5/PA6/PA7 走硬件 SPI1，其它引脚走软件位翻转） ---- */

static uint8_t g_spi_ready = 0u;

static int n_spi_init(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t sck = 0;
    int32_t miso = 0;
    int32_t mosi = 0;
    int32_t div = 0;
    int rc;

    if (nargs != 4) {
        return jm_native_fail(
            st, JM_NE_PARAM,
            "SPI初始化需要四个参数：SCK引脚, MISO引脚, MOSI引脚, 分频");
    }
    if (jm_value_to_int(args[0], &sck) != 0 ||
        jm_value_to_int(args[1], &miso) != 0 ||
        jm_value_to_int(args[2], &mosi) != 0 ||
        jm_value_to_int(args[3], &div) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "SPI初始化：参数必须是整数");
    }
    if (sck < 0 || sck > 47 || miso < 0 || miso > 47 || mosi < 0 || mosi > 47) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "引脚编码 0–47（0–15=PA0–PA15，16–31=PB0–PB15，32–47=PC0–PC15）");
    }
    if (sck == miso || sck == mosi || miso == mosi) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "SCK / MISO / MOSI 三个引脚不能重复");
    }
    if (div != 2 && div != 4 && div != 8 && div != 16 && div != 32 &&
        div != 64 && div != 128 && div != 256) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "SPI 分频只能是 2/4/8/16/32/64/128/256");
    }

    rc = port_spi_init((uint32_t)sck, (uint32_t)miso, (uint32_t)mosi,
                       (uint32_t)div);
    if (rc < 0) {
        return jm_native_fail(st, JM_NE_HW, "SPI 初始化失败");
    }
    g_spi_ready = 1u;
    *out = jm_value_bool(rc != 0);      /* 真 = 硬件 SPI1，假 = 软件位翻转 */
    return JM_NATIVE_OK;
}

static int n_spi_transfer(JmStateRef st, const JmValue *args, uint8_t nargs,
                          JmValue *out) {
    int32_t byte = 0;
    uint32_t got = 0u;

    if (arg_int(st, args, nargs, &byte, "SPI收发需要一个参数：数据字节") != 0) {
        return -1;
    }
    if (byte < 0 || byte > 255) {
        return jm_native_fail(st, JM_NE_PARAM, "SPI 数据字节范围 0–255");
    }
    if (g_spi_ready == 0u) {
        return jm_native_fail(st, JM_NE_STATE, "SPI 未初始化，先调用 SPI初始化()");
    }
    if (port_spi_transfer((uint32_t)byte, &got) != 0) {
        return jm_native_fail(st, JM_NE_HW, "SPI 收发失败（总线超时）");
    }
    *out = jm_value_int((int32_t)got);
    return JM_NATIVE_OK;
}

static int n_spi_write_block(JmStateRef st, const JmValue *args, uint8_t nargs,
                             JmValue *out) {
    uint32_t data[5];
    uint8_t i;

    if (nargs < 1 || nargs > 5) {
        return jm_native_fail(
            st, JM_NE_PARAM,
            "SPI写块需要 1–5 个参数：数据1[, 数据2 … 数据5]");
    }
    for (i = 0u; i < nargs; i++) {
        int32_t v = 0;
        if (jm_value_to_int(args[i], &v) != 0) {
            return jm_native_fail(st, JM_NE_TYPE, "SPI 数据字节必须是整数");
        }
        if (v < 0 || v > 255) {
            return jm_native_fail(st, JM_NE_PARAM, "SPI 数据字节范围 0–255");
        }
        data[i] = (uint32_t)v;
    }
    if (g_spi_ready == 0u) {
        return jm_native_fail(st, JM_NE_STATE, "SPI 未初始化，先调用 SPI初始化()");
    }
    if (port_spi_write(data, (uint32_t)nargs) != 0) {
        return jm_native_fail(st, JM_NE_HW, "SPI 写块失败（总线超时）");
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

/* 语言层「任务」不再走原生表：创建/让出/延时/等待由 VM 绿色调度器实现。 */

/* ---- 语言层任务间通信（JM8 续）：整数句柄 → 静态 JMK 对象槽 ----
 * 句柄是「槽位号 + 1」，0 与越界都算无效。对象在启动后第一次创建时分配，
 * 不释放（.jsh 没有销毁入口）；同一份 .jsh 在主机端口也用同样的句柄编号。
 */
#define JM_IPC_SLOTS       3u
#define JM_IPC_QUEUE_DEPTH 8u

static JmkQueue g_ipc_queue[JM_IPC_SLOTS];
static uint32_t g_ipc_queue_buf[JM_IPC_SLOTS][JM_IPC_QUEUE_DEPTH];
static uint8_t g_ipc_queue_used[JM_IPC_SLOTS];

static JmkSem g_ipc_sem[JM_IPC_SLOTS];
static uint8_t g_ipc_sem_used[JM_IPC_SLOTS];

static JmkQueue *queue_of(int32_t handle) {
    if (handle < 1 || handle > (int32_t)JM_IPC_SLOTS) {
        return 0;
    }
    if (!g_ipc_queue_used[handle - 1]) {
        return 0;
    }
    return &g_ipc_queue[handle - 1];
}

static JmkSem *sem_of(int32_t handle) {
    if (handle < 1 || handle > (int32_t)JM_IPC_SLOTS) {
        return 0;
    }
    if (!g_ipc_sem_used[handle - 1]) {
        return 0;
    }
    return &g_ipc_sem[handle - 1];
}

/* 可选超时参数：缺省 0（不等待），范围 0–60000 毫秒。 */
static int arg_timeout(JmStateRef st, const JmValue *args, uint8_t nargs,
                       uint8_t index, uint32_t *out) {
    int32_t ms = 0;
    *out = JMK_NO_WAIT;
    if (nargs <= index) {
        return 0;
    }
    if (jm_value_to_int(args[index], &ms) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "超时参数必须是整数");
    }
    if (ms < 0 || ms > 60000) {
        return jm_native_fail(st, JM_NE_PARAM, "超时范围 0–60000 毫秒");
    }
    *out = (uint32_t)ms;
    return 0;
}

/* JMK 的失败码 → 原生错误：E_STATE 只有一种成因（不在任务上下文），单独说清。 */
static int ipc_fail(JmStateRef st, int rc) {
    if (rc == JMK_E_STATE) {
        return jm_native_fail(st, JM_NE_STATE, "只能在 JMK 任务上下文调用");
    }
    return jm_native_fail(st, JM_NE_HW, "JMK 对象操作失败");
}

static int n_queue_create(JmStateRef st, const JmValue *args, uint8_t nargs,
                          JmValue *out) {
    int32_t cap = 0;
    uint8_t i;
    if (arg_int(st, args, nargs, &cap, "队列创建需要一个参数：槽数") != 0) {
        return -1;
    }
    if (cap < 1 || cap > (int32_t)JM_IPC_QUEUE_DEPTH) {
        return jm_native_fail(st, JM_NE_PARAM, "槽数范围 1–8");
    }
    for (i = 0; i < JM_IPC_SLOTS; i++) {
        if (g_ipc_queue_used[i]) {
            continue;
        }
        if (jmk_queue_init(&g_ipc_queue[i], g_ipc_queue_buf[i],
                           sizeof(uint32_t), (uint32_t)cap) != JMK_OK) {
            return jm_native_fail(st, JM_NE_STATE, "队列初始化失败");
        }
        g_ipc_queue_used[i] = 1u;
        *out = jm_value_int((int32_t)i + 1);
        return JM_NATIVE_OK;
    }
    return jm_native_fail(st, JM_NE_NOMEM, "队列对象已用完（最多 3 个）");
}

static int n_queue_send(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    int32_t handle = 0;
    int32_t value = 0;
    uint32_t timeout = JMK_NO_WAIT;
    JmkQueue *q;
    int rc;
    if (jm_value_to_int(args[0], &handle) != 0 ||
        jm_value_to_int(args[1], &value) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "队列发送：句柄与值必须是整数");
    }
    if (arg_timeout(st, args, nargs, 2, &timeout) != 0) {
        return -1;
    }
    q = queue_of(handle);
    if (!q) {
        return jm_native_fail(st, JM_NE_PARAM, "队列句柄无效");
    }
    rc = jmk_queue_send(q, &value, timeout);
    if (rc == JMK_OK) {
        *out = jm_value_bool(1);
        return JM_NATIVE_OK;
    }
    if (rc == JMK_E_TIMEOUT) {
        *out = jm_value_bool(0);
        return JM_NATIVE_OK;
    }
    return ipc_fail(st, rc);
}

static int n_queue_recv(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    int32_t handle = 0;
    uint32_t value = 0u;
    uint32_t timeout = JMK_NO_WAIT;
    JmkQueue *q;
    int rc;
    if (jm_value_to_int(args[0], &handle) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "队列接收：句柄必须是整数");
    }
    if (arg_timeout(st, args, nargs, 1, &timeout) != 0) {
        return -1;
    }
    q = queue_of(handle);
    if (!q) {
        return jm_native_fail(st, JM_NE_PARAM, "队列句柄无效");
    }
    rc = jmk_queue_recv(q, &value, timeout);
    if (rc == JMK_OK) {
        *out = jm_value_int((int32_t)value);
        return JM_NATIVE_OK;
    }
    if (rc == JMK_E_TIMEOUT) {
        *out = jm_value_nil();
        return JM_NATIVE_OK;
    }
    return ipc_fail(st, rc);
}

static int n_queue_count(JmStateRef st, const JmValue *args, uint8_t nargs,
                         JmValue *out) {
    int32_t handle = 0;
    JmkQueue *q;
    if (arg_int(st, args, nargs, &handle, "队列数量需要一个参数：句柄") != 0) {
        return -1;
    }
    q = queue_of(handle);
    if (!q) {
        return jm_native_fail(st, JM_NE_PARAM, "队列句柄无效");
    }
    *out = jm_value_int((int32_t)jmk_queue_count(q));
    return JM_NATIVE_OK;
}

static int n_queue_destroy(JmStateRef st, const JmValue *args, uint8_t nargs,
                           JmValue *out) {
    int32_t handle = 0;
    JmkQueue *q;
    (void)st;
    if (nargs != 1 || jm_value_to_int(args[0], &handle) != 0) {
        *out = jm_value_bool(0);
        return JM_NATIVE_OK;
    }
    q = queue_of(handle);
    if (!q) {
        *out = jm_value_bool(0);
        return JM_NATIVE_OK;
    }
    if (jmk_queue_destroy(q) != JMK_OK) {
        *out = jm_value_bool(0);
        return JM_NATIVE_OK;
    }
    g_ipc_queue_used[handle - 1] = 0u;
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static int n_sem_create(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    int32_t initial = 0;
    int32_t max = 1;
    uint8_t i;
    if (jm_value_to_int(args[0], &initial) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "信号量创建：初值必须是整数");
    }
    if (nargs > 1 && jm_value_to_int(args[1], &max) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "信号量创建：上限必须是整数");
    }
    if (initial < 0 || initial > 255 || max < 1 || max > 255) {
        return jm_native_fail(st, JM_NE_PARAM, "初值 0–255，上限 1–255");
    }
    if (initial > max) {
        return jm_native_fail(st, JM_NE_PARAM, "初值不能大于上限");
    }
    for (i = 0; i < JM_IPC_SLOTS; i++) {
        if (g_ipc_sem_used[i]) {
            continue;
        }
        if (jmk_sem_init(&g_ipc_sem[i], initial, max) != JMK_OK) {
            return jm_native_fail(st, JM_NE_STATE, "信号量初始化失败");
        }
        g_ipc_sem_used[i] = 1u;
        *out = jm_value_int((int32_t)i + 1);
        return JM_NATIVE_OK;
    }
    return jm_native_fail(st, JM_NE_NOMEM, "信号量对象已用完（最多 3 个）");
}

static int n_sem_wait(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t handle = 0;
    uint32_t timeout = JMK_NO_WAIT;
    JmkSem *s;
    int rc;
    if (jm_value_to_int(args[0], &handle) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "信号量等待：句柄必须是整数");
    }
    if (arg_timeout(st, args, nargs, 1, &timeout) != 0) {
        return -1;
    }
    s = sem_of(handle);
    if (!s) {
        return jm_native_fail(st, JM_NE_PARAM, "信号量句柄无效");
    }
    rc = jmk_sem_wait(s, timeout);
    if (rc == JMK_OK) {
        *out = jm_value_bool(1);
        return JM_NATIVE_OK;
    }
    if (rc == JMK_E_TIMEOUT) {
        *out = jm_value_bool(0);
        return JM_NATIVE_OK;
    }
    return ipc_fail(st, rc);
}

static int n_sem_post(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t handle = 0;
    JmkSem *s;
    int rc;
    if (arg_int(st, args, nargs, &handle, "信号量给出需要一个参数：句柄") != 0) {
        return -1;
    }
    s = sem_of(handle);
    if (!s) {
        return jm_native_fail(st, JM_NE_PARAM, "信号量句柄无效");
    }
    rc = jmk_sem_post(s);
    if (rc == JMK_OK) {
        *out = jm_value_bool(1);
        return JM_NATIVE_OK;
    }
    if (rc == JMK_E_FULL) {
        *out = jm_value_bool(0);
        return JM_NATIVE_OK;
    }
    return ipc_fail(st, rc);
}

/* ---- 语言层中断（JM7）：中断只投递，不跑字节码 ----
 * 数据路径：EXTI_IRQHandler（port 层）→ irq_post_from_isr → jmk_queue_send_from_isr
 *          → 任务里的 队列接收 → .jsh 逻辑。
 * 中断上下文里既不进 VM，也不调用任何 native 函数；ABI v1 的「ISR 不得直调 native」
 * 因此仍然成立。
 */
#define JM_IRQ_LINES 16u   /* EXTI 线号 0–15 */
#define JM_IRQ_PINS  48u   /* 编码引脚：0–15 = PA，16–31 = PB，32–47 = PC */

static JmkQueue *g_irq_queue[JM_IRQ_LINES];
static uint8_t g_irq_pin[JM_IRQ_LINES];   /* 该线当前绑定的编码引脚 + 1；0 = 未绑定 */

/* 由 port 层的中断服务函数调用：只做计数与入队，不做别的事。 */
static void irq_post_from_isr(uint32_t line)
{
    uint32_t payload;

    if (line >= JM_IRQ_LINES || g_irq_queue[line] == 0) {
        return;
    }
    payload = (uint32_t)g_irq_pin[line] - 1u;
    (void)jmk_queue_send_from_isr(g_irq_queue[line], &payload, 0);
}

static int n_irq_bind(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t pin = 0;
    int32_t handle = 0;
    int32_t edge = 0;
    uint32_t line;
    JmkQueue *q;

    if (nargs < 2) {
        return jm_native_fail(st, JM_NE_PARAM, "中断绑定需要：引脚, 队列句柄[, 边沿]");
    }
    if (jm_value_to_int(args[0], &pin) != 0 || jm_value_to_int(args[1], &handle) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "中断绑定：引脚与队列句柄必须是整数");
    }
    if (nargs > 2 && jm_value_to_int(args[2], &edge) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "中断绑定：边沿必须是整数");
    }
    if (pin < 0 || pin >= (int32_t)JM_IRQ_PINS) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–47（0–15=PA，16–31=PB，32–47=PC）");
    }
    if (edge < 0 || edge > 2) {
        return jm_native_fail(st, JM_NE_PARAM, "边沿：0=上升 1=下降 2=双沿");
    }
    q = queue_of(handle);
    if (!q) {
        return jm_native_fail(st, JM_NE_PARAM, "队列句柄无效");
    }
    line = (uint32_t)pin % JM_IRQ_LINES;
    g_irq_queue[line] = q;
    g_irq_pin[line] = (uint8_t)(pin + 1u);
    port_exti_bind((uint32_t)pin, (uint32_t)edge);
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static int n_irq_trigger(JmStateRef st, const JmValue *args, uint8_t nargs,
                         JmValue *out) {
    int32_t pin = 0;
    uint32_t line;

    if (arg_int(st, args, nargs, &pin, "中断触发需要一个参数：引脚") != 0) {
        return -1;
    }
    if (pin < 0 || pin >= (int32_t)JM_IRQ_PINS) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–47（0–15=PA，16–31=PB，32–47=PC）");
    }
    line = (uint32_t)pin % JM_IRQ_LINES;
    if (g_irq_queue[line] == 0) {
        *out = jm_value_bool(0);
        return JM_NATIVE_OK;
    }
    port_exti_sw_trigger((uint32_t)pin);
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static int n_irq_count(JmStateRef st, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    int32_t pin = 0;

    if (arg_int(st, args, nargs, &pin, "中断计数需要一个参数：引脚") != 0) {
        return -1;
    }
    if (pin < 0 || pin >= (int32_t)JM_IRQ_PINS) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–47（0–15=PA，16–31=PB，32–47=PC）");
    }
    *out = jm_value_int((int32_t)port_exti_count((uint32_t)pin));
    return JM_NATIVE_OK;
}

static int n_irq_unbind(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    int32_t pin = 0;
    uint32_t line;

    if (arg_int(st, args, nargs, &pin, "中断解除需要一个参数：引脚") != 0) {
        return -1;
    }
    if (pin < 0 || pin >= (int32_t)JM_IRQ_PINS) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–47（0–15=PA，16–31=PB，32–47=PC）");
    }
    line = (uint32_t)pin % JM_IRQ_LINES;
    if (g_irq_pin[line] == (uint8_t)(pin + 1u)) {
        g_irq_queue[line] = 0;
        g_irq_pin[line] = 0u;
    }
    port_exti_unbind((uint32_t)pin);
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

/* 表项由 micc --emit-native-header 生成，见 runtime/include/jm_native_table.h */
static const JmNativeEntry F103_TABLE[] = {
    F103_NATIVE_TABLE_INIT
};

static const JmNativeEntry ADC_TABLE[] = {
    F103_ADC_TABLE_INIT
};

static const JmNativeEntry TIMER_TABLE[] = {
    F103_TIMER_TABLE_INIT
};

static const JmNativeEntry QUEUE_TABLE[] = {
    JM_QUEUE_TABLE_INIT
};

static const JmNativeEntry SEM_TABLE[] = {
    JM_SEM_TABLE_INIT
};

static const JmNativeEntry IRQ_TABLE[] = {
    JM_IRQ_TABLE_INIT
};

static const JmNativeEntry BUS_TABLE[] = {
    F103_BUS_TABLE_INIT
};

int f103_native_register(void) {
    /* 中断里只允许走这条回调；它只做 jmk_queue_send_from_isr，不进 VM。 */
    port_exti_set_hook(irq_post_from_isr);
    if (jm_native_register(F103_TABLE, (uint8_t)F103_NATIVE_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(ADC_TABLE, (uint8_t)F103_ADC_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(TIMER_TABLE, (uint8_t)F103_TIMER_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(QUEUE_TABLE, (uint8_t)JM_QUEUE_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(SEM_TABLE, (uint8_t)JM_SEM_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(IRQ_TABLE, (uint8_t)JM_IRQ_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(BUS_TABLE, (uint8_t)F103_BUS_ENTRY_COUNT) != 0) {
        return -1;
    }
    return 0;
}

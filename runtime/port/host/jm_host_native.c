#include <stdint.h>
#include <stdio.h>
#include "jm_native.h"
#include "jm_native_table.h"
#include "jm_port.h"

/*
 * 主机端「队列」「信号量」原生表：与板级同一份 ABI（jm_native_table.h 由 native_spec.rs 生成）。
 * 语言层的「任务」不再是原生表：创建/让出/延时/等待都在 VM 绿色调度器里完成，
 * 所以这里没有任务表，也不要主机自己的 yield/sleep 实现（统一走 jm_port_*）。
 */

/* ---- 主机端「队列」「信号量」：与板级同句柄编号、同可见语义 ----
 * 主机只有一个执行流，没有别的任务能在这期间发送/给出，所以“等不到”时
 * 睡满超时再返回 假 / 空；超时为 0 则立即返回。同一份 .jsh 的输出因此与
 * 板上逐字一致（板上若真有对端任务，也只是更快拿到数据）。
 */
#define HOST_IPC_SLOTS       3u
#define HOST_QUEUE_DEPTH     8u

typedef struct {
    int32_t buf[HOST_QUEUE_DEPTH];
    uint32_t cap;
    uint32_t head;
    uint32_t tail;
    uint32_t count;
    uint8_t used;
} HostQueue;

typedef struct {
    int32_t count;
    int32_t max;
    uint8_t used;
} HostSem;

static HostQueue g_host_queue[HOST_IPC_SLOTS];
static HostSem g_host_sem[HOST_IPC_SLOTS];

static HostQueue *host_queue_of(int32_t handle) {
    if (handle < 1 || handle > (int32_t)HOST_IPC_SLOTS) {
        return 0;
    }
    if (!g_host_queue[handle - 1].used) {
        return 0;
    }
    return &g_host_queue[handle - 1];
}

static HostSem *host_sem_of(int32_t handle) {
    if (handle < 1 || handle > (int32_t)HOST_IPC_SLOTS) {
        return 0;
    }
    if (!g_host_sem[handle - 1].used) {
        return 0;
    }
    return &g_host_sem[handle - 1];
}

static int arg_timeout(JmStateRef st, const JmValue *args, uint8_t nargs,
                            uint8_t index, uint32_t *out) {
    int32_t ms = 0;
    *out = 0u;
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

static int n_queue_create(JmStateRef st, const JmValue *args, uint8_t nargs,
                               JmValue *out) {
    int32_t cap = 0;
    uint8_t i;
    if (nargs != 1) {
        return jm_native_fail(st, JM_NE_PARAM, "队列创建需要一个参数：槽数");
    }
    if (jm_value_to_int(args[0], &cap) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "队列创建：槽数必须是整数");
    }
    if (cap < 1 || cap > (int32_t)HOST_QUEUE_DEPTH) {
        return jm_native_fail(st, JM_NE_PARAM, "槽数范围 1–8");
    }
    for (i = 0; i < HOST_IPC_SLOTS; i++) {
        if (g_host_queue[i].used) {
            continue;
        }
        g_host_queue[i].cap = (uint32_t)cap;
        g_host_queue[i].head = 0u;
        g_host_queue[i].tail = 0u;
        g_host_queue[i].count = 0u;
        g_host_queue[i].used = 1u;
        *out = jm_value_int((int32_t)i + 1);
        return JM_NATIVE_OK;
    }
    return jm_native_fail(st, JM_NE_NOMEM, "队列对象已用完（最多 3 个）");
}

static int n_queue_send(JmStateRef st, const JmValue *args, uint8_t nargs,
                             JmValue *out) {
    int32_t handle = 0;
    int32_t value = 0;
    uint32_t timeout = 0u;
    HostQueue *q;
    if (nargs < 2 || jm_value_to_int(args[0], &handle) != 0 ||
        jm_value_to_int(args[1], &value) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "队列发送：句柄与值必须是整数");
    }
    if (arg_timeout(st, args, nargs, 2, &timeout) != 0) {
        return -1;
    }
    q = host_queue_of(handle);
    if (!q) {
        return jm_native_fail(st, JM_NE_PARAM, "队列句柄无效");
    }
    if (q->count < q->cap) {
        q->buf[q->tail] = value;
        q->tail = (q->tail + 1u) % q->cap;
        q->count++;
        *out = jm_value_bool(1);
        return JM_NATIVE_OK;
    }
    if (timeout > 0u) {
        jm_port_sleep_ms(timeout);
    }
    *out = jm_value_bool(0);
    return JM_NATIVE_OK;
}

static int n_queue_recv(JmStateRef st, const JmValue *args, uint8_t nargs,
                             JmValue *out) {
    int32_t handle = 0;
    uint32_t timeout = 0u;
    HostQueue *q;
    if (nargs < 1 || jm_value_to_int(args[0], &handle) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "队列接收：句柄必须是整数");
    }
    if (arg_timeout(st, args, nargs, 1, &timeout) != 0) {
        return -1;
    }
    q = host_queue_of(handle);
    if (!q) {
        return jm_native_fail(st, JM_NE_PARAM, "队列句柄无效");
    }
    if (q->count > 0u) {
        *out = jm_value_int(q->buf[q->head]);
        q->head = (q->head + 1u) % q->cap;
        q->count--;
        return JM_NATIVE_OK;
    }
    if (timeout > 0u) {
        jm_port_sleep_ms(timeout);
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_queue_count(JmStateRef st, const JmValue *args, uint8_t nargs,
                              JmValue *out) {
    int32_t handle = 0;
    HostQueue *q;
    if (nargs != 1 || jm_value_to_int(args[0], &handle) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "队列数量需要一个参数：句柄");
    }
    q = host_queue_of(handle);
    if (!q) {
        return jm_native_fail(st, JM_NE_PARAM, "队列句柄无效");
    }
    *out = jm_value_int((int32_t)q->count);
    return JM_NATIVE_OK;
}

static int n_sem_create(JmStateRef st, const JmValue *args, uint8_t nargs,
                             JmValue *out) {
    int32_t initial = 0;
    int32_t max = 1;
    uint8_t i;
    if (nargs < 1 || jm_value_to_int(args[0], &initial) != 0) {
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
    for (i = 0; i < HOST_IPC_SLOTS; i++) {
        if (g_host_sem[i].used) {
            continue;
        }
        g_host_sem[i].count = initial;
        g_host_sem[i].max = max;
        g_host_sem[i].used = 1u;
        *out = jm_value_int((int32_t)i + 1);
        return JM_NATIVE_OK;
    }
    return jm_native_fail(st, JM_NE_NOMEM, "信号量对象已用完（最多 3 个）");
}

static int n_sem_wait(JmStateRef st, const JmValue *args, uint8_t nargs,
                           JmValue *out) {
    int32_t handle = 0;
    uint32_t timeout = 0u;
    HostSem *s;
    if (nargs < 1 || jm_value_to_int(args[0], &handle) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "信号量等待：句柄必须是整数");
    }
    if (arg_timeout(st, args, nargs, 1, &timeout) != 0) {
        return -1;
    }
    s = host_sem_of(handle);
    if (!s) {
        return jm_native_fail(st, JM_NE_PARAM, "信号量句柄无效");
    }
    if (s->count > 0) {
        s->count--;
        *out = jm_value_bool(1);
        return JM_NATIVE_OK;
    }
    if (timeout > 0u) {
        jm_port_sleep_ms(timeout);
    }
    *out = jm_value_bool(0);
    return JM_NATIVE_OK;
}

static int n_sem_post(JmStateRef st, const JmValue *args, uint8_t nargs,
                           JmValue *out) {
    int32_t handle = 0;
    HostSem *s;
    if (nargs != 1 || jm_value_to_int(args[0], &handle) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "信号量给出需要一个参数：句柄");
    }
    s = host_sem_of(handle);
    if (!s) {
        return jm_native_fail(st, JM_NE_PARAM, "信号量句柄无效");
    }
    if (s->count < s->max) {
        s->count++;
        *out = jm_value_bool(1);
        return JM_NATIVE_OK;
    }
    *out = jm_value_bool(0);
    return JM_NATIVE_OK;
}

static const JmNativeEntry HOST_QUEUE_TABLE[] = {
    JM_QUEUE_TABLE_INIT
};

/* ---- 主机端「中断」：与板级同名同语义 ----
 * PC 上没有 EXTI，中断触发() 直接模拟“中断把引脚号投进队列”这一步；
 * 板上的真实路径是 EXTI_IRQHandler → jmk_queue_send_from_isr → 同一队列。
 * 两边可见结果一致：投递顺序、计数、解除后不再触发。
 */
/* 编码引脚 0–15 = PA，16–31 = PB，32–47 = PC；与板上 EXTI 线号 = 引脚 % 16 一致 */
#define HOST_IRQ_LINES 48u

static HostQueue *g_host_irq_queue[HOST_IRQ_LINES];
static int32_t g_host_irq_count[HOST_IRQ_LINES];

static int n_irq_bind(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t pin = 0;
    int32_t handle = 0;
    int32_t edge = 0;
    HostQueue *q;
    if (nargs < 2) {
        return jm_native_fail(st, JM_NE_PARAM, "中断绑定需要：引脚, 队列句柄[, 边沿]");
    }
    if (jm_value_to_int(args[0], &pin) != 0 || jm_value_to_int(args[1], &handle) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "中断绑定：引脚与队列句柄必须是整数");
    }
    if (nargs > 2 && jm_value_to_int(args[2], &edge) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "中断绑定：边沿必须是整数");
    }
    if (pin < 0 || pin >= (int32_t)HOST_IRQ_LINES) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–47（0–15=PA，16–31=PB，32–47=PC）");
    }
    if (edge < 0 || edge > 2) {
        return jm_native_fail(st, JM_NE_PARAM, "边沿：0=上升 1=下降 2=双沿");
    }
    q = host_queue_of(handle);
    if (!q) {
        return jm_native_fail(st, JM_NE_PARAM, "队列句柄无效");
    }
    g_host_irq_queue[pin] = q;
    g_host_irq_count[pin] = 0;
    {
        /* 同一条 EXTI 线只能选一个端口，与板上 AFIO 换线语义一致 */
        uint32_t line = (uint32_t)pin % 16u;
        uint32_t i;
        for (i = line; i < HOST_IRQ_LINES; i += 16u) {
            if (i != (uint32_t)pin) {
                g_host_irq_queue[i] = 0;
            }
        }
    }
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static int n_irq_trigger(JmStateRef st, const JmValue *args, uint8_t nargs,
                         JmValue *out) {
    int32_t pin = 0;
    HostQueue *q;
    if (nargs != 1) {
        return jm_native_fail(st, JM_NE_PARAM, "中断触发需要一个参数：引脚");
    }
    if (jm_value_to_int(args[0], &pin) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "中断触发：引脚必须是整数");
    }
    if (pin < 0 || pin >= (int32_t)HOST_IRQ_LINES) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–47（0–15=PA，16–31=PB，32–47=PC）");
    }
    q = g_host_irq_queue[pin];
    if (!q) {
        *out = jm_value_bool(0);
        return JM_NATIVE_OK;
    }
    g_host_irq_count[pin]++;
    if (q->count < q->cap) {
        q->buf[q->tail] = pin;
        q->tail = (q->tail + 1u) % q->cap;
        q->count++;
    }
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static int n_irq_count(JmStateRef st, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    int32_t pin = 0;
    if (nargs != 1) {
        return jm_native_fail(st, JM_NE_PARAM, "中断计数需要一个参数：引脚");
    }
    if (jm_value_to_int(args[0], &pin) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "中断计数：引脚必须是整数");
    }
    if (pin < 0 || pin >= (int32_t)HOST_IRQ_LINES) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–47（0–15=PA，16–31=PB，32–47=PC）");
    }
    *out = jm_value_int(g_host_irq_count[pin]);
    return JM_NATIVE_OK;
}

static int n_irq_unbind(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    int32_t pin = 0;
    if (nargs != 1) {
        return jm_native_fail(st, JM_NE_PARAM, "中断解除需要一个参数：引脚");
    }
    if (jm_value_to_int(args[0], &pin) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "中断解除：引脚必须是整数");
    }
    if (pin < 0 || pin >= (int32_t)HOST_IRQ_LINES) {
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–47（0–15=PA，16–31=PB，32–47=PC）");
    }
    g_host_irq_queue[pin] = 0;
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static const JmNativeEntry HOST_SEM_TABLE[] = {
    JM_SEM_TABLE_INIT
};

static const JmNativeEntry HOST_IRQ_TABLE[] = {
    JM_IRQ_TABLE_INIT
};

/* ---- 主机端 ADC / 定时器替身 ----
 * PC 上没有 ADC 和 TIM2，但同一份 .jsh 要在主机上回归。这里给的是**确定性替身**：
 * ADC读(0) 按固定锯齿波返回，定时器计数 用 jm_port_millis 换算成 16 位毫秒。
 * 板级 f103_native.c 里的同名函数才是真的寄存器驱动；两边名字与参数个数一致。
 */
#define HOST_ADC_STEPS 8u
static const int32_t g_host_adc_wave[HOST_ADC_STEPS] = {
    0, 1024, 2048, 3072, 4095, 3072, 2048, 1024
};
static uint32_t g_host_adc_pos;
static uint32_t g_host_timer_base;

static int n_adc_init(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    (void)st;
    (void)args;
    (void)nargs;
    g_host_adc_pos = 0u;
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static int n_adc_read(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t channel = 0;
    (void)nargs;
    if (jm_value_to_int(args[0], &channel) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "ADC读：通道必须是整数");
    }
    if (channel < 0 || channel > 7) {
        return jm_native_fail(st, JM_NE_PARAM, "通道范围 0–7");
    }
    *out = jm_value_int(g_host_adc_wave[g_host_adc_pos % HOST_ADC_STEPS]);
    g_host_adc_pos++;
    return JM_NATIVE_OK;
}

static int n_timer_init(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    (void)st;
    (void)args;
    (void)nargs;
    g_host_timer_base = jm_port_millis();
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static int n_timer_delay(JmStateRef st, const JmValue *args, uint8_t nargs,
                         JmValue *out) {
    int32_t ms = 0;
    (void)nargs;
    if (jm_value_to_int(args[0], &ms) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "定时器延时：毫秒必须是整数");
    }
    if (ms < 0 || ms > 60000) {
        return jm_native_fail(st, JM_NE_PARAM, "延时范围 0–60000 毫秒");
    }
    jm_port_sleep_ms((uint32_t)ms);
    *out = jm_value_bool(1);
    return JM_NATIVE_OK;
}

static int n_timer_count(JmStateRef st, const JmValue *args, uint8_t nargs,
                         JmValue *out) {
    (void)st;
    (void)args;
    (void)nargs;
    *out = jm_value_int((int32_t)(uint16_t)(jm_port_millis() - g_host_timer_base));
    return JM_NATIVE_OK;
}

/* ---- 主机端 PWM 替身 ----
 * PC 上没有 TIM4 与引脚输出，这里只保存状态并做与板级相同的参数校验；
 * 参数越界、未启动就改占空比/停止，都在主机上以同一套错误码暴露。
 */
static uint8_t g_host_pwm_on = 0u;

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
    g_host_pwm_on = 1u;
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_pwm_set_duty(JmStateRef st, const JmValue *args, uint8_t nargs,
                          JmValue *out) {
    int32_t duty = 0;
    if (nargs != 1 || jm_value_to_int(args[0], &duty) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "PWM占空比需要一个整数参数：千分比");
    }
    if (duty < 0 || duty > 1000) {
        return jm_native_fail(st, JM_NE_PARAM, "PWM 占空比范围 0–1000（千分比）");
    }
    if (g_host_pwm_on == 0u) {
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
    if (g_host_pwm_on == 0u) {
        return jm_native_fail(st, JM_NE_STATE, "PWM 未启动，先调用 PWM启动()");
    }
    g_host_pwm_on = 0u;
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static const JmNativeEntry HOST_ADC_TABLE[] = {
    F103_ADC_TABLE_INIT
};

static const JmNativeEntry HOST_TIMER_TABLE[] = {
    F103_TIMER_TABLE_INIT
};

/* ---- 主机端 I2C 替身 ----
 * PC 上没有 I2C 总线，这里只做两件事：
 *   1) 与板级完全相同的参数校验与错误码；
 *   2) 把「发出去的字节」打到 stdout，供逐条核对 SSD1306 初始化命令序列。
 * 为了让扫描与初始化示例在主机上跑通，模拟 0x3C 有应答（SSD1306 常见地址）。
 */
static uint8_t g_host_i2c_ready = 0u;
static uint8_t g_host_i2c_hw = 0u;

static int n_i2c_init(JmStateRef st, const JmValue *args, uint8_t nargs,
                      JmValue *out) {
    int32_t scl = 0;
    int32_t sda = 0;
    int32_t khz = 0;

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

    g_host_i2c_hw = (scl == 26 && sda == 27) ? 1u : 0u;
    g_host_i2c_ready = 1u;
    printf("[I2C] 初始化：%s，SCL=%d SDA=%d，%d kHz\n",
           g_host_i2c_hw ? "硬件 I2C2" : "软件位翻转", scl, sda, khz);
    *out = jm_value_bool(g_host_i2c_hw);
    return JM_NATIVE_OK;
}

static int host_i2c_check_addr(JmStateRef st, int32_t addr) {
    if (addr < 0 || addr > 127) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 设备地址范围 0–127");
    }
    if (g_host_i2c_ready == 0u) {
        return jm_native_fail(st, JM_NE_STATE, "I2C 未初始化，先调用 I2C初始化()");
    }
    if (addr != 0x3C) {
        char msg[96];
        snprintf(msg, sizeof(msg), "I2C 设备 0x%02X 未应答（主机端口只模拟 0x3C）",
                 (unsigned)addr);
        return jm_native_fail(st, JM_NE_HW, msg);
    }
    return 0;
}

static void host_i2c_trace(int32_t addr, uint32_t ctrl, const uint32_t *data,
                           uint8_t count) {
    uint8_t i;
    if (ctrl != 0x00u) {
        return;   /* 0x40 是显存数据，逐字节打印会把输出冲爆 */
    }
    printf("[I2C] 0x%02X 命令：", (unsigned)addr);
    for (i = 0u; i < count; i++) {
        printf(" 0x%02X", (unsigned)data[i]);
    }
    printf("\n");
}

static int n_i2c_write(JmStateRef st, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    int32_t addr = 0;
    int32_t ctrl = 0;
    int32_t data = 0;
    uint32_t byte;

    if (nargs != 3) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "I2C写需要三个参数：设备地址, 控制字节, 数据字节");
    }
    if (jm_value_to_int(args[0], &addr) != 0 ||
        jm_value_to_int(args[1], &ctrl) != 0 ||
        jm_value_to_int(args[2], &data) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "I2C写：参数必须是整数");
    }
    if (ctrl < 0 || ctrl > 255 || data < 0 || data > 255) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 控制字节/数据字节范围 0–255");
    }
    if (host_i2c_check_addr(st, addr) != 0) {
        return -1;
    }
    byte = (uint32_t)data;
    host_i2c_trace(addr, (uint32_t)ctrl, &byte, 1u);
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_i2c_write_block(JmStateRef st, const JmValue *args, uint8_t nargs,
                             JmValue *out) {
    int32_t addr = 0;
    int32_t ctrl = 0;
    uint32_t data[6];
    uint8_t count = 0u;
    uint8_t i;

    if (nargs < 3 || nargs > 8) {
        return jm_native_fail(st, JM_NE_PARAM,
                              "I2C写块需要 3–8 个参数：设备地址, 控制字节, 数据1[, 数据2 … 数据6]");
    }
    if (jm_value_to_int(args[0], &addr) != 0 ||
        jm_value_to_int(args[1], &ctrl) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "I2C写块：地址与控制字节必须是整数");
    }
    if (ctrl < 0 || ctrl > 255) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 控制字节范围 0–255");
    }
    for (i = 2u; i < nargs; i++) {
        int32_t v = 0;
        if (jm_value_to_int(args[i], &v) != 0) {
            return jm_native_fail(st, JM_NE_TYPE, "I2C 数据字节必须是整数");
        }
        if (v < 0 || v > 255) {
            return jm_native_fail(st, JM_NE_PARAM, "I2C 数据字节范围 0–255");
        }
        data[count] = (uint32_t)v;
        count++;
    }
    if (host_i2c_check_addr(st, addr) != 0) {
        return -1;
    }
    host_i2c_trace(addr, (uint32_t)ctrl, data, count);
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_i2c_probe(JmStateRef st, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    int32_t addr = 0;

    if (nargs != 1 || jm_value_to_int(args[0], &addr) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "I2C探测需要一个整数参数：设备地址");
    }
    if (addr < 0 || addr > 127) {
        return jm_native_fail(st, JM_NE_PARAM, "I2C 设备地址范围 0–127");
    }
    if (g_host_i2c_ready == 0u) {
        return jm_native_fail(st, JM_NE_STATE, "I2C 未初始化，先调用 I2C初始化()");
    }
    *out = jm_value_bool(addr == 0x3C);   /* 主机只模拟 SSD1306 的常见地址 */
    return JM_NATIVE_OK;
}

static const JmNativeEntry HOST_BUS_TABLE[] = {
    F103_BUS_TABLE_INIT
};

int jm_host_native_register(void) {
    if (jm_native_register(HOST_QUEUE_TABLE, (uint8_t)JM_QUEUE_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(HOST_SEM_TABLE, (uint8_t)JM_SEM_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(HOST_IRQ_TABLE, (uint8_t)JM_IRQ_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(HOST_ADC_TABLE, (uint8_t)F103_ADC_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(HOST_TIMER_TABLE, (uint8_t)F103_TIMER_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(HOST_BUS_TABLE, (uint8_t)F103_BUS_ENTRY_COUNT) != 0) {
        return -1;
    }
    return 0;
}

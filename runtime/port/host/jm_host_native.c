#include <stdint.h>
#include "jm_native.h"
#include "jm_native_table.h"

/*
 * 主机端「任务」原生表：与板级同一份 ABI（jm_native_table.h 由 native_spec.rs 生成）。
 * 主机只有一个执行流，所以 让出() 让出一次时间片即可；延时() 真正睡满毫秒数。
 * 这样同一份 .jsh 在 PC 上的可见输出与板上一致（时序精度不同）。
 */

#ifdef _WIN32
#include <windows.h>
static void host_yield(void) {
    SwitchToThread();
}
static void host_sleep_ms(uint32_t ms) {
    Sleep((DWORD)ms);
}
#else
#include <sched.h>
#include <time.h>
static void host_yield(void) {
    sched_yield();
}
static void host_sleep_ms(uint32_t ms) {
    struct timespec ts;
    ts.tv_sec = (time_t)(ms / 1000u);
    ts.tv_nsec = (long)(ms % 1000u) * 1000000L;
    nanosleep(&ts, 0);
}
#endif

static int n_task_yield(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    (void)args;
    if (nargs != 0) {
        return jm_native_fail(st, JM_NE_PARAM, "让出不接受参数");
    }
    host_yield();
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static int n_task_delay(JmStateRef st, const JmValue *args, uint8_t nargs,
                        JmValue *out) {
    int32_t ms = 0;
    if (nargs != 1) {
        return jm_native_fail(st, JM_NE_PARAM, "延时需要一个参数：毫秒");
    }
    if (jm_value_to_int(args[0], &ms) != 0) {
        return jm_native_fail(st, JM_NE_TYPE, "延时：参数必须是整数");
    }
    if (ms < 0 || ms > 60000) {
        return jm_native_fail(st, JM_NE_PARAM, "延时范围 0–60000");
    }
    host_sleep_ms((uint32_t)ms);
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static const JmNativeEntry HOST_TASK_TABLE[] = {
    JM_TASK_TABLE_INIT
};

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
        host_sleep_ms(timeout);
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
        host_sleep_ms(timeout);
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
        host_sleep_ms(timeout);
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
#define HOST_IRQ_LINES 5u

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
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–4（PA0–PA4）");
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
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–4（PA0–PA4）");
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
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–4（PA0–PA4）");
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
        return jm_native_fail(st, JM_NE_PARAM, "引脚范围 0–4（PA0–PA4）");
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

int jm_host_native_register(void) {
    if (jm_native_register(HOST_TASK_TABLE, (uint8_t)JM_TASK_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(HOST_QUEUE_TABLE, (uint8_t)JM_QUEUE_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(HOST_SEM_TABLE, (uint8_t)JM_SEM_ENTRY_COUNT) != 0) {
        return -1;
    }
    if (jm_native_register(HOST_IRQ_TABLE, (uint8_t)JM_IRQ_ENTRY_COUNT) != 0) {
        return -1;
    }
    return 0;
}

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

int jm_host_native_register(void) {
    return jm_native_register(HOST_TASK_TABLE, (uint8_t)JM_TASK_ENTRY_COUNT);
}

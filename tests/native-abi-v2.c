/*
 * Native ABI v2 主机 C 回归（不上板）。
 * 覆盖：登记 flags 校验、ISR 解析/直调、上下文 st == NULL、错误码、
 *       普通 jm_native_call 不受 flags 影响。
 */

#include <stdio.h>
#include <string.h>

#include "jm_internal.h"
#include "jm_native.h"

static int g_fail;

static void check(const char *name, int ok, int got, int want) {
    if (ok) {
        printf("[通过] %s\n", name);
        return;
    }
    printf("[不通过] %s：期望 %d，实际 %d\n", name, want, got);
    g_fail++;
}

static JmStateRef g_seen_st;
static int g_plain_called;
static int g_state_called;

/* 安全项：st 是上下文信号；任务上下文照常可用。 */
static int safe_fn(JmStateRef st, const JmValue *args, uint8_t nargs,
                   JmValue *out) {
    int32_t v = 0;
    (void)nargs;

    g_seen_st = st;
    if (jm_value_to_int(args[0], &v) != 0) {
        return JM_NE_TYPE;
    }
    *out = jm_value_int(v + 1);
    return JM_NATIVE_OK;
}

/* 普通项：ISR 直调不得进入这里。 */
static int plain_fn(JmStateRef st, const JmValue *args, uint8_t nargs,
                    JmValue *out) {
    (void)st;
    (void)args;
    (void)nargs;
    g_plain_called++;
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

/* 模拟「前置条件未满足」的安全项：ISR 上下文只能返回正数 JM_NE_*。 */
static int need_state_fn(JmStateRef st, const JmValue *args, uint8_t nargs,
                         JmValue *out) {
    (void)args;
    (void)nargs;
    g_state_called++;
    if (!st) {
        return JM_NE_STATE;
    }
    *out = jm_value_nil();
    return JM_NATIVE_OK;
}

static const JmNativeEntry TABLE[] = {
    {"安全项", safe_fn, 1, 1, JM_NATIVE_ISR_SAFE, 0},
    {"普通项", plain_fn, 0, 0, 0, 0},
    {"需状态项", need_state_fn, 0, 0, JM_NATIVE_ISR_SAFE, 0},
};

static const JmNativeEntry BAD_FLAGS[] = {
    {"坏标志", safe_fn, 1, 1, 0x02u, 0},
};

static const JmNativeEntry BAD_RESERVED[] = {
    {"坏保留位", safe_fn, 1, 1, JM_NATIVE_ISR_SAFE, 1},
};

int main(void) {
    uint8_t safe_id = 0xFFu;
    uint8_t plain_id = 0xFFu;
    uint8_t state_id = 0xFFu;
    uint8_t missing_id = 0xFFu;
    JmValue args[2];
    JmValue out = JM_NIL;
    int rc;

    rc = jm_native_register(TABLE, (uint8_t)(sizeof(TABLE) / sizeof(TABLE[0])));
    check("登记合法表", rc == 0, rc, 0);

    rc = jm_native_register(BAD_FLAGS, 1);
    check("登记拒绝未知 flags 位", rc == -1, rc, -1);

    rc = jm_native_register(BAD_RESERVED, 1);
    check("登记拒绝非 0 reserved", rc == -1, rc, -1);

    rc = jm_native_isr_resolve("安全项", &safe_id);
    check("解析安全项", rc == 0 && safe_id != 0xFFu, rc, 0);

    rc = jm_native_isr_resolve("普通项", &plain_id);
    check("解析普通项返回 -2", rc == JM_NATIVE_ISR_E_NOT_SAFE, rc,
          JM_NATIVE_ISR_E_NOT_SAFE);
    /* 解析失败不会写 id；后续要验证「直调被拒绝」，用 TABLE 里的已知下标。 */
    plain_id = 1u;

    rc = jm_native_isr_resolve("不存在", &missing_id);
    check("解析不存在的名字返回 -1", rc == JM_NATIVE_ISR_E_ID, rc,
          JM_NATIVE_ISR_E_ID);

    rc = jm_native_isr_resolve("需状态项", &state_id);
    check("解析需状态项", rc == 0, rc, 0);

    /* 安全项：正常直调，st 必须是 NULL，返回值写回 out。 */
    args[0] = jm_value_int(41);
    rc = jm_native_isr_call(safe_id, args, 1, &out);
    check("安全项直调成功", rc == 0, rc, 0);
    check("安全项 st == NULL", g_seen_st == NULL, g_seen_st != NULL, 0);
    check("安全项返回值正确", jm_value_get_int(out) == 42,
          (int)jm_value_get_int(out), 42);

    /* out 可为 NULL：只执行、不取结果。 */
    rc = jm_native_isr_call(safe_id, args, 1, NULL);
    check("安全项 out 可为 NULL", rc == 0, rc, 0);

    /* 普通项：直调拒绝，且函数体没有执行。 */
    g_plain_called = 0;
    rc = jm_native_isr_call(plain_id, NULL, 0, NULL);
    check("普通项直调返回 -2", rc == JM_NATIVE_ISR_E_NOT_SAFE, rc,
          JM_NATIVE_ISR_E_NOT_SAFE);
    check("普通项函数体未执行", g_plain_called == 0, g_plain_called, 0);

    /* 实参个数越界。 */
    rc = jm_native_isr_call(safe_id, NULL, 0, NULL);
    check("实参太少返回 -3", rc == JM_NATIVE_ISR_E_ARGC, rc,
          JM_NATIVE_ISR_E_ARGC);
    rc = jm_native_isr_call(safe_id, args, 2, NULL);
    check("实参太多返回 -3", rc == JM_NATIVE_ISR_E_ARGC, rc,
          JM_NATIVE_ISR_E_ARGC);

    /* args == NULL 但 nargs != 0。 */
    rc = jm_native_isr_call(safe_id, NULL, 1, NULL);
    check("args 为 NULL 返回 -4", rc == JM_NATIVE_ISR_E_PARAM, rc,
          JM_NATIVE_ISR_E_PARAM);

    /* 非法 id。 */
    rc = jm_native_isr_call(200u, NULL, 0, NULL);
    check("非法 id 返回 -1", rc == JM_NATIVE_ISR_E_ID, rc, JM_NATIVE_ISR_E_ID);

    /* 前置条件不满足：正数错误码原样返回。 */
    rc = jm_native_isr_call(state_id, NULL, 0, NULL);
    check("前置条件失败返回 33", rc == JM_NE_STATE, rc, JM_NE_STATE);

    /* 普通分派路径不受 flags 影响。 */
    g_plain_called = 0;
    rc = jm_native_call(&jm_state, (uint8_t)(JM_NATIVE_ID_BASE + safe_id),
                        args, 1, &out);
    check("普通调用安全项成功", rc == 0 && jm_value_get_int(out) == 42,
          rc, 0);
    check("普通调用 st 非 NULL", g_seen_st == &jm_state, g_seen_st != &jm_state,
          0);
    rc = jm_native_call(&jm_state, (uint8_t)(JM_NATIVE_ID_BASE + plain_id),
                        NULL, 0, &out);
    check("普通调用普通项成功", rc == 0 && g_plain_called == 1,
          rc, 0);

    if (g_fail != 0) {
        printf("Native ABI v2 主机回归失败：%d 项\n", g_fail);
        return 1;
    }
    printf("Native ABI v2 主机回归：全部通过\n");
    return 0;
}

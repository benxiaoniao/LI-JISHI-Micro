#include "jm_internal.h"
#include "jm_native.h"

/*
 * 原生函数表（Native ABI v2）的实现。
 * 公开契约在 jm_native.h 与 docs/原生函数表.md；这里只做登记、绑定、分派与值读写。
 */

#define JM_NATIVE_MAX_TABLES 8
#define JM_NATIVE_MAX_ENTRIES 64

static const JmNativeEntry *g_entries[JM_NATIVE_MAX_ENTRIES];
static uint8_t g_ntables;
static uint8_t g_nentries;

/* ---------------- 登记 ---------------- */

int jm_native_register(const JmNativeEntry *entries, uint8_t count) {
    if (!entries || count == 0) {
        return -1;
    }
    if (g_ntables >= JM_NATIVE_MAX_TABLES) {
        return -1;
    }
    if ((uint32_t)g_nentries + count > JM_NATIVE_MAX_ENTRIES) {
        return -1;
    }

    /* 先整体校验，避免半截登记 */
    for (uint8_t i = 0; i < count; i++) {
        const JmNativeEntry *e = &entries[i];
        if (!e->name || !e->fn) {
            return -1;
        }
        if (e->max_args > JM_NATIVE_MAX_ARGS || e->min_args > e->max_args) {
            return -1;
        }
        if ((e->flags & (uint8_t)~JM_NATIVE_ISR_SAFE) != 0 || e->reserved != 0) {
            return -1;
        }
        for (uint8_t k = 0; k < g_nentries; k++) {
            if (strcmp(g_entries[k]->name, e->name) == 0) {
                return -1;
            }
        }
        for (uint8_t k = 0; k < i; k++) {
            if (strcmp(entries[k].name, e->name) == 0) {
                return -1;
            }
        }
    }

    for (uint8_t i = 0; i < count; i++) {
        g_entries[g_nentries++] = &entries[i];
    }
    g_ntables++;
    return 0;
}

unsigned jm_native_abi_version(void) {
    return JM_NATIVE_ABI_VERSION;
}

/* ---------------- ISR 直调 ---------------- */

int jm_native_isr_resolve(const char *name, uint8_t *id) {
    if (!name || !id) {
        return JM_NATIVE_ISR_E_PARAM;
    }
    for (uint8_t i = 0; i < g_nentries; i++) {
        if (strcmp(g_entries[i]->name, name) == 0) {
            if ((g_entries[i]->flags & JM_NATIVE_ISR_SAFE) == 0) {
                return JM_NATIVE_ISR_E_NOT_SAFE;
            }
            *id = i;
            return JM_NATIVE_ISR_OK;
        }
    }
    return JM_NATIVE_ISR_E_ID;
}

int jm_native_isr_call(uint8_t id, const JmValue *args, uint8_t nargs,
                       JmValue *out) {
    const JmNativeEntry *e;
    JmValue tmp;
    int rc;

    if (id >= g_nentries) {
        return JM_NATIVE_ISR_E_ID;
    }
    e = g_entries[id];
    if ((e->flags & JM_NATIVE_ISR_SAFE) == 0) {
        return JM_NATIVE_ISR_E_NOT_SAFE;
    }
    if (nargs < e->min_args || nargs > e->max_args) {
        return JM_NATIVE_ISR_E_ARGC;
    }
    if (args == NULL && nargs != 0) {
        return JM_NATIVE_ISR_E_PARAM;
    }

    /* st == NULL 是 ISR 上下文的唯一信号；这里不碰 JmState。 */
    tmp = JM_NIL;
    rc = e->fn(NULL, args, nargs, &tmp);
    if (rc != 0) {
        return rc;
    }
    if (out) {
        *out = tmp;
    }
    return JM_NATIVE_ISR_OK;
}

/* ---------------- 绑定到名字表 ---------------- */

int jm_native_bind_name(JmState *st, uint8_t name_idx, const char *name) {
    for (uint8_t i = 0; i < g_nentries; i++) {
        if (strcmp(g_entries[i]->name, name) == 0) {
            st->globals[name_idx] = jm_v_builtin((uint8_t)(JM_NATIVE_ID_BASE + i));
            return 1;
        }
    }
    return 0;
}

/* ---------------- 分派 ---------------- */

/* 按名字查 native id；找不到返回 0（native id 从 64 起，0 可当「无」）。 */
uint8_t jm_native_find(const char *name) {
    for (uint8_t i = 0; i < g_nentries; i++) {
        if (strcmp(g_entries[i]->name, name) == 0) {
            return (uint8_t)(JM_NATIVE_ID_BASE + i);
        }
    }
    return 0;
}

int jm_native_call(JmState *st, uint8_t id, JmValue *args, uint8_t nargs,
                   JmValue *out) {
    uint8_t idx = (uint8_t)(id - JM_NATIVE_ID_BASE);
    if (idx >= g_nentries) {
        return jm_fail(st, E_NOT_CALLABLE, "未知原生函数");
    }
    const JmNativeEntry *e = g_entries[idx];
    if (nargs < e->min_args || nargs > e->max_args) {
        return jm_fail_aux(st, E_ARGC, "原生函数实参个数不符", (int)nargs);
    }

    JmValue tmp = JM_NIL;
    int rc = e->fn(st, args, nargs, &tmp);
    if (rc != 0) {
        if (!st->has_err) {
            jm_fail(st, JM_NE_STATE, "原生函数失败（未调用 jm_native_fail）");
        }
        return -1;
    }
    *out = tmp;
    return 0;
}

/* ---------------- 值读写 ---------------- */

int jm_value_kind(JmValue v) {
    switch (JM_TAG(v)) {
    case JM_TAG_INT:
        return JM_VK_INT;
    case JM_TAG_TRUE:
    case JM_TAG_FALSE:
        return JM_VK_BOOL;
    case JM_TAG_NIL:
        return JM_VK_NIL;
    case JM_TAG_HEAP:
        return (jm_obj(v)->type == JM_T_STR) ? JM_VK_TEXT : JM_VK_OTHER;
    default:
        return JM_VK_OTHER;
    }
}

int jm_value_to_int(JmValue v, int32_t *out) {
    if (!out || JM_TAG(v) != JM_TAG_INT) {
        return -1;
    }
    *out = jm_int(v);
    return 0;
}

int32_t jm_value_get_int(JmValue v) {
    return jm_int(v);
}

int jm_value_text(JmValue v, const char **s, uint32_t *len) {
    if (!s || !len) {
        return -1;
    }
    if (JM_TAG(v) != JM_TAG_HEAP || jm_obj(v)->type != JM_T_STR) {
        return -1;
    }
    JmStr *str = (JmStr *)jm_obj(v);
    *s = (const char *)str->data;
    *len = str->len;
    return 0;
}

JmValue jm_value_int(int32_t v) {
    if (v < JM_INT_MIN || v > JM_INT_MAX) {
        return JM_NIL;   /* 超出 29 位整数范围：契约要求调用方先做范围检查 */
    }
    return jm_v_int(v);
}

JmValue jm_value_bool(int b) {
    return jm_v_bool(b);
}

JmValue jm_value_nil(void) {
    return JM_NIL;
}

int jm_value_make_text(JmStateRef st, const char *s, uint32_t len, JmValue *out) {
    if (!st || !out) {
        return -1;
    }
    return jm_str_new((JmState *)st, s, len, out);
}

void jm_value_retain(JmValue v) {
    jm_retain(v);
}

void jm_value_release(JmValue v) {
    jm_release(v);
}

int jm_native_fail(JmStateRef st, int code, const char *detail) {
    if (!st) {
        return -1;
    }
    return jm_fail((JmState *)st, code, detail);
}

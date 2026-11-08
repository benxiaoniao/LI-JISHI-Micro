#include "jm_debug.h"
#include "jm_internal.h"

#if JM_DEBUG_HOOKS

static uint16_t rd16(const uint8_t *p) {
    return (uint16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
}

static const uint8_t *ce_of(uint8_t i) {
    return jm_state.base + jm_state.code_start + (uint32_t)i * 10u;
}

static const char *name_of(uint8_t i) {
    uint16_t off = rd16(jm_state.base + jm_state.name_start + (uint32_t)i * 2u);
    return (const char *)(jm_state.base + off);
}

/* ---------------- 暂停状态 ---------------- */

static int g_paused = 0;
static uint8_t g_paused_task = 0;

void jm_dbg_mark_paused(uint8_t task) {
    g_paused = 1;
    g_paused_task = task;
}

int jm_dbg_paused(void) { return g_paused; }

int jm_dbg_resume(void) {
    if (!g_paused) {
        return -1;
    }
    JmState *st = &jm_state;
    /* 让调度器从暂停任务开始轮转：jm_green_pick 先试 current+1，把 current
     * 挪到暂停任务的前一个槽位即可保证第一步回到它。 */
    st->current = (uint8_t)((g_paused_task + st->ntask - 1u) % st->ntask);
    g_paused = 0;
    return jm_execute(st);
}

/* ---------------- 任务视图 ---------------- */

int jm_dbg_task_count(void) { return (int)jm_state.ntask; }

int jm_dbg_current_task(void) { return (int)jm_state.current; }

int jm_dbg_task_state(int task) {
    if (task < 0 || task >= (int)jm_state.ntask) {
        return -1;
    }
    return (int)jm_state.tasks[task].state;
}

/* ---------------- 调用栈 ---------------- */

int jm_dbg_frames(int task, JmDbgFrame *out, int max) {
    JmState *st = &jm_state;
    if (task < 0 || task >= (int)st->ntask || !out || max <= 0) {
        return -1;
    }
    JmGreenTask *g = &st->tasks[task];
    int fp = (int)g->fp;
    int n = 0;
    for (int i = fp; i >= 0 && n < max; i--, n++) {
        const JmFrame *f = &g->frames[i];
        out[n].code = f->code_idx;
        out[n].ip = f->ip;
        out[n].base = f->base;
        out[n].nlocals = ce_of(f->code_idx)[6];
    }
    return n;
}

/* ---------------- 值读取 ---------------- */

int jm_dbg_local(int task, int frame, uint8_t slot, JmValue *out) {
    JmState *st = &jm_state;
    if (task < 0 || task >= (int)st->ntask || frame < 0 || !out) {
        return -1;
    }
    JmGreenTask *g = &st->tasks[task];
    int idx = (int)g->fp - frame;
    if (idx < 0) {
        return -1;
    }
    const JmFrame *f = &g->frames[idx];
    uint8_t nl = ce_of(f->code_idx)[6];
    if (slot >= nl) {
        return -1;
    }
    uint32_t vi = (uint32_t)f->base + slot;
    if (vi >= g->stack_cap) {
        return -1;
    }
    JmValue v = g->stack[vi];
    if (v == JM_UNDEF) {
        return -1; /* 槽还没被赋值 */
    }
    *out = v;
    return 0;
}

/* 钩子内读当前帧：st->fp 是执行中的活下标，tasks[].fp 只在让出时保存。 */
int jm_dbg_local_current(uint8_t slot, JmValue *out) {
    JmState *st = &jm_state;
    int ti = (int)st->current;
    if (ti < 0 || ti >= (int)st->ntask || !out) {
        return -1;
    }
    JmGreenTask *g = &st->tasks[ti];
    int idx = (int)st->fp;
    if (idx < 0) {
        return -1;
    }
    const JmFrame *f = &g->frames[idx];
    uint8_t nl = ce_of(f->code_idx)[6];
    if (slot >= nl) {
        return -1;
    }
    uint32_t vi = (uint32_t)f->base + slot;
    if (vi >= g->stack_cap) {
        return -1;
    }
    JmValue v = g->stack[vi];
    if (v == JM_UNDEF) {
        return -1;
    }
    *out = v;
    return 0;
}

int jm_dbg_global(const char *name, JmValue *out) {
    JmState *st = &jm_state;
    if (!name || !out) {
        return -1;
    }
    for (uint8_t i = 0; i < st->nnames; i++) {
        if (strcmp(name_of(i), name) == 0) {
            if (st->globals[i] == JM_UNDEF) {
                return -1;
            }
            *out = st->globals[i];
            return 0;
        }
    }
    return -1;
}

int jm_dbg_code_name(uint8_t code, const char **out) {
    JmState *st = &jm_state;
    if (code >= st->ncodes || !out) {
        return -1;
    }
    *out = name_of(ce_of(code)[4]);
    return 0;
}

/* ---------------- 值 → 文本 ---------------- */

typedef struct {
    char *buf;
    size_t cap;
    size_t len;
} TextSink;

static int text_sink(void *ctx, const char *s, uint32_t n) {
    TextSink *b = (TextSink *)ctx;
    if (b->cap == 0) {
        return -1;
    }
    size_t room = b->cap - 1u - b->len;
    if ((size_t)n > room) {
        return -1; /* 一次写不完就报错，避免悄悄截断 */
    }
    memcpy(b->buf + b->len, s, n);
    b->len += n;
    b->buf[b->len] = '\0';
    return 0;
}

int jm_dbg_value_text(JmValue v, char *buf, size_t cap) {
    if (!buf || cap == 0) {
        return -1;
    }
    buf[0] = '\0';
    TextSink s = {buf, cap, 0};
    return jm_repr(&jm_state, v, 0, text_sink, &s, 0);
}

/* ---------------- 条件断点的最小类型视图 ---------------- */

int jm_dbg_value_kind(JmValue v) {
    switch (JM_TAG(v)) {
    case JM_TAG_INT:
        return JM_DBG_KIND_INT;
    case JM_TAG_TRUE:
    case JM_TAG_FALSE:
        return JM_DBG_KIND_BOOL;
    case JM_TAG_HEAP:
        return jm_obj(v)->type == JM_T_STR ? JM_DBG_KIND_TEXT : JM_DBG_KIND_OTHER;
    default:
        return JM_DBG_KIND_OTHER;
    }
}

int32_t jm_dbg_value_int(JmValue v) { return jm_int(v); }

int jm_dbg_value_bool(JmValue v) { return JM_TAG(v) == JM_TAG_TRUE ? 1 : 0; }

/* 端口没接入调试交互时的空钩子：弱符号，端口给了强定义就自动让位。
 * 有了它，"只打开开关、不接交互"的固件也能独立链接并量体积。 */
__attribute__((weak)) int jm_host_dbg_tick(uint8_t code, uint16_t ip, uint8_t fp) {
    (void)code;
    (void)ip;
    (void)fp;
    return 0;
}

#endif /* JM_DEBUG_HOOKS */

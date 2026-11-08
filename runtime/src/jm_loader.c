#include "jm_internal.h"

static uint16_t rd16(const uint8_t *p) {
    return (uint16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
}
static uint32_t rd32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

int jm_loader_init(JmState *st, const void *data, size_t len) {
    st->base = (const uint8_t *)data;
    st->len = (uint32_t)len;
    if (len < 12) {
        return jm_fail(st, E_FILE, "文件太短");
    }
    const uint8_t *p = st->base;
    if (p[0] != 'J' || p[1] != 'M' || p[2] != 'B' || p[3] != '1') {
        return jm_fail(st, E_FILE, "不是 .jmb 文件（魔数不对）");
    }
    if (p[4] != 4) {
        return jm_fail(st, E_FILE, "不支持的字节码版本（需要 JMB v4）");
    }
    st->nconsts = p[6];
    st->nnames = p[7];
    st->ncodes = p[8];
    st->entry = p[9];
    st->nclasses = p[10];
    if (st->entry >= st->ncodes) {
        return jm_fail(st, E_FILE, "入口代码下标越界");
    }

    st->const_start = 12;
    st->name_start = st->const_start + (uint32_t)st->nconsts * 8u;
    st->code_start = st->name_start + (uint32_t)st->nnames * 2u;
    uint32_t cap_start = st->code_start + (uint32_t)st->ncodes * 10u;
    uint32_t ncap_total = 0;
    for (uint8_t i = 0; i < st->ncodes; i++) {
        ncap_total += st->base[st->code_start + (uint32_t)i * 10u + 8u];
    }
    uint32_t tables_end = cap_start + ncap_total * 2u;
    if (tables_end > st->len) {
        return jm_fail(st, E_FILE, "表区越界");
    }

    for (uint8_t i = 0; i < st->nconsts; i++) {
        const uint8_t *e = st->base + st->const_start + (uint32_t)i * 8u;
        uint8_t kind = e[0];
        if (kind > K_DEC) {
            return jm_fail(st, E_FILE, "未知常量种类");
        }
        if (kind == K_STR) {
            uint16_t slen = rd16(e + 2);
            uint32_t off = rd32(e + 4);
            if ((uint64_t)off + slen > st->len) {
                return jm_fail(st, E_FILE, "字符串常量越界");
            }
        }
    }
    for (uint8_t i = 0; i < st->nnames; i++) {
        uint16_t off = rd16(st->base + st->name_start + (uint32_t)i * 2u);
        if (off >= st->len) {
            return jm_fail(st, E_FILE, "名字偏移越界");
        }
    }
    for (uint8_t i = 0; i < st->ncodes; i++) {
        const uint8_t *e = st->base + st->code_start + (uint32_t)i * 10u;
        uint16_t off = rd16(e + 0);
        uint16_t clen = rd16(e + 2);
        if ((uint32_t)off + clen > st->len) {
            return jm_fail(st, E_FILE, "指令流越界");
        }
        if (e[5] > e[6]) {
            return jm_fail(st, E_FILE, "形参个数大于局部槽数");
        }
        if (e[4] >= st->nnames) {
            return jm_fail(st, E_FILE, "代码名字下标越界");
        }
    }

    /* 捕获表：每项 (父帧槽, 子帧槽)，子帧槽必须落在该函数的局部槽内。 */
    uint32_t cap_cur = cap_start;
    for (uint8_t i = 0; i < st->ncodes; i++) {
        const uint8_t *e = st->base + st->code_start + (uint32_t)i * 10u;
        uint8_t ncap = e[8];
        uint8_t nlocals = e[6];
        for (uint8_t k = 0; k < ncap; k++, cap_cur += 2u) {
            uint8_t cslot = st->base[cap_cur + 1u];
            if (cslot >= nlocals) {
                return jm_fail(st, E_FILE, "捕获槽越界");
            }
        }
    }
    if (cap_cur < tables_end) {
        return jm_fail(st, E_FILE, "捕获表长度不一致");
    }

    /* 类表：定长 8 B 记录数组，随后是字段名数组与方法(name,code)数组，
     * 记录里的偏移都相对 class_start。 */
    st->class_start = tables_end;
    uint32_t cls_end = st->class_start + (uint32_t)st->nclasses * 8u;
    if (cls_end > st->len) {
        return jm_fail(st, E_FILE, "类表越界");
    }
    for (uint8_t i = 0; i < st->nclasses; i++) {
        const uint8_t *e = st->base + st->class_start + (uint32_t)i * 8u;
        uint8_t nfields = e[1];
        uint8_t nmethods = e[2];
        uint8_t init_code = e[3];
        uint32_t foff = st->class_start + rd16(e + 4);
        uint32_t moff = st->class_start + rd16(e + 6);
        if (e[0] >= st->nnames) {
            return jm_fail(st, E_FILE, "类名下标越界");
        }
        if (init_code >= st->ncodes) {
            return jm_fail(st, E_FILE, "初始化代码下标越界");
        }
        if (foff + nfields > st->len || moff + (uint32_t)nmethods * 2u > st->len) {
            return jm_fail(st, E_FILE, "类成员表越界");
        }
        for (uint8_t k = 0; k < nfields; k++) {
            if (st->base[foff + k] >= st->nnames) {
                return jm_fail(st, E_FILE, "字段名下标越界");
            }
        }
        for (uint8_t k = 0; k < nmethods; k++) {
            const uint8_t *m = st->base + moff + (uint32_t)k * 2u;
            if (m[0] >= st->nnames || m[1] >= st->ncodes) {
                return jm_fail(st, E_FILE, "方法表下标越界");
            }
        }
    }
    return 0;
}

/* 载入后把三种绿色等待的 native id 先查好，运行期不再按名字查表。
 * 注意不能扫 .jmb 名字表：等待调用编译成 OP_TASK_WAIT（立即数带种类），
 * 名字表里根本没有这三个名字，只能直接问 native 表。 */
static void jm_resolve_wait_native(JmState *st) {
    static const char *const names[3] = {"队列接收", "队列发送", "信号量等待"};
    for (uint8_t k = 0; k < 3; k++) {
        st->wait_native[k] = jm_native_find(names[k]);
    }
}

int jm_run(const void *jmb, size_t len) {
    JmState *st = &jm_state;
    memset(st, 0, sizeof(*st));
#if JM_STACK_WATERMARK
    jm_stack_hw = 0;
#endif
    for (int i = 0; i < JM_MAX_GLOBALS; i++) {
        st->globals[i] = JM_UNDEF;
        st->const_cache[i] = JM_UNDEF;
    }
    jm_heap_init();

    /* 绿色任务：root 用原有的静态数组，子任务用各自切片。 */
    st->stack = st->root_stack;
    st->stack_cap = JM_STACK_SLOTS;
    st->frames = st->root_frames;
    st->frames_cap = JM_MAX_FRAMES;
    st->tries = st->root_tries;
    st->tries_cap = JM_MAX_TRY_DEPTH;
    st->tasks[0].stack = st->root_stack;
    st->tasks[0].stack_cap = JM_STACK_SLOTS;
    st->tasks[0].frames = st->root_frames;
    st->tasks[0].frames_cap = JM_MAX_FRAMES;
    st->tasks[0].tries = st->root_tries;
    st->tasks[0].tries_cap = JM_MAX_TRY_DEPTH;
    st->tasks[0].state = JM_TASK_READY;
    st->tasks[0].body = JM_UNDEF;
    st->current = 0;
    st->ntask = 1;

    if (jm_loader_init(st, jmb, len)) {
        st->has_err = 1;
        return -1;
    }
    jm_builtin_install(st);
    jm_resolve_wait_native(st);

    const uint8_t *ce = st->base + st->code_start + (uint32_t)st->entry * 10u;
    uint8_t nlocals = ce[6];
    st->frames[0].code_idx = st->entry;
    st->frames[0].ip = 0;
    st->frames[0].base = 0;
    st->fp = 0;
    st->sp = 0;
    for (uint8_t i = 0; i < nlocals; i++) {
        if (st->sp >= (int)st->stack_cap) {
            jm_fail(st, E_STACK, "值栈溢出");
            st->has_err = 1;
            return -1;
        }
        st->stack[st->sp++] = JM_UNDEF;
    }
    JM_HW_NOTE(st);
    st->tasks[0].sp = (uint16_t)st->sp; /* root 的局部槽已铺好，交给调度器 */

    if (jm_execute(st)) {
        return -1;
    }
    return 0;
}

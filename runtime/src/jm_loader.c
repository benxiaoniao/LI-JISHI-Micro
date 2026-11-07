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
    if (p[4] != 1) {
        return jm_fail(st, E_FILE, "不支持的字节码版本");
    }
    st->nconsts = p[6];
    st->nnames = p[7];
    st->ncodes = p[8];
    st->entry = p[9];
    if (st->entry >= st->ncodes) {
        return jm_fail(st, E_FILE, "入口代码下标越界");
    }

    st->const_start = 12;
    st->name_start = st->const_start + (uint32_t)st->nconsts * 8u;
    st->code_start = st->name_start + (uint32_t)st->nnames * 2u;
    uint32_t tables_end = st->code_start + (uint32_t)st->ncodes * 8u;
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
        const uint8_t *e = st->base + st->code_start + (uint32_t)i * 8u;
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
    return 0;
}

int jm_run(const void *jmb, size_t len) {
    JmState *st = &jm_state;
    memset(st, 0, sizeof(*st));
    for (int i = 0; i < JM_MAX_GLOBALS; i++) {
        st->globals[i] = JM_UNDEF;
        st->const_cache[i] = JM_UNDEF;
    }
    jm_heap_init();

    if (jm_loader_init(st, jmb, len)) {
        st->has_err = 1;
        return -1;
    }
    jm_builtin_install(st);

    const uint8_t *ce = st->base + st->code_start + (uint32_t)st->entry * 8u;
    uint8_t nlocals = ce[6];
    st->frames[0].code_idx = st->entry;
    st->frames[0].ip = 0;
    st->frames[0].base = 0;
    st->fp = 0;
    st->sp = 0;
    for (uint8_t i = 0; i < nlocals; i++) {
        if (st->sp >= JM_STACK_SLOTS) {
            jm_fail(st, E_STACK, "值栈溢出");
            st->has_err = 1;
            return -1;
        }
        st->stack[st->sp++] = JM_UNDEF;
    }

    if (jm_execute(st)) {
        return -1;
    }
    return 0;
}

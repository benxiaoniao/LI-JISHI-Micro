#include "jm_internal.h"

#define B_PRINT 0
#define B_TEXT 1
#define B_INT 2
#define B_LEN 3
#define B_RANGE 4
#define B_TYPE 5
#define B_SUM 6
#define B_MAX 7
#define B_MIN 8
#define B_ABS 9

typedef struct {
    const char *name;
    uint8_t id;
} JmBuiltinDef;

static const JmBuiltinDef BUILTINS[] = {
    {"打印", B_PRINT}, {"文本", B_TEXT}, {"整数", B_INT},   {"长度", B_LEN},
    {"范围", B_RANGE}, {"类型", B_TYPE}, {"总和", B_SUM},   {"最大", B_MAX},
    {"最小", B_MIN},   {"绝对值", B_ABS},
};
#define NBUILTINS ((int)(sizeof(BUILTINS) / sizeof(BUILTINS[0])))

static const uint8_t *name_entry(JmState *st, uint8_t i) {
    uint16_t off = (uint16_t)(st->base[st->name_start + (uint32_t)i * 2u] |
                              (st->base[st->name_start + (uint32_t)i * 2u + 1] << 8));
    return st->base + off;
}

void jm_builtin_install(JmState *st) {
    for (uint8_t i = 0; i < st->nnames; i++) {
        const char *nm = (const char *)name_entry(st, i);
        for (int k = 0; k < NBUILTINS; k++) {
            if (strcmp(nm, BUILTINS[k].name) == 0) {
                st->globals[i] = jm_v_builtin(BUILTINS[k].id);
                break;
            }
        }
    }
}

/* ---------------- 输出 ---------------- */

static int port_sink(void *ctx, const char *s, uint32_t n) {
    (void)ctx;
    jm_port_write(s, n);
    return 0;
}

static void put_int(JmSink sink, void *ctx, int32_t v) {
    char buf[16];
    int n = 0;
    if (v < 0) {
        buf[n++] = '-';
        v = -v;
    }
    char tmp[12];
    int t = 0;
    if (v == 0) {
        tmp[t++] = '0';
    }
    while (v > 0) {
        tmp[t++] = (char)('0' + (v % 10));
        v /= 10;
    }
    while (t > 0) {
        buf[n++] = tmp[--t];
    }
    sink(ctx, buf, (uint32_t)n);
}

int jm_repr(JmState *st, JmValue v, int quoted, JmSink sink, void *ctx, int depth) {
    if (depth > 8) {
        return jm_fail(st, E_OBJ, "嵌套太深");
    }
    #define S(x) sink(ctx, (x), (uint32_t)strlen(x))
    switch (JM_TAG(v)) {
    case JM_TAG_INT:
        put_int(sink, ctx, jm_int(v));
        return 0;
    case JM_TAG_NIL:
        S("空");
        return 0;
    case JM_TAG_TRUE:
        S("真");
        return 0;
    case JM_TAG_FALSE:
        S("假");
        return 0;
    case JM_TAG_UNDEF:
        S("未定义");
        return 0;
    case JM_TAG_FUNC:
        S("<函数>");
        return 0;
    case JM_TAG_BUILTIN: {
        uint8_t id = (uint8_t)JM_PAYLOAD(v);
        S("<内建 ");
        if (id < NBUILTINS) {
            S(BUILTINS[id].name);
        }
        S(">");
        return 0;
    }
    case JM_TAG_HEAP: {
        JmObj *o = jm_obj(v);
        switch (o->type) {
        case JM_T_STR: {
            JmStr *s = (JmStr *)o;
            if (quoted) {
                sink(ctx, "'", 1);
            }
            sink(ctx, (const char *)s->data, s->len);
            if (quoted) {
                sink(ctx, "'", 1);
            }
            return 0;
        }
        case JM_T_LIST: {
            JmList *l = (JmList *)o;
            sink(ctx, "[", 1);
            for (uint32_t i = 0; i < l->len; i++) {
                if (i) {
                    sink(ctx, ", ", 2);
                }
                if (jm_repr(st, l->items[i], 1, sink, ctx, depth + 1)) {
                    return -1;
                }
            }
            sink(ctx, "]", 1);
            return 0;
        }
        case JM_T_DICT: {
            JmDict *d = (JmDict *)o;
            sink(ctx, "{", 1);
            for (uint32_t i = 0; i < d->len; i++) {
                if (i) {
                    sink(ctx, ", ", 2);
                }
                if (jm_repr(st, d->entries[i].key, 1, sink, ctx, depth + 1)) {
                    return -1;
                }
                sink(ctx, ": ", 2);
                if (jm_repr(st, d->entries[i].val, 1, sink, ctx, depth + 1)) {
                    return -1;
                }
            }
            sink(ctx, "}", 1);
            return 0;
        }
        case JM_T_RANGE:
            S("<范围>");
            return 0;
        case JM_T_ITER:
            S("<迭代器>");
            return 0;
        default:
            S("<对象>");
            return 0;
        }
    }
    default:
        S("<?>");
        return 0;
    }
    #undef S
}

/* ---------------- 工具 ---------------- */

static uint32_t utf8_count(const uint8_t *s, uint32_t len) {
    uint32_t i = 0, n = 0;
    while (i < len) {
        uint8_t c = s[i];
        uint32_t step = 1;
        if ((c & 0xE0) == 0xC0) step = 2;
        else if ((c & 0xF0) == 0xE0) step = 3;
        else if ((c & 0xF8) == 0xF0) step = 4;
        if (i + step > len) step = 1;
        i += step;
        n++;
    }
    return n;
}

typedef struct {
    JmState *st;
    uint8_t *p;
    uint32_t len;
    uint32_t cap;
    int oom;
} JmBuf;

static int buf_sink(void *ctx, const char *s, uint32_t n) {
    JmBuf *b = (JmBuf *)ctx;
    if (b->oom) {
        return -1;
    }
    if (b->len + n > b->cap) {
        uint32_t cap = b->cap ? b->cap : 32;
        while (cap < b->len + n) {
            cap *= 2;
        }
        uint8_t *np = (uint8_t *)jm_raw_alloc(cap);
        if (!np) {
            b->oom = 1;
            return -1;
        }
        if (b->p) {
            memcpy(np, b->p, b->len);
            jm_raw_free(b->p);
        }
        b->p = np;
        b->cap = cap;
    }
    memcpy(b->p + b->len, s, n);
    b->len += n;
    return 0;
}

static int to_text(JmState *st, JmValue v, JmValue *out) {
    if (JM_TAG(v) == JM_TAG_HEAP && jm_obj(v)->type == JM_T_STR) {
        *out = v;
        jm_retain(v);
        return 0;
    }
    JmBuf b = {st, NULL, 0, 0, 0};
    if (jm_repr(st, v, 0, buf_sink, &b, 0)) {
        if (b.p) {
            jm_raw_free(b.p);
        }
        return -1;
    }
    int rc = jm_str_new(st, b.p ? (const char *)b.p : "", b.len, out);
    if (b.p) {
        jm_raw_free(b.p);
    }
    return rc;
}

static int parse_int_str(const char *s, uint32_t len, int32_t *out) {
    uint32_t i = 0;
    while (i < len && (s[i] == ' ' || s[i] == '\t')) i++;
    int neg = 0;
    if (i < len && (s[i] == '+' || s[i] == '-')) {
        neg = s[i] == '-';
        i++;
    }
    if (i >= len) {
        return -1;
    }
    int64_t v = 0;
    int any = 0;
    while (i < len && s[i] >= '0' && s[i] <= '9') {
        any = 1;
        v = v * 10 + (s[i] - '0');
        if (v > 300000000) {
            return -1;
        }
        i++;
    }
    while (i < len && (s[i] == ' ' || s[i] == '\t')) i++;
    if (!any || i < len) {
        return -1;
    }
    if (neg) {
        v = -v;
    }
    if (v < JM_INT_MIN || v > JM_INT_MAX) {
        return -1;
    }
    *out = (int32_t)v;
    return 0;
}

static uint32_t range_len(int32_t start, int32_t stop, int32_t step) {
    int64_t a = start, b = stop, s = step;
    if (s > 0) {
        if (a >= b) return 0;
        return (uint32_t)((b - a + s - 1) / s);
    }
    if (s < 0) {
        if (a <= b) return 0;
        int64_t sp = -s;
        return (uint32_t)((a - b + sp - 1) / sp);
    }
    return 0;
}

static int iterable_ints(JmState *st, JmValue v, int32_t *out_min, int32_t *out_max,
                         int64_t *out_sum, uint32_t *out_count) {
    int64_t sum = 0;
    int32_t mn = 0, mx = 0;
    uint32_t cnt = 0;
    if (JM_TAG(v) != JM_TAG_HEAP) {
        return jm_fail(st, E_TYPE, "这里需要一个列表或范围");
    }
    JmObj *o = jm_obj(v);
    if (o->type == JM_T_LIST) {
        JmList *l = (JmList *)o;
        for (uint32_t i = 0; i < l->len; i++) {
            if (JM_TAG(l->items[i]) != JM_TAG_INT) {
                return jm_fail(st, E_TYPE, "列表里必须是整数");
            }
            int32_t x = jm_int(l->items[i]);
            if (cnt == 0) {
                mn = mx = x;
            } else {
                if (x < mn) mn = x;
                if (x > mx) mx = x;
            }
            sum += x;
            cnt++;
        }
    } else if (o->type == JM_T_RANGE) {
        JmRange *r = (JmRange *)o;
        if (r->len == 0) {
            *out_count = 0;
            *out_sum = 0;
            *out_min = 0;
            *out_max = 0;
            return 0;
        }
        int64_t first = r->start;
        int64_t last = (int64_t)r->start + (int64_t)r->step * (int64_t)(r->len - 1);
        sum = (int64_t)r->len * (first + last) / 2;
        mn = r->step > 0 ? (int32_t)first : (int32_t)last;
        mx = r->step > 0 ? (int32_t)last : (int32_t)first;
        cnt = r->len;
    } else {
        return jm_fail(st, E_TYPE, "这里需要一个列表或范围");
    }
    *out_sum = sum;
    *out_count = cnt;
    *out_min = mn;
    *out_max = mx;
    return 0;
}

/* ---------------- 分派 ---------------- */

int jm_builtin_call(JmState *st, uint8_t id, JmValue *args, uint8_t nargs,
                    JmValue *out) {
    switch (id) {
    case B_PRINT: {
        for (uint8_t i = 0; i < nargs; i++) {
            if (i) {
                jm_port_write(" ", 1);
            }
            if (jm_repr(st, args[i], 0, port_sink, NULL, 0)) {
                return -1;
            }
        }
        jm_port_write("\n", 1);
        *out = JM_NIL;
        return 0;
    }
    case B_TEXT: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "文本() 要 1 个参数");
        }
        return to_text(st, args[0], out);
    }
    case B_INT: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "整数() 要 1 个参数");
        }
        if (JM_TAG(args[0]) == JM_TAG_INT) {
            *out = args[0];
            return 0;
        }
        if (JM_TAG(args[0]) == JM_TAG_HEAP && jm_obj(args[0])->type == JM_T_STR) {
            JmStr *s = (JmStr *)jm_obj(args[0]);
            int32_t v;
            if (parse_int_str((const char *)s->data, s->len, &v) == 0) {
                *out = jm_v_int(v);
                return 0;
            }
        }
        return jm_fail(st, E_TYPE, "整数() 只接受整数或数字文本");
    }
    case B_LEN: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "长度() 要 1 个参数");
        }
        uint32_t n = 0;
        if (JM_TAG(args[0]) != JM_TAG_HEAP) {
            return jm_fail(st, E_TYPE, "这个值没有长度");
        }
        JmObj *o = jm_obj(args[0]);
        switch (o->type) {
        case JM_T_STR:
            n = utf8_count(((JmStr *)o)->data, ((JmStr *)o)->len);
            break;
        case JM_T_LIST:
            n = ((JmList *)o)->len;
            break;
        case JM_T_DICT:
            n = ((JmDict *)o)->len;
            break;
        case JM_T_RANGE:
            n = ((JmRange *)o)->len;
            break;
        default:
            return jm_fail(st, E_TYPE, "这个值没有长度");
        }
        if ((int32_t)n > JM_INT_MAX) {
            return jm_fail(st, E_OVERFLOW, "长度超出整数范围");
        }
        *out = jm_v_int((int32_t)n);
        return 0;
    }
    case B_RANGE: {
        if (nargs < 1 || nargs > 3) {
            return jm_fail(st, E_ARGC, "范围() 要 1~3 个参数");
        }
        int32_t a[3] = {0, 0, 0};
        for (uint8_t i = 0; i < nargs; i++) {
            if (JM_TAG(args[i]) != JM_TAG_INT) {
                return jm_fail(st, E_TYPE, "范围() 只接受整数");
            }
            a[i] = jm_int(args[i]);
        }
        int32_t start = 0, stop = 0, step = 1;
        if (nargs == 1) {
            stop = a[0];
        } else if (nargs == 2) {
            start = a[0];
            stop = a[1];
        } else {
            start = a[0];
            stop = a[1];
            step = a[2];
            if (step == 0) {
                return jm_fail(st, E_TYPE, "范围() 的步长不能是 0");
            }
        }
        JmRange *r = jm_range_new_obj(start, step, range_len(start, stop, step));
        if (!r) {
            return jm_fail(st, E_OOM, "创建范围失败");
        }
        *out = jm_heap_value(r);
        return 0;
    }
    case B_TYPE: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "类型() 要 1 个参数");
        }
        const char *name;
        jm_type_name(args[0], &name);
        return jm_str_new(st, name, (uint32_t)strlen(name), out);
    }
    case B_SUM: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "总和() 要 1 个参数");
        }
        int32_t mn, mx;
        int64_t sum;
        uint32_t cnt;
        if (iterable_ints(st, args[0], &mn, &mx, &sum, &cnt)) {
            return -1;
        }
        if (sum < JM_INT_MIN || sum > JM_INT_MAX) {
            return jm_fail(st, E_OVERFLOW, "总和溢出");
        }
        *out = jm_v_int((int32_t)sum);
        return 0;
    }
    case B_MAX:
    case B_MIN: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, id == B_MAX ? "最大() 要 1 个参数"
                                                    : "最小() 要 1 个参数");
        }
        int32_t mn, mx;
        int64_t sum;
        uint32_t cnt;
        if (iterable_ints(st, args[0], &mn, &mx, &sum, &cnt)) {
            return -1;
        }
        if (cnt == 0) {
            return jm_fail(st, E_TYPE, "空序列没有最大/最小");
        }
        *out = jm_v_int(id == B_MAX ? mx : mn);
        return 0;
    }
    case B_ABS: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "绝对值() 要 1 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_INT) {
            return jm_fail(st, E_TYPE, "绝对值() 只接受整数");
        }
        int64_t v = jm_int(args[0]);
        if (v < 0) {
            v = -v;
        }
        if (v > JM_INT_MAX) {
            return jm_fail(st, E_OVERFLOW, "整数溢出");
        }
        *out = jm_v_int((int32_t)v);
        return 0;
    }
    default:
        return jm_fail(st, E_NOT_CALLABLE, "未知内建函数");
    }
}

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
#define B_DEC 10
#define B_SET 11
#define B_SET_ADD 12
#define B_SET_HAS 13
#define B_SET_DEL 14
#define B_BYTES 15
#define B_BYTES_TEXT 16
#define B_BYTES_LIST 17
#define B_BYTES_HEX 18
#define B_HEX_BYTES 19
#define B_BYTES_AT 20
#define B_STR_SUB 21
#define B_STR_FIND 22
#define B_STR_REPLACE 23
#define B_STR_SPLIT 24
#define B_STR_JOIN 25
#define B_STR_TRIM 26
#define B_STR_UPPER 27
#define B_STR_LOWER 28
#define B_LIST_APPEND 29
#define B_LIST_INSERT 30
#define B_LIST_REMOVE 31
#define B_LIST_SLICE 32
#define B_LIST_REVERSE 33
#define B_LIST_SORT 34
#define B_DICT_KEYS 35
#define B_DICT_HAS 36
#define B_DICT_MERGE 37
#define B_DICT_DEL 38
#define B_DICT_GET 39

typedef struct {
    const char *name;
    uint8_t id;
} JmBuiltinDef;

static const JmBuiltinDef BUILTINS[] = {
    {"打印", B_PRINT}, {"文本", B_TEXT}, {"整数", B_INT},   {"长度", B_LEN},
    {"范围", B_RANGE}, {"类型", B_TYPE}, {"总和", B_SUM},   {"最大", B_MAX},
    {"最小", B_MIN},   {"绝对值", B_ABS}, {"小数", B_DEC},
    {"集合", B_SET},   {"集合加入", B_SET_ADD}, {"集合有", B_SET_HAS},
    {"集合删除", B_SET_DEL},
    {"字节", B_BYTES}, {"字节文本", B_BYTES_TEXT}, {"字节列表", B_BYTES_LIST},
    {"字节十六进制", B_BYTES_HEX}, {"十六进制字节", B_HEX_BYTES},
    {"字节取", B_BYTES_AT},
    {"子串", B_STR_SUB}, {"查找", B_STR_FIND}, {"替换", B_STR_REPLACE},
    {"分割", B_STR_SPLIT}, {"连接", B_STR_JOIN}, {"去空白", B_STR_TRIM},
    {"转大写", B_STR_UPPER}, {"转小写", B_STR_LOWER},
    {"追加", B_LIST_APPEND}, {"插入", B_LIST_INSERT}, {"移除", B_LIST_REMOVE},
    {"切片", B_LIST_SLICE}, {"反转", B_LIST_REVERSE}, {"排序", B_LIST_SORT},
    {"键列表", B_DICT_KEYS}, {"包含", B_DICT_HAS}, {"合并", B_DICT_MERGE},
    {"移除键", B_DICT_DEL}, {"默认值", B_DICT_GET},
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
        /* 内建没占用的名字，交给原生函数表（Native ABI v2） */
        if (st->globals[i] == JM_UNDEF) {
            jm_native_bind_name(st, i, nm);
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

static void put_dec(JmSink sink, void *ctx, int32_t raw) {
    char buf[24];
    int n = 0;
    int64_t v = raw;
    if (v < 0) {
        buf[n++] = '-';
        v = -v;
    }
    uint32_t ip = (uint32_t)(v >> JM_DEC_FRAC_BITS);
    uint32_t frac = (uint32_t)(v & (JM_DEC_SCALE - 1));
    /* 4 位小数：足够覆盖 1/4096 精度 */
    uint32_t d = (uint32_t)(((uint64_t)frac * 10000u + JM_DEC_SCALE / 2) / JM_DEC_SCALE);
    if (d >= 10000u) {
        d -= 10000u;
        ip++;
    }

    char tmp[12];
    int t = 0;
    if (ip == 0) {
        tmp[t++] = '0';
    }
    while (ip > 0) {
        tmp[t++] = (char)('0' + (ip % 10u));
        ip /= 10u;
    }
    while (t > 0) {
        buf[n++] = tmp[--t];
    }
    buf[n++] = '.';

    char f[4];
    f[0] = (char)('0' + (d / 1000u));
    f[1] = (char)('0' + ((d / 100u) % 10u));
    f[2] = (char)('0' + ((d / 10u) % 10u));
    f[3] = (char)('0' + (d % 10u));
    int fl = 4;
    while (fl > 1 && f[fl - 1] == '0') {
        fl--;
    }
    for (int i = 0; i < fl; i++) {
        buf[n++] = f[i];
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
        case JM_T_SET: {
            JmSet *s = (JmSet *)o;
            sink(ctx, "集合{", 7);
            for (uint32_t i = 0; i < s->len; i++) {
                if (i) {
                    sink(ctx, ", ", 2);
                }
                if (jm_repr(st, s->items[i], 1, sink, ctx, depth + 1)) {
                    return -1;
                }
            }
            sink(ctx, "}", 1);
            return 0;
        }
        case JM_T_BYTES: {
            static const char HX[] = "0123456789ABCDEF";
            JmBytes *b = (JmBytes *)o;
            sink(ctx, "字节<", 7);
            for (uint32_t i = 0; i < b->len; i++) {
                char t[3];
                uint32_t n = 0;
                if (i) {
                    t[n++] = ' ';
                }
                t[n++] = HX[b->data[i] >> 4];
                t[n++] = HX[b->data[i] & 15u];
                sink(ctx, t, n);
            }
            sink(ctx, ">", 1);
            return 0;
        }
        case JM_T_DEC:
            put_dec(sink, ctx, ((JmDec *)o)->raw);
            return 0;
        case JM_T_RANGE:
            S("<范围>");
            return 0;
        case JM_T_ITER:
            S("<迭代器>");
            return 0;
        case JM_T_CLOSURE:
            S("<函数>");
            return 0;
        case JM_T_INSTANCE: {
            JmInstance *in = (JmInstance *)o;
            if (in->class_idx >= st->nclasses) {
                S("<实例>");
                return 0;
            }
            /* 类记录 10 B：[0]=类名 [1]=字段数 [4..5]=字段名数组偏移。 */
            const uint8_t *ce = st->base + st->class_start + (uint32_t)in->class_idx * 10u;
            const char *cname = (const char *)name_entry(st, ce[0]);
            sink(ctx, cname, (uint32_t)strlen(cname));
            sink(ctx, "{", 1);
            uint16_t foff = (uint16_t)(st->base[st->class_start + (uint32_t)in->class_idx * 10u + 4u] |
                                       (st->base[st->class_start + (uint32_t)in->class_idx * 10u + 5u] << 8));
            uint32_t base_off = st->class_start + foff;
            for (uint8_t k = 0; k < ce[1]; k++) {
                if (k) {
                    sink(ctx, ", ", 2);
                }
                const char *fname = (const char *)name_entry(st, st->base[base_off + k]);
                sink(ctx, fname, (uint32_t)strlen(fname));
                sink(ctx, ": ", 2);
                if (jm_repr(st, in->fields[k], 1, sink, ctx, depth + 1)) {
                    return -1;
                }
            }
            sink(ctx, "}", 1);
            return 0;
        }
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

/* 文本下标是码点下标：返回 pos 处码点的字节长度；越界返回 0。 */
static uint32_t u8_step(const uint8_t *s, uint32_t len, uint32_t pos) {
    if (pos >= len) {
        return 0;
    }
    uint8_t c = s[pos];
    uint32_t step = 1;
    if ((c & 0xE0) == 0xC0) step = 2;
    else if ((c & 0xF0) == 0xE0) step = 3;
    else if ((c & 0xF8) == 0xF0) step = 4;
    if (pos + step > len) {
        step = 1;
    }
    return step;
}

/* 码点下标 → 字节偏移（idx 已由调用方夹到 [0, 码点数]）。 */
static uint32_t u8_offset(const uint8_t *s, uint32_t len, int32_t idx) {
    uint32_t pos = 0;
    for (int32_t n = 0; n < idx && pos < len; n++) {
        pos += u8_step(s, len, pos);
    }
    return pos;
}

static JmStr *as_str(JmValue v) {
    if (JM_TAG(v) == JM_TAG_HEAP && jm_obj(v)->type == JM_T_STR) {
        return (JmStr *)jm_obj(v);
    }
    return NULL;
}

static JmDict *as_dict(JmValue v) {
    if (JM_TAG(v) == JM_TAG_HEAP && jm_obj(v)->type == JM_T_DICT) {
        return (JmDict *)jm_obj(v);
    }
    return NULL;
}

/* 按键找字典项；返回 1 并写下标。 */
static int dict_at(JmDict *d, JmValue key, uint32_t *idx) {
    for (uint32_t i = 0; i < d->len; i++) {
        if (jm_equal(d->entries[i].key, key)) {
            *idx = i;
            return 1;
        }
    }
    return 0;
}

static int is_ascii_space(uint8_t c) {
    return c == ' ' || c == '\t' || c == '\r' || c == '\n';
}

/* 切片下标：负数从末尾算，越界夹到 [0, 总数]。 */
static int32_t clamp_index(int32_t i, int32_t total) {
    if (i < 0) {
        i += total;
    }
    if (i < 0) {
        i = 0;
    }
    if (i > total) {
        i = total;
    }
    return i;
}

/* 排序比较：整数/小数按数值，文本按 UTF-8 字节序；其他类型报 E010。 */
static int sort_cmp(JmState *st, JmValue a, JmValue b, int *ok) {
    *ok = 1;
    int an = JM_TAG(a) == JM_TAG_INT;
    int bn = JM_TAG(b) == JM_TAG_INT;
    if ((an || jm_is_dec(a)) && (bn || jm_is_dec(b))) {
        int64_t ra = an ? (int64_t)jm_int(a) * JM_DEC_SCALE : (int64_t)jm_dec_raw(a);
        int64_t rb = bn ? (int64_t)jm_int(b) * JM_DEC_SCALE : (int64_t)jm_dec_raw(b);
        return ra < rb ? -1 : (ra > rb ? 1 : 0);
    }
    JmStr *sa = as_str(a);
    JmStr *sb = as_str(b);
    if (sa && sb) {
        uint32_t n = sa->len < sb->len ? sa->len : sb->len;
        int c = n ? memcmp(sa->data, sb->data, n) : 0;
        if (c) {
            return c < 0 ? -1 : 1;
        }
        return sa->len < sb->len ? -1 : (sa->len > sb->len ? 1 : 0);
    }
    *ok = 0;
    jm_fail(st, E_TYPE, "排序() 只能排整数/小数或文本，且不能混类型");
    return 0;
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

/* 把临时缓冲变成文本对象，并释放缓冲。 */
static int buf_to_str(JmState *st, JmBuf *b, JmValue *out) {
    int rc = jm_str_new(st, b->p ? (const char *)b->p : "", b->len, out);
    if (b->p) {
        jm_raw_free(b->p);
        b->p = NULL;
    }
    return rc;
}

/* 缓冲分配失败：释放缓冲并报堆耗尽。 */
static int buf_fail(JmState *st, JmBuf *b) {
    if (b->p) {
        jm_raw_free(b->p);
        b->p = NULL;
    }
    return jm_fail(st, E_OOM, "文本操作内存不足");
}

static int hex_val(char c) {
    if (c >= '0' && c <= '9') {
        return c - '0';
    }
    if (c >= 'a' && c <= 'f') {
        return c - 'a' + 10;
    }
    if (c >= 'A' && c <= 'F') {
        return c - 'A' + 10;
    }
    return -1;
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
        if (jm_is_dec(args[0])) {
            /* 向零截断 */
            *out = jm_v_int((int32_t)(jm_dec_raw(args[0]) / JM_DEC_SCALE));
            return 0;
        }
        return jm_fail(st, E_TYPE, "整数() 只接受整数、小数或数字文本");
    }
    case B_DEC: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "小数() 要 1 个参数");
        }
        int64_t raw;
        if (JM_TAG(args[0]) == JM_TAG_INT) {
            int64_t x = jm_int(args[0]);
            raw = x * JM_DEC_SCALE;
        } else if (jm_is_dec(args[0])) {
            raw = jm_dec_raw(args[0]);
        } else {
            return jm_fail(st, E_TYPE, "小数() 只接受整数或小数");
        }
        if (raw < JM_DEC_RAW_MIN || raw > JM_DEC_RAW_MAX) {
            return jm_fail(st, E_OVERFLOW, "小数超出 Q12 值域（约 ±524288）");
        }
        return jm_dec_new(st, (int32_t)raw, out);
    }
    case B_SET: {
        if (nargs != 0) {
            return jm_fail(st, E_ARGC, "集合() 不接受参数");
        }
        JmSet *s = jm_set_new_obj();
        if (!s) {
            return jm_fail(st, E_OOM, "创建集合失败");
        }
        *out = jm_heap_value(s);
        return 0;
    }
    case B_SET_ADD: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "集合加入() 要 2 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_SET) {
            return jm_fail(st, E_TYPE, "集合加入() 的第一个参数必须是集合");
        }
        jm_retain(args[1]);
        int r = jm_set_add(st, (JmSet *)jm_obj(args[0]), args[1]);
        if (r < 0) {
            jm_release(args[1]);
            return -1;
        }
        *out = r ? JM_TRUE : JM_FALSE;
        return 0;
    }
    case B_SET_HAS: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "集合有() 要 2 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_SET) {
            return jm_fail(st, E_TYPE, "集合有() 的第一个参数必须是集合");
        }
        JmSet *s = (JmSet *)jm_obj(args[0]);
        for (uint32_t i = 0; i < s->len; i++) {
            if (jm_equal(s->items[i], args[1])) {
                *out = JM_TRUE;
                return 0;
            }
        }
        *out = JM_FALSE;
        return 0;
    }
    case B_SET_DEL: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "集合删除() 要 2 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_SET) {
            return jm_fail(st, E_TYPE, "集合删除() 的第一个参数必须是集合");
        }
        JmSet *s = (JmSet *)jm_obj(args[0]);
        for (uint32_t i = 0; i < s->len; i++) {
            if (jm_equal(s->items[i], args[1])) {
                jm_release(s->items[i]);
                if (i + 1 < s->len) {
                    memmove(&s->items[i], &s->items[i + 1],
                            (size_t)(s->len - i - 1) * sizeof(JmValue));
                }
                s->len--;
                *out = JM_TRUE;
                return 0;
            }
        }
        *out = JM_FALSE;
        return 0;
    }
    case B_BYTES: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "字节() 要 1 个参数");
        }
        if (JM_TAG(args[0]) == JM_TAG_HEAP) {
            JmObj *ao = jm_obj(args[0]);
            if (ao->type == JM_T_STR) {
                JmStr *s = (JmStr *)ao;
                JmBytes *b = jm_bytes_new_obj(s->data, s->len);
                if (!b) {
                    return jm_fail(st, E_OOM, "创建字节失败");
                }
                *out = jm_heap_value(b);
                return 0;
            }
            if (ao->type == JM_T_LIST) {
                JmList *l = (JmList *)ao;
                uint8_t *tmp = NULL;
                if (l->len) {
                    tmp = (uint8_t *)jm_raw_alloc(l->len);
                    if (!tmp) {
                        return jm_fail(st, E_OOM, "创建字节失败");
                    }
                }
                for (uint32_t i = 0; i < l->len; i++) {
                    JmValue v = l->items[i];
                    if (JM_TAG(v) != JM_TAG_INT || jm_int(v) < 0 || jm_int(v) > 255) {
                        if (tmp) {
                            jm_raw_free(tmp);
                        }
                        return jm_fail(st, E_TYPE, "字节() 的列表元素必须是 0..255 的整数");
                    }
                    tmp[i] = (uint8_t)jm_int(v);
                }
                JmBytes *b = jm_bytes_new_obj(tmp, l->len);
                if (tmp) {
                    jm_raw_free(tmp);
                }
                if (!b) {
                    return jm_fail(st, E_OOM, "创建字节失败");
                }
                *out = jm_heap_value(b);
                return 0;
            }
        }
        return jm_fail(st, E_TYPE, "字节() 只接受文本或整数列表");
    }
    case B_BYTES_TEXT: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "字节文本() 要 1 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_BYTES) {
            return jm_fail(st, E_TYPE, "字节文本() 要一个字节");
        }
        JmBytes *b = (JmBytes *)jm_obj(args[0]);
        return jm_str_new(st, (const char *)b->data, b->len, out);
    }
    case B_BYTES_LIST: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "字节列表() 要 1 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_BYTES) {
            return jm_fail(st, E_TYPE, "字节列表() 要一个字节");
        }
        JmBytes *b = (JmBytes *)jm_obj(args[0]);
        JmList *l = jm_list_new_obj();
        if (!l) {
            return jm_fail(st, E_OOM, "创建列表失败");
        }
        JmValue lv = jm_heap_value(l);
        for (uint32_t i = 0; i < b->len; i++) {
            if (jm_list_push(st, l, jm_v_int((int32_t)b->data[i]))) {
                jm_release(lv);
                return -1;
            }
        }
        *out = lv;
        return 0;
    }
    case B_BYTES_HEX: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "字节十六进制() 要 1 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_BYTES) {
            return jm_fail(st, E_TYPE, "字节十六进制() 要一个字节");
        }
        static const char HX[] = "0123456789ABCDEF";
        JmBytes *b = (JmBytes *)jm_obj(args[0]);
        JmBuf buf = {st, NULL, 0, 0, 0};
        for (uint32_t i = 0; i < b->len; i++) {
            char t[3];
            uint32_t n = 0;
            if (i) {
                t[n++] = ' ';
            }
            t[n++] = HX[b->data[i] >> 4];
            t[n++] = HX[b->data[i] & 15u];
            if (buf_sink(&buf, t, n)) {
                if (buf.p) {
                    jm_raw_free(buf.p);
                }
                return -1;
            }
        }
        int rc = jm_str_new(st, buf.p ? (const char *)buf.p : "", buf.len, out);
        if (buf.p) {
            jm_raw_free(buf.p);
        }
        return rc;
    }
    case B_HEX_BYTES: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "十六进制字节() 要 1 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_STR) {
            return jm_fail(st, E_TYPE, "十六进制字节() 要一个文本");
        }
        JmStr *s = (JmStr *)jm_obj(args[0]);
        uint8_t *tmp = (uint8_t *)jm_raw_alloc(s->len / 2 + 1);
        if (!tmp) {
            return jm_fail(st, E_OOM, "创建字节失败");
        }
        uint32_t n = 0;
        int hi = -1;
        for (uint32_t i = 0; i < s->len; i++) {
            char ch = (char)s->data[i];
            if (ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r') {
                continue;
            }
            int v = hex_val(ch);
            if (v < 0) {
                jm_raw_free(tmp);
                return jm_fail(st, E_TYPE, "十六进制字节() 里出现了非十六进制字符");
            }
            if (hi < 0) {
                hi = v;
            } else {
                tmp[n++] = (uint8_t)((hi << 4) | v);
                hi = -1;
            }
        }
        if (hi >= 0) {
            jm_raw_free(tmp);
            return jm_fail(st, E_TYPE, "十六进制字节() 的位数必须是偶数");
        }
        JmBytes *b = jm_bytes_new_obj(tmp, n);
        jm_raw_free(tmp);
        if (!b) {
            return jm_fail(st, E_OOM, "创建字节失败");
        }
        *out = jm_heap_value(b);
        return 0;
    }
    case B_BYTES_AT: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "字节取() 要 2 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_BYTES) {
            return jm_fail(st, E_TYPE, "字节取() 的第一个参数必须是字节");
        }
        if (JM_TAG(args[1]) != JM_TAG_INT) {
            return jm_fail(st, E_INDEX_TYPE, "字节下标必须是整数");
        }
        JmBytes *b = (JmBytes *)jm_obj(args[0]);
        int32_t i = jm_int(args[1]);
        if (i < 0) {
            i += (int32_t)b->len;
        }
        if (i < 0 || (uint32_t)i >= b->len) {
            return jm_fail(st, E_INDEX, "字节下标越界");
        }
        *out = jm_v_int((int32_t)b->data[i]);
        return 0;
    }
    case B_STR_SUB: {
        if (nargs != 2 && nargs != 3) {
            return jm_fail(st, E_ARGC, "子串() 要 2~3 个参数");
        }
        JmStr *s = as_str(args[0]);
        if (!s) {
            return jm_fail(st, E_TYPE, "子串() 的第一个参数必须是文本");
        }
        if (JM_TAG(args[1]) != JM_TAG_INT ||
            (nargs == 3 && JM_TAG(args[2]) != JM_TAG_INT)) {
            return jm_fail(st, E_INDEX_TYPE, "子串() 的起止下标必须是整数");
        }
        int32_t total = (int32_t)utf8_count(s->data, s->len);
        int32_t a = jm_int(args[1]);
        int32_t b = nargs == 3 ? jm_int(args[2]) : total;
        if (a < 0) a += total;
        if (b < 0) b += total;
        if (a < 0) a = 0;
        if (a > total) a = total;
        if (b < 0) b = 0;
        if (b > total) b = total;
        if (b < a) b = a;
        uint32_t pa = u8_offset(s->data, s->len, a);
        uint32_t pb = u8_offset(s->data, s->len, b);
        return jm_str_new(st, (const char *)(s->data + pa), pb - pa, out);
    }
    case B_STR_FIND: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "查找() 要 2 个参数");
        }
        if (JM_TAG(args[0]) == JM_TAG_HEAP && jm_obj(args[0])->type == JM_T_LIST) {
            JmList *l = (JmList *)jm_obj(args[0]);
            for (uint32_t i = 0; i < l->len; i++) {
                if (jm_equal(l->items[i], args[1])) {
                    *out = jm_v_int((int32_t)i);
                    return 0;
                }
            }
            *out = jm_v_int(-1);
            return 0;
        }
        JmStr *h = as_str(args[0]);
        JmStr *n = as_str(args[1]);
        if (!h || !n) {
            return jm_fail(st, E_TYPE, "查找() 的第一个参数必须是文本或列表");
        }
        int32_t found = -1;
        if (n->len == 0) {
            found = 0;
        } else if (n->len <= h->len) {
            for (uint32_t i = 0; i + n->len <= h->len; i++) {
                if (memcmp(h->data + i, n->data, n->len) == 0) {
                    found = (int32_t)utf8_count(h->data, i);
                    break;
                }
            }
        }
        *out = jm_v_int(found);
        return 0;
    }
    case B_STR_REPLACE: {
        if (nargs != 3) {
            return jm_fail(st, E_ARGC, "替换() 要 3 个参数");
        }
        JmStr *h = as_str(args[0]);
        JmStr *o = as_str(args[1]);
        JmStr *nw = as_str(args[2]);
        if (!h || !o || !nw) {
            return jm_fail(st, E_TYPE, "替换() 要三个文本");
        }
        if (o->len == 0) {
            /* 空子串没有替换意义，原样返回，避免死循环。 */
            return jm_str_new(st, (const char *)h->data, h->len, out);
        }
        JmBuf buf = {st, NULL, 0, 0, 0};
        uint32_t i = 0;
        while (i < h->len) {
            if (i + o->len <= h->len && memcmp(h->data + i, o->data, o->len) == 0) {
                if (buf_sink(&buf, (const char *)nw->data, nw->len)) {
                    return buf_fail(st, &buf);
                }
                i += o->len;
            } else {
                if (buf_sink(&buf, (const char *)(h->data + i), 1)) {
                    return buf_fail(st, &buf);
                }
                i++;
            }
        }
        return buf_to_str(st, &buf, out);
    }
    case B_STR_SPLIT: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "分割() 要 2 个参数");
        }
        JmStr *h = as_str(args[0]);
        JmStr *sep = as_str(args[1]);
        if (!h || !sep) {
            return jm_fail(st, E_TYPE, "分割() 要两个文本");
        }
        JmList *l = jm_list_new_obj();
        if (!l) {
            return jm_fail(st, E_OOM, "创建列表失败");
        }
        JmValue lv = jm_heap_value(l);
        if (sep->len == 0) {
            /* 空分隔符：按码点逐个切。 */
            uint32_t pos = 0;
            while (pos < h->len) {
                uint32_t step = u8_step(h->data, h->len, pos);
                JmValue sv;
                if (jm_str_new(st, (const char *)(h->data + pos), step, &sv)) {
                    jm_release(lv);
                    return -1;
                }
                if (jm_list_push(st, l, sv)) {
                    jm_release(sv);
                    jm_release(lv);
                    return -1;
                }
                pos += step;
            }
        } else {
            uint32_t start = 0;
            uint32_t i = 0;
            while (i + sep->len <= h->len) {
                if (memcmp(h->data + i, sep->data, sep->len) == 0) {
                    JmValue sv;
                    if (jm_str_new(st, (const char *)(h->data + start), i - start, &sv)) {
                        jm_release(lv);
                        return -1;
                    }
                    if (jm_list_push(st, l, sv)) {
                        jm_release(sv);
                        jm_release(lv);
                        return -1;
                    }
                    i += sep->len;
                    start = i;
                } else {
                    i++;
                }
            }
            JmValue sv;
            if (jm_str_new(st, (const char *)(h->data + start), h->len - start, &sv)) {
                jm_release(lv);
                return -1;
            }
            if (jm_list_push(st, l, sv)) {
                jm_release(sv);
                jm_release(lv);
                return -1;
            }
        }
        *out = lv;
        return 0;
    }
    case B_STR_JOIN: {
        if (nargs != 1 && nargs != 2) {
            return jm_fail(st, E_ARGC, "连接() 要 1~2 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_LIST) {
            return jm_fail(st, E_TYPE, "连接() 的第一个参数必须是列表");
        }
        JmStr *sep = NULL;
        if (nargs == 2) {
            sep = as_str(args[1]);
            if (!sep) {
                return jm_fail(st, E_TYPE, "连接() 的分隔符必须是文本");
            }
        }
        JmList *l = (JmList *)jm_obj(args[0]);
        JmBuf buf = {st, NULL, 0, 0, 0};
        for (uint32_t i = 0; i < l->len; i++) {
            JmStr *s = as_str(l->items[i]);
            if (!s) {
                if (buf.p) {
                    jm_raw_free(buf.p);
                    buf.p = NULL;
                }
                return jm_fail(st, E_TYPE, "连接() 的列表元素必须都是文本");
            }
            if (i && sep && sep->len &&
                buf_sink(&buf, (const char *)sep->data, sep->len)) {
                return buf_fail(st, &buf);
            }
            if (s->len && buf_sink(&buf, (const char *)s->data, s->len)) {
                return buf_fail(st, &buf);
            }
        }
        return buf_to_str(st, &buf, out);
    }
    case B_STR_TRIM: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "去空白() 要 1 个参数");
        }
        JmStr *s = as_str(args[0]);
        if (!s) {
            return jm_fail(st, E_TYPE, "去空白() 要一个文本");
        }
        uint32_t a = 0;
        uint32_t b = s->len;
        while (a < b && is_ascii_space(s->data[a])) {
            a++;
        }
        while (b > a && is_ascii_space(s->data[b - 1])) {
            b--;
        }
        return jm_str_new(st, (const char *)(s->data + a), b - a, out);
    }
    case B_STR_UPPER:
    case B_STR_LOWER: {
        int upper = id == B_STR_UPPER;
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, upper ? "转大写() 要 1 个参数" : "转小写() 要 1 个参数");
        }
        JmStr *s = as_str(args[0]);
        if (!s) {
            return jm_fail(st, E_TYPE, upper ? "转大写() 要一个文本" : "转小写() 要一个文本");
        }
        JmBuf buf = {st, NULL, 0, 0, 0};
        for (uint32_t i = 0; i < s->len; i++) {
            uint8_t c = s->data[i];
            if (upper && c >= 'a' && c <= 'z') {
                c = (uint8_t)(c - 'a' + 'A');
            } else if (!upper && c >= 'A' && c <= 'Z') {
                c = (uint8_t)(c - 'A' + 'a');
            }
            if (buf_sink(&buf, (const char *)&c, 1)) {
                return buf_fail(st, &buf);
            }
        }
        return buf_to_str(st, &buf, out);
    }
    case B_LIST_APPEND: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "追加() 要 2 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_LIST) {
            return jm_fail(st, E_TYPE, "追加() 的第一个参数必须是列表");
        }
        JmList *l = (JmList *)jm_obj(args[0]);
        jm_retain(args[1]);
        if (jm_list_push(st, l, args[1])) {
            jm_release(args[1]);
            return -1;
        }
        *out = JM_NIL;
        return 0;
    }
    case B_LIST_INSERT: {
        if (nargs != 3) {
            return jm_fail(st, E_ARGC, "插入() 要 3 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_LIST) {
            return jm_fail(st, E_TYPE, "插入() 的第一个参数必须是列表");
        }
        if (JM_TAG(args[1]) != JM_TAG_INT) {
            return jm_fail(st, E_INDEX_TYPE, "插入() 的下标必须是整数");
        }
        JmList *l = (JmList *)jm_obj(args[0]);
        int32_t len = (int32_t)l->len;
        int32_t i = jm_int(args[1]);
        if (i < 0) i += len;
        if (i < 0) i = 0;
        if (i > len) i = len;
        if (jm_list_reserve(st, l, (uint32_t)len + 1)) {
            return -1;
        }
        jm_retain(args[2]);
        if (i < len) {
            memmove(&l->items[i + 1], &l->items[i],
                    (size_t)(len - i) * sizeof(JmValue));
        }
        l->items[i] = args[2];
        l->len = (uint32_t)len + 1;
        *out = JM_NIL;
        return 0;
    }
    case B_LIST_REMOVE: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "移除() 要 2 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_LIST) {
            return jm_fail(st, E_TYPE, "移除() 的第一个参数必须是列表");
        }
        if (JM_TAG(args[1]) != JM_TAG_INT) {
            return jm_fail(st, E_INDEX_TYPE, "移除() 的下标必须是整数");
        }
        JmList *l = (JmList *)jm_obj(args[0]);
        int32_t i = jm_int(args[1]);
        if (i < 0) i += (int32_t)l->len;
        if (i < 0 || (uint32_t)i >= l->len) {
            return jm_fail(st, E_INDEX, "移除() 下标越界");
        }
        JmValue v = l->items[i];
        if ((uint32_t)i + 1 < l->len) {
            memmove(&l->items[i], &l->items[i + 1],
                    (size_t)(l->len - (uint32_t)i - 1) * sizeof(JmValue));
        }
        l->len--;
        /* 所有权移交给调用方，不 release。 */
        *out = v;
        return 0;
    }
    case B_LIST_SLICE: {
        if (nargs != 2 && nargs != 3) {
            return jm_fail(st, E_ARGC, "切片() 要 2~3 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_LIST) {
            return jm_fail(st, E_TYPE, "切片() 的第一个参数必须是列表");
        }
        if (JM_TAG(args[1]) != JM_TAG_INT ||
            (nargs == 3 && JM_TAG(args[2]) != JM_TAG_INT)) {
            return jm_fail(st, E_INDEX_TYPE, "切片() 的起止下标必须是整数");
        }
        JmList *l = (JmList *)jm_obj(args[0]);
        int32_t len = (int32_t)l->len;
        int32_t a = clamp_index(jm_int(args[1]), len);
        int32_t b = nargs == 3 ? clamp_index(jm_int(args[2]), len) : len;
        if (b < a) {
            b = a;
        }
        JmList *dst = jm_list_new_obj();
        if (!dst) {
            return jm_fail(st, E_OOM, "创建列表失败");
        }
        JmValue dv = jm_heap_value(dst);
        for (int32_t i = a; i < b; i++) {
            JmValue v = l->items[i];
            jm_retain(v);
            if (jm_list_push(st, dst, v)) {
                jm_release(v);
                jm_release(dv);
                return -1;
            }
        }
        *out = dv;
        return 0;
    }
    case B_LIST_REVERSE: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "反转() 要 1 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_LIST) {
            return jm_fail(st, E_TYPE, "反转() 要一个列表");
        }
        JmList *l = (JmList *)jm_obj(args[0]);
        JmList *dst = jm_list_new_obj();
        if (!dst) {
            return jm_fail(st, E_OOM, "创建列表失败");
        }
        JmValue dv = jm_heap_value(dst);
        for (uint32_t i = l->len; i > 0; i--) {
            JmValue v = l->items[i - 1];
            jm_retain(v);
            if (jm_list_push(st, dst, v)) {
                jm_release(v);
                jm_release(dv);
                return -1;
            }
        }
        *out = dv;
        return 0;
    }
    case B_LIST_SORT: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "排序() 要 1 个参数");
        }
        if (JM_TAG(args[0]) != JM_TAG_HEAP || jm_obj(args[0])->type != JM_T_LIST) {
            return jm_fail(st, E_TYPE, "排序() 要一个列表");
        }
        JmList *l = (JmList *)jm_obj(args[0]);
        JmList *dst = jm_list_new_obj();
        if (!dst) {
            return jm_fail(st, E_OOM, "创建列表失败");
        }
        JmValue dv = jm_heap_value(dst);
        for (uint32_t i = 0; i < l->len; i++) {
            JmValue v = l->items[i];
            jm_retain(v);
            if (jm_list_push(st, dst, v)) {
                jm_release(v);
                jm_release(dv);
                return -1;
            }
        }
        /* 稳定插入排序，直接在结果列表上挪动，不再额外分配。 */
        for (uint32_t i = 1; i < dst->len; i++) {
            JmValue key = dst->items[i];
            uint32_t j = i;
            while (j > 0) {
                int ok = 0;
                int c = sort_cmp(st, dst->items[j - 1], key, &ok);
                if (!ok) {
                    jm_release(dv);
                    return -1;
                }
                if (c <= 0) {
                    break;
                }
                dst->items[j] = dst->items[j - 1];
                j--;
            }
            dst->items[j] = key;
        }
        *out = dv;
        return 0;
    }
    case B_DICT_KEYS: {
        if (nargs != 1) {
            return jm_fail(st, E_ARGC, "键列表() 要 1 个参数");
        }
        JmDict *d = as_dict(args[0]);
        if (!d) {
            return jm_fail(st, E_TYPE, "键列表() 要一个字典");
        }
        JmList *l = jm_list_new_obj();
        if (!l) {
            return jm_fail(st, E_OOM, "创建列表失败");
        }
        JmValue lv = jm_heap_value(l);
        for (uint32_t i = 0; i < d->len; i++) {
            JmValue k = d->entries[i].key;
            jm_retain(k);
            if (jm_list_push(st, l, k)) {
                jm_release(k);
                jm_release(lv);
                return -1;
            }
        }
        *out = lv;
        return 0;
    }
    case B_DICT_HAS: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "包含() 要 2 个参数");
        }
        JmDict *d = as_dict(args[0]);
        if (!d) {
            return jm_fail(st, E_TYPE, "包含() 的第一个参数必须是字典");
        }
        uint32_t at = 0;
        *out = dict_at(d, args[1], &at) ? JM_TRUE : JM_FALSE;
        return 0;
    }
    case B_DICT_GET: {
        if (nargs != 3) {
            return jm_fail(st, E_ARGC, "默认值() 要 3 个参数");
        }
        JmDict *d = as_dict(args[0]);
        if (!d) {
            return jm_fail(st, E_TYPE, "默认值() 的第一个参数必须是字典");
        }
        uint32_t at = 0;
        JmValue v = dict_at(d, args[1], &at) ? d->entries[at].val : args[2];
        jm_retain(v);
        *out = v;
        return 0;
    }
    case B_DICT_DEL: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "移除键() 要 2 个参数");
        }
        JmDict *d = as_dict(args[0]);
        if (!d) {
            return jm_fail(st, E_TYPE, "移除键() 的第一个参数必须是字典");
        }
        uint32_t at = 0;
        if (!dict_at(d, args[1], &at)) {
            *out = JM_NIL;
            return 0;
        }
        JmValue v = d->entries[at].val;
        jm_release(d->entries[at].key);
        if (at + 1 < d->len) {
            memmove(&d->entries[at], &d->entries[at + 1],
                    (size_t)(d->len - at - 1) * sizeof(JmEntry));
        }
        d->len--;
        /* 所有权移交给调用方，不 release。 */
        *out = v;
        return 0;
    }
    case B_DICT_MERGE: {
        if (nargs != 2) {
            return jm_fail(st, E_ARGC, "合并() 要 2 个参数");
        }
        JmDict *a = as_dict(args[0]);
        JmDict *b = as_dict(args[1]);
        if (!a || !b) {
            return jm_fail(st, E_TYPE, "合并() 要两个字典");
        }
        JmDict *dst = jm_dict_new_obj();
        if (!dst) {
            return jm_fail(st, E_OOM, "创建字典失败");
        }
        JmValue dv = jm_heap_value(dst);
        for (int pass = 0; pass < 2; pass++) {
            JmDict *src = pass == 0 ? a : b;
            for (uint32_t i = 0; i < src->len; i++) {
                JmValue k = src->entries[i].key;
                JmValue v = src->entries[i].val;
                jm_retain(k);
                jm_retain(v);
                if (jm_dict_set(st, dst, k, v)) {
                    jm_release(k);
                    jm_release(v);
                    jm_release(dv);
                    return -1;
                }
            }
        }
        *out = dv;
        return 0;
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
        case JM_T_SET:
            n = ((JmSet *)o)->len;
            break;
        case JM_T_BYTES:
            n = ((JmBytes *)o)->len;
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
        if (jm_is_dec(args[0])) {
            int64_t v = jm_dec_raw(args[0]);
            if (v < 0) {
                v = -v;
            }
            if (v > JM_DEC_RAW_MAX) {
                return jm_fail(st, E_OVERFLOW, "小数溢出");
            }
            return jm_dec_new(st, (int32_t)v, out);
        }
        if (JM_TAG(args[0]) != JM_TAG_INT) {
            return jm_fail(st, E_TYPE, "绝对值() 只接受整数或小数");
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
        if (id >= JM_NATIVE_ID_BASE) {
            return jm_native_call(st, id, args, nargs, out);
        }
        return jm_fail(st, E_NOT_CALLABLE, "未知内建函数");
    }
}

#include "jm_internal.h"

JmState jm_state;

int jm_fail(JmState *st, int code, const char *detail) {
    return jm_fail_aux(st, code, detail, 0);
}

int jm_fail_aux(JmState *st, int code, const char *detail, int aux) {
    if (!st->has_err) {
        st->err.code = code;
        st->err.detail = detail;
        st->err.aux = aux;
        st->has_err = 1;
    }
    return -1;
}

/* 未捕获抛出的显示缓冲：共 64 字节 = 60 字节正文 + 3 字节「…」 + 1 字节 NUL。
 * 正文放不下时截断并以「…」结尾，明确提示还有内容没显示。 */
#define UNCAUGHT_BODY 60u

/* 把抛出值渲染进 JmState 的固定显示缓冲（超长截断）。 */
typedef struct {
    char *buf;
    uint32_t cap;
    uint32_t len;
    int truncated;
} ThrowSink;

static int throw_sink(void *ctx, const char *s, uint32_t n) {
    ThrowSink *b = (ThrowSink *)ctx;
    uint32_t room = b->cap - 1u - b->len;
    if (n > room) {
        n = room;
        b->truncated = 1;
    }
    for (uint32_t i = 0; i < n; i++) {
        b->buf[b->len++] = s[i];
    }
    return 0;
}

/* 截断点可能落在 UTF-8 字符中间：退回到完整的字符边界，避免半个汉字。 */
static uint32_t utf8_trim_tail(const char *buf, uint32_t len) {
    uint32_t run = 0;
    uint32_t p = len;
    while (p > 0 && run < 3 && ((uint8_t)buf[p - 1] & 0xC0) == 0x80) {
        p--;
        run++;
    }
    if (p > 0) {
        uint8_t c = (uint8_t)buf[p - 1];
        uint32_t need = 1;
        if (c >= 0xF0) {
            need = 4;
        } else if (c >= 0xE0) {
            need = 3;
        } else if (c >= 0xC0) {
            need = 2;
        }
        if (run + 1u < need) {
            return p - 1u; /* 头字节的后续被截掉了，整字符丢掉 */
        }
    }
    return len;
}

static uint16_t rd16(const uint8_t *p) {
    return (uint16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
}
static uint32_t rd32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static const uint8_t *const_entry(JmState *st, uint8_t i) {
    return st->base + st->const_start + (uint32_t)i * 8u;
}
static const uint8_t *code_entry(JmState *st, uint8_t i) {
    return st->base + st->code_start + (uint32_t)i * 10u;
}

/* 代码对象 i 的捕获表起始偏移（捕获表紧跟在全部代码表项之后）。 */
static uint32_t cap_offset(JmState *st, uint8_t i) {
    uint32_t off = st->code_start + (uint32_t)st->ncodes * 10u;
    for (uint8_t k = 0; k < i; k++) {
        off += (uint32_t)code_entry(st, k)[8] * 2u;
    }
    return off;
}
static const char *name_at(JmState *st, uint8_t i) {
    uint16_t off = rd16(st->base + st->name_start + (uint32_t)i * 2u);
    return (const char *)(st->base + off);
}

/* 类记录：8 B，偏移都相对 class_start。
 * [0]=name_idx [1]=nfields [2]=nmethods [3]=init_code [4..5]=fields_off [6..7]=methods_off */
static const uint8_t *class_entry(JmState *st, uint8_t i) {
    return st->base + st->class_start + (uint32_t)i * 8u;
}

/* 按名字表下标查字段槽位；返回 -1 表示没有。 */
static int class_field_slot(JmState *st, uint8_t ci, uint8_t ni) {
    const uint8_t *e = class_entry(st, ci);
    uint32_t off = st->class_start + rd16(e + 4);
    for (uint8_t k = 0; k < e[1]; k++) {
        if (st->base[off + k] == ni) {
            return (int)k;
        }
    }
    return -1;
}

/* 按名字表下标查方法代码对象下标；返回 -1 表示没有。 */
static int class_method_code(JmState *st, uint8_t ci, uint8_t ni) {
    const uint8_t *e = class_entry(st, ci);
    uint32_t off = st->class_start + rd16(e + 6);
    for (uint8_t k = 0; k < e[2]; k++) {
        const uint8_t *m = st->base + off + (uint32_t)k * 2u;
        if (m[0] == ni) {
            return (int)m[1];
        }
    }
    return -1;
}

static int load_const(JmState *st, uint8_t idx, JmValue *out) {
    if (idx >= st->nconsts) {
        return jm_fail(st, E_CONST, "常量下标越界");
    }
    const uint8_t *e = const_entry(st, idx);
    switch (e[0]) {
    case K_INT: {
        int32_t v = (int32_t)rd32(e + 4);
        if (v < JM_INT_MIN || v > JM_INT_MAX) {
            return jm_fail(st, E_CONST, "整数常量超出 29 位");
        }
        *out = jm_v_int(v);
        return 0;
    }
    case K_STR: {
        if (st->const_cache[idx] != JM_UNDEF) {
            *out = st->const_cache[idx];
            jm_retain(*out);
            return 0;
        }
        uint16_t len = rd16(e + 2);
        uint32_t off = rd32(e + 4);
        if (off + len > st->len) {
            return jm_fail(st, E_FILE, "字符串常量越界");
        }
        JmValue v;
        if (jm_str_new(st, (const char *)(st->base + off), len, &v)) {
            return -1;
        }
        st->const_cache[idx] = v; /* 缓存持有一份 */
        jm_retain(v);
        *out = v;
        return 0;
    }
    case K_DEC: {
        if (st->const_cache[idx] != JM_UNDEF) {
            *out = st->const_cache[idx];
            jm_retain(*out);
            return 0;
        }
        int32_t raw = (int32_t)rd32(e + 4);
        if (raw < JM_DEC_RAW_MIN || raw > JM_DEC_RAW_MAX) {
            return jm_fail(st, E_CONST, "小数常量超出 Q12 值域");
        }
        JmValue v;
        if (jm_dec_new(st, raw, &v)) {
            return -1;
        }
        st->const_cache[idx] = v;
        jm_retain(v);
        *out = v;
        return 0;
    }
    case K_NIL:
        *out = JM_NIL;
        return 0;
    case K_TRUE:
        *out = JM_TRUE;
        return 0;
    case K_FALSE:
        *out = JM_FALSE;
        return 0;
    default:
        return jm_fail(st, E_CONST, "未知常量种类");
    }
}

int jm_truthy(JmValue v) {
    switch (JM_TAG(v)) {
    case JM_TAG_INT:
        return jm_int(v) != 0;
    case JM_TAG_NIL:
    case JM_TAG_FALSE:
    case JM_TAG_UNDEF:
        return 0;
    case JM_TAG_TRUE:
    case JM_TAG_FUNC:
    case JM_TAG_BUILTIN:
        return 1;
    case JM_TAG_HEAP: {
        JmObj *o = jm_obj(v);
        switch (o->type) {
        case JM_T_STR:
            return ((JmStr *)o)->len != 0;
        case JM_T_LIST:
            return ((JmList *)o)->len != 0;
        case JM_T_DICT:
            return ((JmDict *)o)->len != 0;
        case JM_T_RANGE:
            return ((JmRange *)o)->len != 0;
        case JM_T_DEC:
            return ((JmDec *)o)->raw != 0;
        default:
            return 1;
        }
    }
    default:
        return 1;
    }
}

int jm_equal(JmValue a, JmValue b) {
    uint8_t ta = JM_TAG(a), tb = JM_TAG(b);
    if (ta == JM_TAG_INT && tb == JM_TAG_INT) {
        return jm_int(a) == jm_int(b);
    }
    /* 整数与小数按数值比较：整数 × 4096 对齐到 Q12。 */
    if (ta == JM_TAG_INT && jm_is_dec(b)) {
        return (int64_t)jm_int(a) * JM_DEC_SCALE == (int64_t)jm_dec_raw(b);
    }
    if (tb == JM_TAG_INT && jm_is_dec(a)) {
        return (int64_t)jm_int(b) * JM_DEC_SCALE == (int64_t)jm_dec_raw(a);
    }
    if (ta != tb) {
        return 0;
    }
    switch (ta) {
    case JM_TAG_NIL:
    case JM_TAG_FALSE:
    case JM_TAG_TRUE:
    case JM_TAG_UNDEF:
        return 1;
    case JM_TAG_FUNC:
    case JM_TAG_BUILTIN:
        return JM_PAYLOAD(a) == JM_PAYLOAD(b);
    case JM_TAG_HEAP: {
        if (a == b) {
            return 1;
        }
        JmObj *oa = jm_obj(a);
        JmObj *ob = jm_obj(b);
        if (oa->type != ob->type) {
            return 0;
        }
        if (oa->type == JM_T_STR) {
            JmStr *sa = (JmStr *)oa;
            JmStr *sb = (JmStr *)ob;
            return sa->len == sb->len &&
                   (sa->len == 0 || memcmp(sa->data, sb->data, sa->len) == 0);
        }
        if (oa->type == JM_T_DEC && ob->type == JM_T_DEC) {
            return ((JmDec *)oa)->raw == ((JmDec *)ob)->raw;
        }
        if (oa->type == JM_T_LIST) {
            JmList *la = (JmList *)oa;
            JmList *lb = (JmList *)ob;
            if (la->len != lb->len) {
                return 0;
            }
            for (uint32_t i = 0; i < la->len; i++) {
                if (!jm_equal(la->items[i], lb->items[i])) {
                    return 0;
                }
            }
            return 1;
        }
        return 0;
    }
    default:
        return 0;
    }
}

int jm_type_name(JmValue v, const char **name) {
    switch (JM_TAG(v)) {
    case JM_TAG_INT:
        *name = "整数";
        return 0;
    case JM_TAG_NIL:
        *name = "空";
        return 0;
    case JM_TAG_FALSE:
    case JM_TAG_TRUE:
        *name = "布尔";
        return 0;
    case JM_TAG_FUNC:
    case JM_TAG_BUILTIN:
        *name = "函数";
        return 0;
    case JM_TAG_UNDEF:
        *name = "未定义";
        return 0;
    case JM_TAG_HEAP: {
        JmObj *o = jm_obj(v);
        switch (o->type) {
        case JM_T_STR:
            *name = "文本";
            return 0;
        case JM_T_LIST:
            *name = "列表";
            return 0;
        case JM_T_DICT:
            *name = "字典";
            return 0;
        case JM_T_RANGE:
            *name = "范围";
            return 0;
        case JM_T_ITER:
            *name = "迭代器";
            return 0;
        case JM_T_DEC:
            *name = "小数";
            return 0;
        case JM_T_CLOSURE:
            *name = "函数";
            return 0;
        case JM_T_INSTANCE:
            *name = "实例";
            return 0;
        default:
            *name = "对象";
            return 0;
        }
    }
    default:
        *name = "对象";
        return 0;
    }
}

static int int_range_ok(int64_t v) {
    return v >= JM_INT_MIN && v <= JM_INT_MAX;
}

/* 数值统一取 Q12 raw：整数 × 4096，小数直接取 raw。 */
static int num_raw(JmValue v, int64_t *raw) {
    if (JM_TAG(v) == JM_TAG_INT) {
        *raw = (int64_t)jm_int(v) * JM_DEC_SCALE;
        return 1;
    }
    if (jm_is_dec(v)) {
        *raw = (int64_t)jm_dec_raw(v);
        return 1;
    }
    return 0;
}

static int dec_raw_ok(int64_t raw) {
    return raw >= JM_DEC_RAW_MIN && raw <= JM_DEC_RAW_MAX;
}

/* 四舍五入（远离零），d 必须为正。 */
static int64_t round_div(int64_t n, int64_t d) {
    if (n >= 0) {
        return (n + d / 2) / d;
    }
    return -(((-n) + d / 2) / d);
}

static int64_t floor_div64(int64_t a, int64_t b) {
    int64_t q = a / b;
    int64_t m = a % b;
    if (m != 0 && ((m < 0) != (b < 0))) {
        q--;
    }
    return q;
}

/* 64 位乘法：先判断是否越界，避免 C 的有符号乘法回绕。
 * a、b 的绝对值都远小于 INT64_MAX；预留舍入用的半个除数。 */
static int mul_guard(int64_t a, int64_t b, int64_t *out) {
    if (a == 0 || b == 0) {
        *out = 0;
        return 1;
    }
    int64_t aa = a < 0 ? -a : a;
    int64_t bb = b < 0 ? -b : b;
    if (aa > (INT64_MAX - JM_DEC_SCALE / 2) / bb) {
        return 0;
    }
    *out = a * b;
    return 1;
}

static int dec_result(JmState *st, int64_t raw, JmValue *out) {
    if (!dec_raw_ok(raw)) {
        return jm_fail(st, E_OVERFLOW, "小数溢出（Q12 值域约 ±524288）");
    }
    return jm_dec_new(st, (int32_t)raw, out);
}

int jm_binop(JmState *st, uint8_t sub, JmValue l, JmValue r, JmValue *out) {
    if (JM_TAG(l) == JM_TAG_INT && JM_TAG(r) == JM_TAG_INT) {
        int64_t a = jm_int(l);
        int64_t b = jm_int(r);
        int64_t res = 0;
        switch (sub) {
        case BIN_ADD:
            res = a + b;
            break;
        case BIN_SUB:
            res = a - b;
            break;
        case BIN_MUL:
            res = a * b;
            break;
        case BIN_DIV:
            if (b == 0) {
                return jm_fail(st, E_ZERO_DIV, "除以零");
            }
            if (a % b != 0) {
                return jm_fail(st, E_DIV_NONINT,
                               "整数真除法结果不是整数；改用 // 或写成小数（如 3300.0）");
            }
            res = a / b;
            break;
        case BIN_FLOORDIV: {
            if (b == 0) {
                return jm_fail(st, E_ZERO_DIV, "除以零");
            }
            int64_t q = a / b;
            int64_t m = a % b;
            if (m != 0 && ((m < 0) != (b < 0))) {
                q--;
            }
            res = q;
            break;
        }
        case BIN_MOD: {
            if (b == 0) {
                return jm_fail(st, E_ZERO_MOD, "取模零");
            }
            int64_t m = a % b;
            if (m != 0 && ((m < 0) != (b < 0))) {
                m += b;
            }
            res = m;
            break;
        }
        case BIN_POW: {
            if (b < 0) {
                return jm_fail(st, E_POW, "幂指数不能是负数");
            }
            res = 1;
            for (int64_t i = 0; i < b; i++) {
                res *= a;
                if (!int_range_ok(res)) {
                    return jm_fail(st, E_OVERFLOW, "整数溢出");
                }
            }
            break;
        }
        default:
            return jm_fail(st, E_TYPE, "未知运算");
        }
        if (!int_range_ok(res)) {
            return jm_fail(st, E_OVERFLOW, "整数溢出");
        }
        *out = jm_v_int((int32_t)res);
        return 0;
    }

    /* 小数路径：任一操作数是小数，结果按 Q12 计算（/ 不再要求整除）。 */
    {
        int64_t A = 0, B = 0;
        if ((jm_is_dec(l) || jm_is_dec(r)) && num_raw(l, &A) && num_raw(r, &B)) {
            int64_t raw = 0;
            switch (sub) {
            case BIN_ADD:
                raw = A + B;
                break;
            case BIN_SUB:
                raw = A - B;
                break;
            case BIN_MUL: {
                int64_t prod;
                if (!mul_guard(A, B, &prod)) {
                    return jm_fail(st, E_OVERFLOW, "小数溢出（Q12 值域约 ±524288）");
                }
                raw = round_div(prod, JM_DEC_SCALE);
                break;
            }
            case BIN_DIV: {
                if (B == 0) {
                    return jm_fail(st, E_ZERO_DIV, "除以零");
                }
                int64_t n = A * JM_DEC_SCALE;
                int64_t d = B;
                if (d < 0) {
                    d = -d;
                    n = -n;
                }
                raw = round_div(n, d);
                break;
            }
            case BIN_FLOORDIV: {
                if (B == 0) {
                    return jm_fail(st, E_ZERO_DIV, "除以零");
                }
                int64_t q = floor_div64(A, B);
                if (!int_range_ok(q)) {
                    return jm_fail(st, E_OVERFLOW, "整数溢出");
                }
                *out = jm_v_int((int32_t)q);
                return 0;
            }
            case BIN_MOD: {
                if (B == 0) {
                    return jm_fail(st, E_ZERO_MOD, "取模零");
                }
                int64_t q = floor_div64(A, B);
                raw = A - B * q;
                break;
            }
            case BIN_POW: {
                int64_t e;
                if (JM_TAG(r) == JM_TAG_INT) {
                    e = jm_int(r);
                } else if (jm_is_dec(r) && jm_dec_raw(r) % JM_DEC_SCALE == 0) {
                    e = jm_dec_raw(r) / JM_DEC_SCALE;
                } else {
                    return jm_fail(st, E_POW, "幂指数必须是整数");
                }
                if (e < 0) {
                    return jm_fail(st, E_POW, "幂指数不能是负数");
                }
                int64_t acc = JM_DEC_SCALE; /* 1.0 */
                for (int64_t i = 0; i < e; i++) {
                    int64_t prod;
                    if (!mul_guard(acc, A, &prod)) {
                        return jm_fail(st, E_OVERFLOW, "小数溢出");
                    }
                    acc = round_div(prod, JM_DEC_SCALE);
                    if (!dec_raw_ok(acc)) {
                        return jm_fail(st, E_OVERFLOW, "小数溢出");
                    }
                }
                raw = acc;
                break;
            }
            default:
                return jm_fail(st, E_TYPE, "未知运算");
            }
            return dec_result(st, raw, out);
        }
    }

    if (sub == BIN_ADD && JM_TAG(l) == JM_TAG_HEAP && JM_TAG(r) == JM_TAG_HEAP) {
        JmObj *oa = jm_obj(l);
        JmObj *ob = jm_obj(r);
        if (oa->type == JM_T_STR && ob->type == JM_T_STR) {
            JmStr *sa = (JmStr *)oa;
            JmStr *sb = (JmStr *)ob;
            JmStr *res = jm_str_new_obj(NULL, sa->len + sb->len);
            if (!res) {
                return jm_fail(st, E_OOM, "拼接文本失败");
            }
            memcpy(res->data, sa->data, sa->len);
            memcpy(res->data + sa->len, sb->data, sb->len);
            *out = jm_heap_value(res);
            return 0;
        }
        if (oa->type == JM_T_LIST && ob->type == JM_T_LIST) {
            JmList *la = (JmList *)oa;
            JmList *lb = (JmList *)ob;
            JmList *res = jm_list_new_obj();
            if (!res) {
                return jm_fail(st, E_OOM, "拼接列表失败");
            }
            JmValue lv = jm_heap_value(res);
            if (jm_list_reserve(st, res, la->len + lb->len)) {
                jm_release(lv);
                return -1;
            }
            for (uint32_t i = 0; i < la->len; i++) {
                res->items[i] = la->items[i];
                jm_retain(la->items[i]);
            }
            for (uint32_t i = 0; i < lb->len; i++) {
                res->items[la->len + i] = lb->items[i];
                jm_retain(lb->items[i]);
            }
            res->len = la->len + lb->len;
            *out = lv;
            return 0;
        }
    }
    return jm_fail(st, E_TYPE, "类型不支持该运算");
}

int jm_compare(JmState *st, uint8_t sub, JmValue l, JmValue r, JmValue *out) {
    if (sub == CMP_EQ || sub == CMP_NE) {
        int eq = jm_equal(l, r);
        *out = jm_v_bool(sub == CMP_EQ ? eq : !eq);
        return 0;
    }
    if (JM_TAG(l) == JM_TAG_INT && JM_TAG(r) == JM_TAG_INT) {
        int32_t a = jm_int(l), b = jm_int(r);
        switch (sub) {
        case CMP_LT:
            *out = jm_v_bool(a < b);
            return 0;
        case CMP_GT:
            *out = jm_v_bool(a > b);
            return 0;
        case CMP_LE:
            *out = jm_v_bool(a <= b);
            return 0;
        case CMP_GE:
            *out = jm_v_bool(a >= b);
            return 0;
        }
    }
    {
        int64_t A = 0, B = 0;
        if ((jm_is_dec(l) || jm_is_dec(r)) && num_raw(l, &A) && num_raw(r, &B)) {
            switch (sub) {
            case CMP_LT:
                *out = jm_v_bool(A < B);
                return 0;
            case CMP_GT:
                *out = jm_v_bool(A > B);
                return 0;
            case CMP_LE:
                *out = jm_v_bool(A <= B);
                return 0;
            case CMP_GE:
                *out = jm_v_bool(A >= B);
                return 0;
            }
        }
    }
    if (JM_TAG(l) == JM_TAG_HEAP && JM_TAG(r) == JM_TAG_HEAP) {
        JmObj *oa = jm_obj(l);
        JmObj *ob = jm_obj(r);
        if (oa->type == JM_T_STR && ob->type == JM_T_STR) {
            JmStr *sa = (JmStr *)oa;
            JmStr *sb = (JmStr *)ob;
            uint32_t n = sa->len < sb->len ? sa->len : sb->len;
            int c = n ? memcmp(sa->data, sb->data, n) : 0;
            if (c == 0) {
                if (sa->len < sb->len) c = -1;
                else if (sa->len > sb->len) c = 1;
            }
            switch (sub) {
            case CMP_LT:
                *out = jm_v_bool(c < 0);
                return 0;
            case CMP_GT:
                *out = jm_v_bool(c > 0);
                return 0;
            case CMP_LE:
                *out = jm_v_bool(c <= 0);
                return 0;
            case CMP_GE:
                *out = jm_v_bool(c >= 0);
                return 0;
            }
        }
    }
    return jm_fail(st, E_TYPE, "类型不支持比较");
}

static uint32_t utf8_next(const uint8_t *s, uint32_t len, uint32_t pos) {
    if (pos >= len) {
        return pos;
    }
    uint8_t c = s[pos];
    uint32_t n = 1;
    if ((c & 0x80) == 0) {
        n = 1;
    } else if ((c & 0xE0) == 0xC0) {
        n = 2;
    } else if ((c & 0xF0) == 0xE0) {
        n = 3;
    } else if ((c & 0xF8) == 0xF0) {
        n = 4;
    }
    if (pos + n > len) {
        return len;
    }
    return pos + n;
}

static uint32_t utf8_count(const uint8_t *s, uint32_t len) {
    uint32_t i = 0, n = 0;
    while (i < len) {
        i = utf8_next(s, len, i);
        n++;
    }
    return n;
}

static int dict_find(JmDict *d, JmValue key, uint32_t *idx) {
    for (uint32_t i = 0; i < d->len; i++) {
        if (jm_equal(d->entries[i].key, key)) {
            *idx = i;
            return 1;
        }
    }
    return 0;
}

int jm_getitem(JmState *st, JmValue obj, JmValue idx, JmValue *out) {
    if (JM_TAG(obj) != JM_TAG_HEAP) {
        return jm_fail(st, E_INDEX_TYPE, "这个值不能取下标");
    }
    JmObj *o = jm_obj(obj);
    if (o->type == JM_T_LIST) {
        if (JM_TAG(idx) != JM_TAG_INT) {
            return jm_fail(st, E_INDEX_TYPE, "列表下标必须是整数");
        }
        JmList *l = (JmList *)o;
        int32_t i = jm_int(idx);
        if (i < 0) {
            i += (int32_t)l->len;
        }
        if (i < 0 || (uint32_t)i >= l->len) {
            return jm_fail(st, E_INDEX, "列表下标越界");
        }
        *out = l->items[i];
        jm_retain(*out);
        return 0;
    }
    if (o->type == JM_T_STR) {
        if (JM_TAG(idx) != JM_TAG_INT) {
            return jm_fail(st, E_INDEX_TYPE, "文本下标必须是整数");
        }
        JmStr *s = (JmStr *)o;
        int32_t i = jm_int(idx);
        if (i < 0) {
            i += (int32_t)utf8_count(s->data, s->len);
        }
        if (i < 0) {
            return jm_fail(st, E_INDEX, "文本下标越界");
        }
        uint32_t pos = 0;
        for (int32_t k = 0; k < i; k++) {
            if (pos >= s->len) {
                return jm_fail(st, E_INDEX, "文本下标越界");
            }
            pos = utf8_next(s->data, s->len, pos);
        }
        if (pos >= s->len) {
            return jm_fail(st, E_INDEX, "文本下标越界");
        }
        uint32_t end = utf8_next(s->data, s->len, pos);
        return jm_str_new(st, (const char *)(s->data + pos), end - pos, out);
    }
    if (o->type == JM_T_DICT) {
        uint32_t at;
        if (dict_find((JmDict *)o, idx, &at)) {
            *out = ((JmDict *)o)->entries[at].val;
            jm_retain(*out);
            return 0;
        }
        return jm_fail(st, E_INDEX, "字典里没有这个键");
    }
    if (o->type == JM_T_RANGE) {
        if (JM_TAG(idx) != JM_TAG_INT) {
            return jm_fail(st, E_INDEX_TYPE, "范围下标必须是整数");
        }
        JmRange *r = (JmRange *)o;
        int32_t i = jm_int(idx);
        if (i < 0) {
            i += (int32_t)r->len;
        }
        if (i < 0 || (uint32_t)i >= r->len) {
            return jm_fail(st, E_INDEX, "范围下标越界");
        }
        int64_t v = (int64_t)r->start + (int64_t)r->step * i;
        if (!int_range_ok(v)) {
            return jm_fail(st, E_OVERFLOW, "范围取值溢出");
        }
        *out = jm_v_int((int32_t)v);
        return 0;
    }
    return jm_fail(st, E_INDEX_TYPE, "这个值不能取下标");
}

int jm_setitem(JmState *st, JmValue obj, JmValue idx, JmValue val) {
    if (JM_TAG(obj) != JM_TAG_HEAP) {
        return jm_fail(st, E_INDEX_TYPE, "这个值不能写入下标");
    }
    JmObj *o = jm_obj(obj);
    if (o->type == JM_T_LIST) {
        if (JM_TAG(idx) != JM_TAG_INT) {
            return jm_fail(st, E_INDEX_TYPE, "列表下标必须是整数");
        }
        JmList *l = (JmList *)o;
        int32_t i = jm_int(idx);
        if (i < 0) {
            i += (int32_t)l->len;
        }
        if (i < 0 || (uint32_t)i >= l->len) {
            return jm_fail(st, E_INDEX, "列表下标越界");
        }
        jm_release(l->items[i]);
        l->items[i] = val;
        return 0;
    }
    if (o->type == JM_T_DICT) {
        return jm_dict_set(st, (JmDict *)o, idx, val);
    }
    return jm_fail(st, E_TYPE, "文本/范围不能写入");
}

int jm_iter_new(JmState *st, JmValue obj, JmValue *out) {
    if (JM_TAG(obj) != JM_TAG_HEAP) {
        return jm_fail(st, E_NOT_ITER, "这个值不能遍历");
    }
    JmObj *o = jm_obj(obj);
    uint32_t kind;
    switch (o->type) {
    case JM_T_LIST:
        kind = 0;
        break;
    case JM_T_STR:
        kind = 1;
        break;
    case JM_T_DICT:
        kind = 2;
        break;
    case JM_T_RANGE:
        kind = 3;
        break;
    default:
        return jm_fail(st, E_NOT_ITER, "这个值不能遍历");
    }
    JmIter *it = jm_iter_new_obj(obj, kind);
    if (!it) {
        return jm_fail(st, E_OOM, "创建迭代器失败");
    }
    *out = jm_heap_value(it);
    return 0;
}

int jm_iter_next(JmState *st, JmValue iter, int *done, JmValue *out) {
    if (JM_TAG(iter) != JM_TAG_HEAP || jm_obj(iter)->type != JM_T_ITER) {
        return jm_fail(st, E_ITER, "不是迭代器");
    }
    JmIter *it = (JmIter *)jm_obj(iter);
    JmValue c = it->container;
    JmObj *o = jm_obj(c);
    *done = 0;
    switch (it->kind) {
    case 0: {
        JmList *l = (JmList *)o;
        if (it->idx >= l->len) {
            *done = 1;
            return 0;
        }
        *out = l->items[it->idx++];
        jm_retain(*out);
        return 0;
    }
    case 1: {
        JmStr *s = (JmStr *)o;
        if (it->idx >= s->len) {
            *done = 1;
            return 0;
        }
        uint32_t end = utf8_next(s->data, s->len, it->idx);
        uint32_t start = it->idx;
        it->idx = end;
        return jm_str_new(st, (const char *)(s->data + start), end - start, out);
    }
    case 2: {
        JmDict *d = (JmDict *)o;
        if (it->idx >= d->len) {
            *done = 1;
            return 0;
        }
        *out = d->entries[it->idx++].key;
        jm_retain(*out);
        return 0;
    }
    case 3: {
        JmRange *r = (JmRange *)o;
        if (it->idx >= r->len) {
            *done = 1;
            return 0;
        }
        int64_t v = (int64_t)r->start + (int64_t)r->step * (int64_t)it->idx;
        it->idx++;
        if (!int_range_ok(v)) {
            return jm_fail(st, E_OVERFLOW, "范围取值溢出");
        }
        *out = jm_v_int((int32_t)v);
        return 0;
    }
    default:
        return jm_fail(st, E_ITER, "未知迭代器");
    }
}

int jm_execute(JmState *st) {
    JmFrame *f = &st->frames[st->fp];
    uint8_t cur_code = f->code_idx;
    const uint8_t *ce = code_entry(st, cur_code);
    const uint8_t *code = st->base + rd16(ce + 0);
    uint16_t clen = rd16(ce + 2);
    uint8_t cur_nlocals = ce[6];
    uint16_t ip = f->ip;
    uint16_t cur_ip = 0;

#define VM_FAIL(code_, detail_)                                                    \
    do {                                                                           \
        st->err.code_idx = cur_code;                                               \
        st->err.ip = cur_ip;                                                       \
        jm_fail(st, (code_), (detail_));                                           \
        goto fault;                                                                \
    } while (0)
#define PUSH_V(v_)                                                                 \
    do {                                                                           \
        if (st->sp >= JM_STACK_SLOTS) {                                            \
            VM_FAIL(E_STACK, "值栈溢出");                                           \
        }                                                                          \
        st->stack[st->sp++] = (v_);                                                \
    } while (0)
#define POP_V(dst_)                                                                \
    do {                                                                           \
        if (st->sp < 1) {                                                          \
            VM_FAIL(E_UNDERFLOW, "值栈下溢");                                       \
        }                                                                          \
        (dst_) = st->stack[--st->sp];                                              \
    } while (0)

    while (1) {
        cur_ip = ip;
        if (ip >= clen) {
            VM_FAIL(E_OPCODE, "指令流越界");
        }
        uint8_t op = code[ip++];
        switch (op) {
        case OP_HALT:
            goto done;

        case OP_CONST: {
            uint8_t a = code[ip++];
            JmValue v;
            if (load_const(st, a, &v)) {
                goto fault;
            }
            PUSH_V(v);
            break;
        }
        case OP_LOAD_LOCAL: {
            uint8_t a = code[ip++];
            if (a >= cur_nlocals) {
                VM_FAIL(E_LOCAL, "局部槽越界");
            }
            JmValue v = st->stack[f->base + a];
            if (v == JM_UNDEF) {
                VM_FAIL(E_LOCAL, "读取未初始化的局部变量");
            }
            jm_retain(v);
            PUSH_V(v);
            break;
        }
        case OP_STORE_LOCAL: {
            uint8_t a = code[ip++];
            if (a >= cur_nlocals) {
                VM_FAIL(E_LOCAL, "局部槽越界");
            }
            JmValue v;
            POP_V(v);
            jm_release(st->stack[f->base + a]);
            st->stack[f->base + a] = v;
            break;
        }
        case OP_LOAD_GLOBAL: {
            uint8_t a = code[ip++];
            if (a >= st->nnames) {
                VM_FAIL(E_NAMES, "名字下标越界");
            }
            JmValue v = st->globals[a];
            if (v == JM_UNDEF) {
                VM_FAIL(E_NAME, name_at(st, a));
            }
            jm_retain(v);
            PUSH_V(v);
            break;
        }
        case OP_STORE_GLOBAL: {
            uint8_t a = code[ip++];
            if (a >= st->nnames) {
                VM_FAIL(E_NAMES, "名字下标越界");
            }
            JmValue v;
            POP_V(v);
            jm_release(st->globals[a]);
            st->globals[a] = v;
            break;
        }
        case OP_POP: {
            JmValue v;
            POP_V(v);
            jm_release(v);
            break;
        }
        case OP_DUP: {
            if (st->sp < 1) {
                VM_FAIL(E_UNDERFLOW, "值栈下溢");
            }
            JmValue v = st->stack[st->sp - 1];
            jm_retain(v);
            PUSH_V(v);
            break;
        }
        case OP_SWAP: {
            if (st->sp < 2) {
                VM_FAIL(E_UNDERFLOW, "值栈下溢");
            }
            JmValue t = st->stack[st->sp - 1];
            st->stack[st->sp - 1] = st->stack[st->sp - 2];
            st->stack[st->sp - 2] = t;
            break;
        }
        case OP_BINOP: {
            uint8_t sub = code[ip++];
            JmValue r, l, out;
            POP_V(r);
            POP_V(l);
            if (jm_binop(st, sub, l, r, &out)) {
                jm_release(l);
                jm_release(r);
                goto fault;
            }
            jm_release(l);
            jm_release(r);
            PUSH_V(out);
            break;
        }
        case OP_NEG: {
            JmValue v;
            POP_V(v);
            if (jm_is_dec(v)) {
                int64_t n = -(int64_t)jm_dec_raw(v);
                jm_release(v);
                if (!dec_raw_ok(n)) {
                    VM_FAIL(E_OVERFLOW, "小数溢出");
                }
                JmValue out;
                if (jm_dec_new(st, (int32_t)n, &out)) {
                    VM_FAIL(E_OOM, "创建小数失败");
                }
                PUSH_V(out);
                break;
            }
            if (JM_TAG(v) != JM_TAG_INT) {
                jm_release(v);
                VM_FAIL(E_TYPE, "只有整数或小数能取负");
            }
            int64_t n = -(int64_t)jm_int(v);
            jm_release(v);
            if (!int_range_ok(n)) {
                VM_FAIL(E_OVERFLOW, "整数溢出");
            }
            PUSH_V(jm_v_int((int32_t)n));
            break;
        }
        case OP_NOT: {
            JmValue v;
            POP_V(v);
            int t = jm_truthy(v);
            jm_release(v);
            PUSH_V(jm_v_bool(!t));
            break;
        }
        case OP_CMP: {
            uint8_t sub = code[ip++];
            JmValue r, l, out;
            POP_V(r);
            POP_V(l);
            if (jm_compare(st, sub, l, r, &out)) {
                jm_release(l);
                jm_release(r);
                goto fault;
            }
            jm_release(l);
            jm_release(r);
            PUSH_V(out);
            break;
        }
        case OP_JUMP: {
            ip = rd16(code + ip);
            break;
        }
        case OP_JUMP_IF_FALSE: {
            uint16_t tgt = rd16(code + ip);
            ip += 2;
            JmValue v;
            POP_V(v);
            int t = jm_truthy(v);
            jm_release(v);
            if (!t) {
                ip = tgt;
            }
            break;
        }
        case OP_JUMP_IF_TRUE: {
            uint16_t tgt = rd16(code + ip);
            ip += 2;
            JmValue v;
            POP_V(v);
            int t = jm_truthy(v);
            jm_release(v);
            if (t) {
                ip = tgt;
            }
            break;
        }
        case OP_JUMP_IF_FALSE_OR_POP: {
            uint16_t tgt = rd16(code + ip);
            ip += 2;
            if (st->sp < 1) {
                VM_FAIL(E_UNDERFLOW, "值栈下溢");
            }
            if (!jm_truthy(st->stack[st->sp - 1])) {
                ip = tgt;
            } else {
                JmValue v;
                POP_V(v);
                jm_release(v);
            }
            break;
        }
        case OP_JUMP_IF_TRUE_OR_POP: {
            uint16_t tgt = rd16(code + ip);
            ip += 2;
            if (st->sp < 1) {
                VM_FAIL(E_UNDERFLOW, "值栈下溢");
            }
            if (jm_truthy(st->stack[st->sp - 1])) {
                ip = tgt;
            } else {
                JmValue v;
                POP_V(v);
                jm_release(v);
            }
            break;
        }
        case OP_CALL: {
            uint8_t argc = code[ip++];
            if (st->sp < (int)argc + 1) {
                VM_FAIL(E_UNDERFLOW, "调用时值栈不足");
            }
            JmValue callee = st->stack[st->sp - argc - 1];
            if (JM_TAG(callee) == JM_TAG_BUILTIN) {
                JmValue out;
                if (jm_builtin_call(st, (uint8_t)JM_PAYLOAD(callee),
                                    &st->stack[st->sp - argc], argc, &out)) {
                    goto fault;
                }
                for (uint8_t i = 0; i < argc; i++) {
                    jm_release(st->stack[st->sp - argc + i]);
                }
                st->sp -= argc;
                st->sp -= 1; /* 弹出被调用值 */
                jm_release(callee);
                PUSH_V(out);
                break;
            }
            if (JM_TAG(callee) == JM_TAG_FUNC) {
                uint8_t idx = (uint8_t)JM_PAYLOAD(callee);
                if (idx >= st->ncodes) {
                    VM_FAIL(E_CODE, "代码对象越界");
                }
                const uint8_t *nce = code_entry(st, idx);
                uint8_t nparams = nce[5];
                uint8_t nl = nce[6];
                if (argc != nparams) {
                    VM_FAIL(E_ARGC, "实参个数与形参不符");
                }
                if (st->fp + 1 >= JM_MAX_FRAMES) {
                    VM_FAIL(E_FRAMES, "调用层数太深");
                }
                f->ip = ip;
                st->fp++;
                JmFrame *nf = &st->frames[st->fp];
                nf->code_idx = idx;
                nf->ip = 0;
                nf->base = (uint16_t)(st->sp - argc);
                for (uint8_t i = nparams; i < nl; i++) {
                    PUSH_V(JM_UNDEF);
                }
                f = nf;
                cur_code = idx;
                cur_nlocals = nl;
                ce = nce;
                code = st->base + rd16(ce + 0);
                clen = rd16(ce + 2);
                ip = 0;
                break;
            }
            if (JM_TAG(callee) == JM_TAG_HEAP && jm_obj(callee)->type == JM_T_CLOSURE) {
                JmClosure *cl = (JmClosure *)jm_obj(callee);
                uint8_t idx = cl->code_idx;
                if (idx >= st->ncodes) {
                    VM_FAIL(E_CODE, "闭包代码对象越界");
                }
                const uint8_t *nce = code_entry(st, idx);
                uint8_t nparams = nce[5];
                uint8_t nl = nce[6];
                uint8_t ncap = nce[8];
                if (ncap != cl->ncap) {
                    VM_FAIL(E_CODE, "闭包捕获数与代码对象不符");
                }
                if (argc != nparams) {
                    VM_FAIL(E_ARGC, "实参个数与形参不符");
                }
                if (st->fp + 1 >= JM_MAX_FRAMES) {
                    VM_FAIL(E_FRAMES, "调用层数太深");
                }
                f->ip = ip;
                st->fp++;
                JmFrame *nf = &st->frames[st->fp];
                nf->code_idx = idx;
                nf->ip = 0;
                nf->base = (uint16_t)(st->sp - argc);
                for (uint8_t i = nparams; i < nl; i++) {
                    PUSH_V(JM_UNDEF);
                }
                /* 捕获槽在本帧入口填值（帧自己持有一份引用）。 */
                uint32_t capoff = cap_offset(st, idx);
                for (uint8_t k = 0; k < ncap; k++, capoff += 2u) {
                    uint8_t cslot = st->base[capoff + 1];
                    if (cslot < nparams || cslot >= nl) {
                        VM_FAIL(E_CODE, "闭包捕获槽越界");
                    }
                    JmValue cv = cl->caps[k];
                    jm_retain(cv);
                    st->stack[nf->base + cslot] = cv;
                }
                f = nf;
                cur_code = idx;
                cur_nlocals = nl;
                ce = nce;
                code = st->base + rd16(ce + 0);
                clen = rd16(ce + 2);
                ip = 0;
                break;
            }
            VM_FAIL(E_NOT_CALLABLE, "这个值不能调用");
        }
        case OP_RETURN: {
            JmValue ret;
            POP_V(ret);
            /* 正常路径编译器已发 TRY_END；这里兜底丢掉本帧遗留的尝试记录。 */
            while (st->tp > 0 && st->tries[st->tp - 1].frame_idx == (uint8_t)st->fp) {
                st->tp--;
            }
            if (st->fp == 0) {
                jm_release(ret);
                goto done;
            }
            for (uint16_t i = 0; i < cur_nlocals; i++) {
                jm_release(st->stack[f->base + i]);
            }
            uint16_t pos = (uint16_t)(f->base - 1);
            jm_release(st->stack[pos]);
            st->stack[pos] = ret;
            st->sp = f->base;
            st->fp--;
            f = &st->frames[st->fp];
            cur_code = f->code_idx;
            ce = code_entry(st, cur_code);
            code = st->base + rd16(ce + 0);
            clen = rd16(ce + 2);
            cur_nlocals = ce[6];
            ip = f->ip;
            break;
        }
        case OP_RETURN_METHOD: {
            JmValue ret;
            POP_V(ret);
            while (st->tp > 0 && st->tries[st->tp - 1].frame_idx == (uint8_t)st->fp) {
                st->tp--;
            }
            if (st->fp == 0) {
                jm_release(ret);
                goto done;
            }
            for (uint16_t i = 1; i < cur_nlocals; i++) {
                jm_release(st->stack[f->base + i]);
            }
            uint16_t slot0 = f->base;
            jm_release(st->stack[slot0]);
            st->stack[slot0] = ret;
            st->sp = (int)slot0 + 1;
            st->fp--;
            f = &st->frames[st->fp];
            cur_code = f->code_idx;
            ce = code_entry(st, cur_code);
            code = st->base + rd16(ce + 0);
            clen = rd16(ce + 2);
            cur_nlocals = ce[6];
            ip = f->ip;
            break;
        }
        case OP_NEW_INSTANCE: {
            uint8_t ci = code[ip++];
            if (ci >= st->nclasses) {
                VM_FAIL(E_TYPE, "类下标越界");
            }
            uint8_t nfields = class_entry(st, ci)[1];
            JmInstance *in = jm_instance_new_obj(ci, nfields);
            if (!in) {
                VM_FAIL(E_OOM, "创建实例失败");
            }
            PUSH_V(jm_heap_value(in));
            break;
        }
        case OP_GET_FIELD: {
            uint8_t ni = code[ip++];
            if (ni >= st->nnames) {
                VM_FAIL(E_NAMES, "名字下标越界");
            }
            JmValue rcv;
            POP_V(rcv);
            if (JM_TAG(rcv) != JM_TAG_HEAP || jm_obj(rcv)->type != JM_T_INSTANCE) {
                jm_release(rcv);
                VM_FAIL(E_NOT_INSTANCE, "只能对实例取字段");
            }
            JmInstance *in = (JmInstance *)jm_obj(rcv);
            if (in->class_idx >= st->nclasses) {
                jm_release(rcv);
                VM_FAIL(E_FIELD, "实例的类下标越界");
            }
            int fi = class_field_slot(st, in->class_idx, ni);
            if (fi < 0) {
                jm_release(rcv);
                VM_FAIL(E_FIELD, "实例没有这个字段");
            }
            JmValue fv = in->fields[fi];
            jm_retain(fv);
            jm_release(rcv);
            PUSH_V(fv);
            break;
        }
        case OP_SET_FIELD: {
            uint8_t ni = code[ip++];
            if (ni >= st->nnames) {
                VM_FAIL(E_NAMES, "名字下标越界");
            }
            JmValue val, rcv;
            POP_V(val);
            POP_V(rcv);
            if (JM_TAG(rcv) != JM_TAG_HEAP || jm_obj(rcv)->type != JM_T_INSTANCE) {
                jm_release(val);
                jm_release(rcv);
                VM_FAIL(E_NOT_INSTANCE, "只能对实例写字段");
            }
            JmInstance *in = (JmInstance *)jm_obj(rcv);
            if (in->class_idx >= st->nclasses) {
                jm_release(val);
                jm_release(rcv);
                VM_FAIL(E_FIELD, "实例的类下标越界");
            }
            int fi = class_field_slot(st, in->class_idx, ni);
            if (fi < 0) {
                jm_release(val);
                jm_release(rcv);
                VM_FAIL(E_FIELD, "实例没有这个字段");
            }
            jm_release(in->fields[fi]);
            in->fields[fi] = val;
            jm_release(rcv);
            break;
        }
        case OP_CALL_METHOD: {
            uint8_t ni = code[ip++];
            uint8_t argc = code[ip++];
            if (ni >= st->nnames) {
                VM_FAIL(E_NAMES, "名字下标越界");
            }
            if (st->sp < (int)argc + 1) {
                VM_FAIL(E_UNDERFLOW, "调用时值栈不足");
            }
            JmValue rcv = st->stack[st->sp - argc - 1];
            if (JM_TAG(rcv) != JM_TAG_HEAP || jm_obj(rcv)->type != JM_T_INSTANCE) {
                VM_FAIL(E_NOT_INSTANCE, "只能对实例调方法");
            }
            JmInstance *in = (JmInstance *)jm_obj(rcv);
            if (in->class_idx >= st->nclasses) {
                VM_FAIL(E_METHOD, "实例的类下标越界");
            }
            int idx = class_method_code(st, in->class_idx, ni);
            if (idx < 0) {
                VM_FAIL(E_METHOD, "实例没有这个方法");
            }
            const uint8_t *nce = code_entry(st, (uint8_t)idx);
            uint8_t nparams = nce[5];
            uint8_t nl = nce[6];
            if ((uint16_t)argc + 1u != nparams) {
                VM_FAIL(E_ARGC, "实参个数与形参不符");
            }
            if (st->fp + 1 >= JM_MAX_FRAMES) {
                VM_FAIL(E_FRAMES, "调用层数太深");
            }
            f->ip = ip;
            st->fp++;
            JmFrame *nf = &st->frames[st->fp];
            nf->code_idx = (uint8_t)idx;
            nf->ip = 0;
            nf->base = (uint16_t)(st->sp - argc - 1);
            for (uint8_t i = nparams; i < nl; i++) {
                PUSH_V(JM_UNDEF);
            }
            f = nf;
            cur_code = (uint8_t)idx;
            cur_nlocals = nl;
            ce = nce;
            code = st->base + rd16(ce + 0);
            clen = rd16(ce + 2);
            ip = 0;
            break;
        }
        case OP_CALL_INIT: {
            uint8_t ci = code[ip++];
            uint8_t argc = code[ip++];
            if (ci >= st->nclasses) {
                VM_FAIL(E_TYPE, "类下标越界");
            }
            if (st->sp < (int)argc + 1) {
                VM_FAIL(E_UNDERFLOW, "构造时值栈不足");
            }
            JmValue rcv = st->stack[st->sp - argc - 1];
            if (JM_TAG(rcv) != JM_TAG_HEAP || jm_obj(rcv)->type != JM_T_INSTANCE) {
                VM_FAIL(E_NOT_INSTANCE, "构造的目标不是实例");
            }
            JmInstance *in = (JmInstance *)jm_obj(rcv);
            if (in->class_idx != ci) {
                VM_FAIL(E_OBJ, "构造接收者与类不符");
            }
            uint8_t idx = class_entry(st, ci)[3];
            if (idx >= st->ncodes) {
                VM_FAIL(E_CODE, "初始化代码对象越界");
            }
            const uint8_t *nce = code_entry(st, idx);
            uint8_t nparams = nce[5];
            uint8_t nl = nce[6];
            if ((uint16_t)argc + 1u != nparams) {
                VM_FAIL(E_ARGC, "实参个数与初始化形参不符");
            }
            if (st->fp + 1 >= JM_MAX_FRAMES) {
                VM_FAIL(E_FRAMES, "调用层数太深");
            }
            f->ip = ip;
            st->fp++;
            JmFrame *nf = &st->frames[st->fp];
            nf->code_idx = idx;
            nf->ip = 0;
            nf->base = (uint16_t)(st->sp - argc - 1);
            for (uint8_t i = nparams; i < nl; i++) {
                PUSH_V(JM_UNDEF);
            }
            f = nf;
            cur_code = idx;
            cur_nlocals = nl;
            ce = nce;
            code = st->base + rd16(ce + 0);
            clen = rd16(ce + 2);
            ip = 0;
            break;
        }
        case OP_BUILD_LIST: {
            uint8_t n = code[ip++];
            JmValue lv;
            if (jm_list_new(st, &lv)) {
                goto fault;
            }
            JmList *l = (JmList *)jm_obj(lv);
            if (jm_list_reserve(st, l, n)) {
                jm_release(lv);
                goto fault;
            }
            for (uint8_t i = 0; i < n; i++) {
                l->items[i] = st->stack[st->sp - n + i];
            }
            l->len = n;
            st->sp -= n;
            PUSH_V(lv);
            break;
        }
        case OP_BUILD_DICT: {
            uint8_t n = code[ip++];
            JmDict *d = jm_dict_new_obj();
            if (!d) {
                VM_FAIL(E_OOM, "创建字典失败");
            }
            JmValue dv = jm_heap_value(d);
            for (uint8_t i = 0; i < n; i++) {
                JmValue k = st->stack[st->sp - 2 * n + 2 * i];
                JmValue v = st->stack[st->sp - 2 * n + 2 * i + 1];
                if (jm_dict_set(st, d, k, v)) {
                    jm_release(dv);
                    goto fault;
                }
            }
            st->sp -= 2 * n;
            PUSH_V(dv);
            break;
        }
        case OP_GET_ITEM: {
            JmValue idx, obj, out;
            POP_V(idx);
            POP_V(obj);
            int rc = jm_getitem(st, obj, idx, &out);
            jm_release(obj);
            jm_release(idx);
            if (rc) {
                goto fault;
            }
            PUSH_V(out);
            break;
        }
        case OP_SET_ITEM: {
            JmValue val, idx, obj;
            POP_V(val);
            POP_V(idx);
            POP_V(obj);
            int rc = jm_setitem(st, obj, idx, val);
            jm_release(obj);
            jm_release(idx);
            if (rc) {
                jm_release(val);
                goto fault;
            }
            break;
        }
        case OP_GET_ITER: {
            JmValue obj, itv;
            POP_V(obj);
            int rc = jm_iter_new(st, obj, &itv);
            jm_release(obj);
            if (rc) {
                goto fault;
            }
            PUSH_V(itv);
            break;
        }
        case OP_FOR_ITER: {
            uint16_t tgt = rd16(code + ip);
            ip += 2;
            if (st->sp < 1) {
                VM_FAIL(E_UNDERFLOW, "值栈下溢");
            }
            JmValue itv = st->stack[st->sp - 1];
            int done;
            JmValue elem;
            if (jm_iter_next(st, itv, &done, &elem)) {
                goto fault;
            }
            if (done) {
                st->sp--;
                jm_release(itv);
                ip = tgt;
            } else {
                PUSH_V(elem);
            }
            break;
        }
        case OP_MAKE_FUNCTION: {
            uint8_t a = code[ip++];
            if (a >= st->ncodes) {
                VM_FAIL(E_CODE, "代码对象越界");
            }
            PUSH_V(jm_v_func(a));
            break;
        }
        case OP_MAKE_CLOSURE: {
            uint8_t a = code[ip++];
            if (a >= st->ncodes) {
                VM_FAIL(E_CODE, "代码对象越界");
            }
            const uint8_t *nce = code_entry(st, a);
            uint8_t ncap = nce[8];
            if (ncap == 0) {
                PUSH_V(jm_v_func(a));
                break;
            }
            JmClosure *cl = jm_closure_new_obj(a, ncap);
            if (!cl) {
                VM_FAIL(E_OOM, "创建闭包失败");
            }
            uint32_t capoff = cap_offset(st, a);
            for (uint8_t k = 0; k < ncap; k++, capoff += 2u) {
                uint8_t pslot = st->base[capoff];
                if (pslot >= cur_nlocals) {
                    jm_release(jm_heap_value(cl));
                    VM_FAIL(E_CODE, "捕获的父槽越界");
                }
                JmValue v = st->stack[f->base + pslot];
                if (v == JM_UNDEF) {
                    jm_release(jm_heap_value(cl));
                    VM_FAIL(E_LOCAL, "捕获了尚未初始化的局部变量");
                }
                cl->caps[k] = v;
                jm_retain(v);
            }
            PUSH_V(jm_heap_value(cl));
            break;
        }
        case OP_TRY: {
            uint16_t tgt = rd16(code + ip);
            ip += 2;
            if (st->tp >= JM_MAX_TRY_DEPTH) {
                VM_FAIL(E_TRY_DEPTH, "尝试嵌套超过 8 层");
            }
            JmTry *r = &st->tries[st->tp++];
            r->frame_idx = (uint8_t)st->fp;
            r->sp = (uint16_t)st->sp;
            r->handler_ip = tgt;
            break;
        }
        case OP_TRY_END:
            if (st->tp <= 0 || st->tries[st->tp - 1].frame_idx != (uint8_t)st->fp) {
                VM_FAIL(E_TRY_STATE, "尝试栈与帧不匹配（编译器 bug）");
            }
            st->tp--;
            break;
        case OP_THROW: {
            JmValue v;
            POP_V(v);
            if (st->tp <= 0) {
                ThrowSink tb = {st->uncaught, UNCAUGHT_BODY, 0u, 0};
                st->uncaught[0] = 0;
                jm_repr(st, v, 0, throw_sink, &tb, 0);
                if (tb.truncated) {
                    /* 以「…」收尾，避免让人误以为内容只有这么多。 */
                    tb.len = utf8_trim_tail(st->uncaught, tb.len);
                    st->uncaught[tb.len++] = (char)0xE2;
                    st->uncaught[tb.len++] = (char)0x80;
                    st->uncaught[tb.len++] = (char)0xA6;
                }
                st->uncaught[tb.len] = 0;
                jm_release(v);
                VM_FAIL(E_UNCAUGHT, st->uncaught);
            }
            JmTry rec = st->tries[--st->tp];
            /* 栈展开：丢掉被展开帧（含当前帧）在 `尝试` 之后的全部栈项
             * （操作数临时值、被调用值、局部槽/捕获槽都各占一项），
             * 然后恢复到处理器所在帧。 */
            st->fp = (int)rec.frame_idx;
            while (st->sp > (int)rec.sp) {
                st->sp--;
                jm_release(st->stack[st->sp]);
            }
            PUSH_V(v);
            f = &st->frames[st->fp];
            cur_code = f->code_idx;
            ce = code_entry(st, cur_code);
            code = st->base + rd16(ce + 0);
            clen = rd16(ce + 2);
            cur_nlocals = ce[6];
            ip = rec.handler_ip;
            break;
        }
        case OP_IMPORT:
            VM_FAIL(E_IMPORT, "模块导入尚未支持");
        default:
            VM_FAIL(E_OPCODE, "未知指令");
        }
    }

done:
    st->tp = 0;
    while (st->sp > 0) {
        st->sp--;
        jm_release(st->stack[st->sp]);
    }
    return 0;

fault:
    st->err.code_idx = cur_code;
    st->err.ip = cur_ip;
    return -1;

#undef VM_FAIL
#undef PUSH_V
#undef POP_V
}

#ifndef JM_INTERNAL_H
#define JM_INTERNAL_H

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "jishim.h"
#include "jm_opcodes.h"
#include "jm_port.h"

#ifndef JM_VALUE_TYPE_DEFINED
typedef uint32_t JmValue;
#define JM_VALUE_TYPE_DEFINED 1
#endif

/* ---------------- 值：4 字节，低 3 位标签 ---------------- */
#define JM_TAG_INT 0u
#define JM_TAG_NIL 1u
#define JM_TAG_FALSE 2u
#define JM_TAG_TRUE 3u
#define JM_TAG_HEAP 4u
#define JM_TAG_FUNC 5u
#define JM_TAG_BUILTIN 6u
#define JM_TAG_UNDEF 7u

#define JM_TAG(v) ((uint8_t)((v) & 7u))
#define JM_PAYLOAD(v) ((v) >> 3)

#define JM_INT_MAX 268435455
#define JM_INT_MIN (-268435456)

#define JM_NIL ((JmValue)JM_TAG_NIL)
#define JM_TRUE ((JmValue)JM_TAG_TRUE)
#define JM_FALSE ((JmValue)JM_TAG_FALSE)
#define JM_UNDEF ((JmValue)JM_TAG_UNDEF)

static inline JmValue jm_v_int(int32_t i) {
    return (((JmValue)(uint32_t)i) << 3) | JM_TAG_INT;
}
static inline int32_t jm_int(JmValue v) {
    return ((int32_t)((int32_t)v >> 3));
}
static inline JmValue jm_v_bool(int b) { return b ? JM_TRUE : JM_FALSE; }
static inline JmValue jm_v_heap(uint32_t off) {
    return ((JmValue)off << 3) | JM_TAG_HEAP;
}
static inline JmValue jm_v_func(uint8_t idx) {
    return ((JmValue)idx << 3) | JM_TAG_FUNC;
}
static inline JmValue jm_v_builtin(uint8_t id) {
    return ((JmValue)id << 3) | JM_TAG_BUILTIN;
}

/* ---------------- 堆对象 ---------------- */
#define JM_T_STR 1u
#define JM_T_LIST 2u
#define JM_T_DICT 3u
#define JM_T_ITER 4u
#define JM_T_RANGE 5u
#define JM_T_DEC 6u
/* 闭包：代码对象下标 + 捕获槽的值（按值快照、只读）。 */
#define JM_T_CLOSURE 7u

/* 小数：Q12 定点，raw = 真实值 × 4096。
 * raw 用完整 int32（不是 29 位），故值域约 ±524288、精度 1/4096。
 * 混算时整数按 ×4096 提升到 Q12，中间量用 int64。 */
#define JM_DEC_FRAC_BITS 12
#define JM_DEC_SCALE 4096
#define JM_DEC_RAW_MAX 2147483647
#define JM_DEC_RAW_MIN (-2147483647)

typedef struct {
    uint16_t type;
    uint16_t rc;
} JmObj;

typedef struct {
    JmObj o;
    uint32_t len;
    uint8_t data[];
} JmStr;

typedef struct {
    JmObj o;
    uint32_t len;
    uint32_t cap;
    JmValue *items;
} JmList;

typedef struct {
    JmValue key;
    JmValue val;
} JmEntry;

typedef struct {
    JmObj o;
    uint32_t len;
    uint32_t cap;
    JmEntry *entries;
} JmDict;

typedef struct {
    JmObj o;
    JmValue container;
    uint32_t idx;
    uint32_t kind;
} JmIter;

typedef struct {
    JmObj o;
    int32_t start;
    int32_t step;
    uint32_t len;
} JmRange;

typedef struct {
    JmObj o;
    int32_t raw; /* Q12 原始值：真实值 × 4096 */
} JmDec;

/* 闭包对象：头部 8 字节 + caps[ncap]（每项 4 字节）。
 * caps 是创建时父帧槽的值的快照，闭包持有它们的一份引用。 */
typedef struct {
    JmObj o;
    uint8_t code_idx;
    uint8_t ncap;
    uint16_t pad;
    JmValue caps[];
} JmClosure;

extern uint8_t jm_heap[JM_HEAP_SIZE];
static inline void *jm_obj_ptr(JmValue v) {
    return (void *)(jm_heap + JM_PAYLOAD(v));
}
static inline JmObj *jm_obj(JmValue v) { return (JmObj *)jm_obj_ptr(v); }
static inline JmValue jm_heap_value(const void *p) {
    return jm_v_heap((uint32_t)((const uint8_t *)p - jm_heap));
}

static inline int jm_is_dec(JmValue v) {
    return JM_TAG(v) == JM_TAG_HEAP && jm_obj(v)->type == JM_T_DEC;
}
static inline int32_t jm_dec_raw(JmValue v) {
    return ((JmDec *)jm_obj(v))->raw;
}

/* ---------------- 错误 ---------------- */
#define E_STACK 1
#define E_UNDERFLOW 2
#define E_CONST 3
#define E_NAME 4
#define E_NAMES 5
#define E_LOCAL 6
#define E_ZERO_DIV 7
#define E_ZERO_MOD 8
#define E_OVERFLOW 9
#define E_TYPE 10
#define E_NOT_CALLABLE 11
#define E_ARGC 12
#define E_INDEX_TYPE 13
#define E_INDEX 14
#define E_NOT_ITER 15
#define E_ITER 16
#define E_OOM 17
#define E_OBJ 18
#define E_KEY 19
#define E_CODE 20
#define E_FRAMES 21
#define E_OPCODE 22
#define E_DIV_NONINT 23
#define E_POW 24
#define E_IMPORT 25
#define E_FILE 26

typedef struct {
    int code;
    const char *detail;
    int code_idx;
    int ip;
    int aux;
} JmError;

typedef struct {
    uint8_t code_idx;
    uint16_t ip;
    uint16_t base;
} JmFrame;

typedef struct {
    const uint8_t *base;
    uint32_t len;

    uint8_t nconsts;
    uint8_t nnames;
    uint8_t ncodes;
    uint8_t entry;

    uint32_t const_start;
    uint32_t name_start;
    uint32_t code_start;

    JmValue globals[JM_MAX_GLOBALS];
    JmValue stack[JM_STACK_SLOTS];
    int sp;

    JmFrame frames[JM_MAX_FRAMES];
    int fp;

    JmValue const_cache[JM_MAX_GLOBALS];

    JmError err;
    int has_err;
} JmState;

extern JmState jm_state;

int jm_fail(JmState *st, int code, const char *detail);
int jm_fail_aux(JmState *st, int code, const char *detail, int aux);

/* ---------------- 堆 ---------------- */
void jm_heap_init(void);
void *jm_raw_alloc(size_t n);
void jm_raw_free(void *p);

JmObj *jm_obj_alloc(uint16_t type, size_t payload);
JmStr *jm_str_new_obj(const char *s, uint32_t len);
JmList *jm_list_new_obj(void);
JmDict *jm_dict_new_obj(void);
JmIter *jm_iter_new_obj(JmValue container, uint32_t kind);
JmRange *jm_range_new_obj(int32_t start, int32_t step, uint32_t len);
JmDec *jm_dec_new_obj(int32_t raw);
JmClosure *jm_closure_new_obj(uint8_t code_idx, uint8_t ncap);

int jm_list_reserve(JmState *st, JmList *l, uint32_t need);
int jm_list_push(JmState *st, JmList *l, JmValue v);
int jm_dict_set(JmState *st, JmDict *d, JmValue k, JmValue v);
int jm_str_new(JmState *st, const char *s, uint32_t len, JmValue *out);
int jm_list_new(JmState *st, JmValue *out);
int jm_dec_new(JmState *st, int32_t raw, JmValue *out);

void jm_retain(JmValue v);
void jm_release(JmValue v);

/* ---------------- 语义 ---------------- */
int jm_truthy(JmValue v);
int jm_equal(JmValue a, JmValue b);
int jm_binop(JmState *st, uint8_t sub, JmValue l, JmValue r, JmValue *out);
int jm_compare(JmState *st, uint8_t sub, JmValue l, JmValue r, JmValue *out);
int jm_getitem(JmState *st, JmValue obj, JmValue idx, JmValue *out);
int jm_setitem(JmState *st, JmValue obj, JmValue idx, JmValue val);
int jm_iter_new(JmState *st, JmValue obj, JmValue *out);
int jm_iter_next(JmState *st, JmValue iter, int *done, JmValue *out);

/* ---------------- 内建 ---------------- */
int jm_builtin_call(JmState *st, uint8_t id, JmValue *args, uint8_t nargs,
                    JmValue *out);
void jm_builtin_install(JmState *st);
int jm_type_name(JmValue v, const char **name);

/* ---------------- 原生函数表（Native ABI v1，公开头见 jm_native.h） ---------------- */
#define JM_NATIVE_ID_BASE 64u   /* 内建 id 0..63；原生函数从 64 起，最多 64 个 */
int jm_native_bind_name(JmState *st, uint8_t name_idx, const char *name);
int jm_native_call(JmState *st, uint8_t id, JmValue *args, uint8_t nargs,
                   JmValue *out);

/* 输出值的显示形式；quoted=1 时文本带引号（容器内规则） */
typedef int (*JmSink)(void *ctx, const char *s, uint32_t n);
int jm_repr(JmState *st, JmValue v, int quoted, JmSink sink, void *ctx, int depth);

/* ---------------- 加载 ---------------- */
int jm_loader_init(JmState *st, const void *data, size_t len);

/* ---------------- 执行 ---------------- */
int jm_execute(JmState *st);

/* 错误文案 */
void jm_error_format(char *buf, size_t cap);

#endif

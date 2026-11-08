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
/* 实例：类表下标 + 固定字段数组（字段名只存在于 .jmb 类表里）。 */
#define JM_T_INSTANCE 8u

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

/* 实例对象：头部 8 字节 + fields[nfields]（每项 4 字节）。
 * nfields 冗余存一份，free_obj 才能不查类表就逐个 release。 */
typedef struct {
    JmObj o;
    uint8_t class_idx;
    uint8_t nfields;
    uint16_t pad;
    JmValue fields[];
} JmInstance;

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
#define E_UNCAUGHT 27
#define E_TRY_DEPTH 28
#define E_TRY_STATE 29
#define E_FIELD 35
#define E_NOT_INSTANCE 36
#define E_METHOD 37
#define E_TASK_LIMIT 38
#define E_TASK_STATE 39

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

/// 一层 `尝试` 记录：进入时的帧下标、该帧的操作数栈深度、异常处理器入口。
#define JM_MAX_TRY_DEPTH 8
typedef struct {
    uint8_t frame_idx;
    uint16_t sp;
    uint16_t handler_ip;
} JmTry;

/* ---------------- 绿色任务（VM 内协作调度） ----------------
 * root（槽位 0）沿用原来的 512 槽 / 32 帧 / 8 层 try；子任务各有一片
 * 96 槽值栈、24 帧帧栈、4 层 try。try 栈按任务独立切片，容量由
 * `JmState.tries_cap` 在 OP_TRY 处检查，互不覆盖。尺寸变化时
 * static_assert 会挡住，因为 RAM 预算按 6 B/帧、36 B/任务 算过。 */
#define JM_MAX_GREEN_TASKS 4u   /* root + 最多 3 个子任务 */
#define JM_CHILD_STACK_SLOTS 96u
#define JM_CHILD_FRAMES 24u
#define JM_CHILD_TRIES 4u

#define JM_TASK_FREE 0u
#define JM_TASK_READY 1u
#define JM_TASK_RUNNING 2u
#define JM_TASK_SLEEPING 3u
#define JM_TASK_DEAD 4u

typedef struct {
    JmValue *stack;        /* 自己的值栈切片 */
    JmFrame *frames;       /* 自己的帧栈切片 */
    JmTry *tries;          /* 自己的 try 栈切片 */
    uint16_t stack_cap;    /* 槽数 */
    uint16_t sp;
    uint8_t frames_cap;
    uint8_t fp;
    uint8_t tries_cap;
    uint8_t tp;
    uint8_t state;         /* JM_TASK_* */
    uint8_t slot;          /* 1..3；root 为 0（不对外暴露） */
    uint8_t wait_kind;     /* 0 无 / 1 队列接收 / 2 队列发送 / 3 信号量等待 */
    uint8_t wait_native;   /* 保留字段（等待的 native id 在 JmState.wait_native 里） */
    uint32_t wake_tick;    /* 延时到期 / 等待重试的下一个 tick */
    uint32_t deadline_tick;/* 绿色等待的截止 tick */
    JmValue body;          /* 任务体闭包；root 为 UNDEF */
} JmGreenTask;

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

    uint8_t nclasses;
    /* 类表起点（捕获表之后）；类记录 8 B/项，字段/方法数组紧随其后。 */
    uint32_t class_start;

    JmValue globals[JM_MAX_GLOBALS];

    /* 当前任务的三个栈切片。root 指向下面的 root_* 静态数组。 */
    JmValue *stack;
    uint16_t stack_cap;
    int sp;

    JmFrame *frames;
    uint8_t frames_cap;
    int fp;

    JmTry *tries;
    uint8_t tries_cap;
    int tp;
    /// 未捕获异常的显示缓冲（`jm_last_error` 会引用它）。
    char uncaught[64];

    JmValue const_cache[JM_MAX_GLOBALS];

    /* ---- 绿色任务：静态切片与本状态 ---- */
    JmValue root_stack[JM_STACK_SLOTS];
    JmFrame root_frames[JM_MAX_FRAMES];
    JmTry root_tries[JM_MAX_TRY_DEPTH];
    JmValue child_stack[JM_MAX_GREEN_TASKS - 1u][JM_CHILD_STACK_SLOTS];
    JmFrame child_frames[JM_MAX_GREEN_TASKS - 1u][JM_CHILD_FRAMES];
    JmTry child_tries[JM_MAX_GREEN_TASKS - 1u][JM_CHILD_TRIES];
    JmGreenTask tasks[JM_MAX_GREEN_TASKS];
    uint8_t ntask;
    uint8_t current;
    uint8_t wait_native[3];  /* 队列接收 / 队列发送 / 信号量等待 的 native id */

    JmError err;
    int has_err;
} JmState;

/* RAM 预算锁：6 B/帧、36 B/任务（见 docs/设计-语言层任务.md §2）。
 * 结构体一动，这两个断言就会在编译期挡住，避免悄悄超预算。 */
#if defined(__cplusplus) || defined(__GNUC__)
_Static_assert(sizeof(JmFrame) == 6u, "JmFrame 必须是 6 B（RAM 预算按 6 B/帧）");
_Static_assert(sizeof(JmTry) == 6u, "JmTry 必须是 6 B（RAM 预算按此计算）");
#if defined(__SIZEOF_POINTER__) && __SIZEOF_POINTER__ == 4
/* 36 B 是 MCU（32 位指针）上的尺寸；主机是 64 位指针，不能拿它锁预算。 */
_Static_assert(sizeof(JmGreenTask) == 36u, "JmGreenTask 必须是 36 B/任务");
#endif
#endif

extern JmState jm_state;

/* 值栈高水位观测（默认关）。打开后每次 PUSH_V 记录 sp 峰值，`jm_run` 开始时清零，
 * 结束时由端口打印。默认关的理由：PUSH_V 在解释器热路径上，每压一个值多一次比较
 * 和一次可写全局写；需要数据时用 -DJM_STACK_WATERMARK=1 单独构建。 */
#ifndef JM_STACK_WATERMARK
#define JM_STACK_WATERMARK 0
#endif

#if JM_STACK_WATERMARK
extern uint16_t jm_stack_hw;
#define JM_HW_NOTE(st_)                                                            \
    do {                                                                           \
        if ((uint16_t)(st_)->sp > jm_stack_hw) {                                   \
            jm_stack_hw = (uint16_t)(st_)->sp;                                     \
        }                                                                          \
    } while (0)
#else
#define JM_HW_NOTE(st_) ((void)0)
#endif

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
JmInstance *jm_instance_new_obj(uint8_t class_idx, uint8_t nfields);

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
uint8_t jm_native_find(const char *name);
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

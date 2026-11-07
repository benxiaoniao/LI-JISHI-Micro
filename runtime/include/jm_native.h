#ifndef JM_NATIVE_H
#define JM_NATIVE_H

/*
 * 砾 ↔ C 原生函数表（Native ABI v1）
 *
 * 契约正文见 docs/原生函数表.md。这里只放「必须一字不差对齐」的部分：
 * 函数签名、参数上限、值句柄的读写方式、错误码、可重入与中断规则。
 *
 * 版本规则：
 *   - 只增表项、不改签名 → 不升版本
 *   - 改签名 / 改值编码 / 改错误码含义 → 必须升 JM_NATIVE_ABI_VERSION，
 *     并与语言卡的 abi_version 一起发布
 */

#include <stddef.h>
#include <stdint.h>

#define JM_NATIVE_ABI_VERSION 1u

/* 值句柄：4 字节不透明。原生代码不要假设它的位布局，只用下面的读写函数。 */
#ifndef JM_VALUE_TYPE_DEFINED
typedef uint32_t JmValue;
#define JM_VALUE_TYPE_DEFINED 1
#endif

/* 运行状态句柄：不透明，仅回传给 jm_native_fail / jm_value_make_text 使用。 */
typedef void *JmStateRef;

/* 一次调用最多几个参数（多于此由编译器侧限制，不靠原生层兜） */
#define JM_NATIVE_MAX_ARGS 8u

/* 调用成功返回值 */
#define JM_NATIVE_OK 0

/* 原生错误码：占用运行时错误码 30–39 段，避免与 E001–E026 冲突 */
#define JM_NE_PARAM  30   /* 参数取值非法（引脚号、通道号等） */
#define JM_NE_TYPE   31   /* 参数类型不对 */
#define JM_NE_HW     32   /* 硬件操作失败（总线/外设无响应） */
#define JM_NE_STATE  33   /* 当前状态不允许（未初始化、已在用） */
#define JM_NE_NOMEM  34   /* 分配失败 */

/* 值类别（只读判断用） */
#define JM_VK_INT   0
#define JM_VK_BOOL  1
#define JM_VK_NIL   2
#define JM_VK_TEXT  3
#define JM_VK_OTHER 4

/*
 * 原生函数签名。
 *   st     ：运行状态，只用来报错或创建文本，不要保存
 *   args   ：参数数组，按源码实参顺序（左→右），**借用**，不要 retain / 不要存指针
 *   nargs  ：实参个数，必在 [min_args, max_args] 内（运行时已检查）
 *   out    ：返回值写在这里；成功时必须写
 * 返回 0 表示成功；返回负数表示失败，且必须先用 jm_native_fail 写明原因。
 */
typedef int (*JmNativeFn)(JmStateRef st, const JmValue *args, uint8_t nargs,
                          JmValue *out);

/*
 * 表项。name 必须在 .jmb 的名字表里出现才会被绑定；
 * 绑定失败就是运行时的「名字未定义」错误，不会静默忽略。
 */
typedef struct {
    const char *name;
    JmNativeFn fn;
    uint8_t min_args;
    uint8_t max_args;
    uint8_t flags;      /* 保留：v1 必须填 0 */
    uint8_t reserved;   /* 保留：v1 必须填 0 */
} JmNativeEntry;

/* ============ 登记与查询 ============ */

/*
 * 登记一张原生函数表。返回 0 成功，-1 失败（表太多 / 表项太多 / 名字重复）。
 * 可在 jm_run 之前任意次调用；同名表项以先登记者为准。
 */
int jm_native_register(const JmNativeEntry *entries, uint8_t count);

/* 已登记的 ABI 版本（用于包管理器做兼容判断） */
unsigned jm_native_abi_version(void);

/* ============ 值的读写 ============ */

int      jm_value_kind(JmValue v);
int      jm_value_to_int(JmValue v, int32_t *out);              /* 0 成功，-1 类型不符 */
int32_t  jm_value_get_int(JmValue v);                            /* 调用方先确认 kind */

/* 文本：借用，返回 0 成功；s 与 len 在本次调用期间有效，不要保存 */
int      jm_value_text(JmValue v, const char **s, uint32_t *len);

JmValue  jm_value_int(int32_t v);
JmValue  jm_value_bool(int b);
JmValue  jm_value_nil(void);

/* 新建文本（引用计数 1，所有权交给运行时）。失败返回 -1 且 out 不变。 */
int      jm_value_make_text(JmStateRef st, const char *s, uint32_t len, JmValue *out);

/* 仅当你要把借用值带出本次调用时才需要：retain 后由运行时释放 */
void     jm_value_retain(JmValue v);
void     jm_value_release(JmValue v);

/* ============ 报错 ============ */

/* 记录错误并返回 -1，便于 `return jm_native_fail(st, JM_NE_PARAM, "引脚号越界");` */
int jm_native_fail(JmStateRef st, int code, const char *detail);

#endif

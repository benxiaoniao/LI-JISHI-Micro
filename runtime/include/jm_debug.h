#ifndef JM_DEBUG_H
#define JM_DEBUG_H

#include <stddef.h>
#include <stdint.h>

/* 调试内省接口：只在 `JM_DEBUG_HOOKS` 打开时编译。
 * 固件默认不开，整个 jm_debug.c 不进构建，代价 0 B。
 * 接口全部只读，工作在 jm_state 单例上。 */

#ifndef JM_DEBUG_HOOKS
#define JM_DEBUG_HOOKS 0
#endif

#ifndef JM_VALUE_TYPE_DEFINED
typedef uint32_t JmValue;
#define JM_VALUE_TYPE_DEFINED 1
#endif

#if JM_DEBUG_HOOKS

/* 任务视图：root 恒为 0，共 1..4 个绿色任务。 */
int jm_dbg_task_count(void);
int jm_dbg_current_task(void);
int jm_dbg_task_state(int task); /* 0=FREE 1=READY 2=RUNNING 3=SLEEPING 4=DEAD */

/* 调用栈：由内到外，返回实际填充帧数（task 无效返回 -1）。 */
typedef struct {
    uint8_t code;    /* 代码对象下标 */
    uint16_t ip;     /* 该帧当前指令偏移 */
    uint16_t base;   /* 局部槽基址（值栈下标） */
    uint8_t nlocals; /* 该代码对象的局部槽数 */
} JmDbgFrame;

int jm_dbg_frames(int task, JmDbgFrame *out, int max);

/* 值读取：borrowed，不 retain；调用方不得跨暂停点持有。
 * frame 用 jm_dbg_frames 的下标（0 = 最内层）。 */
int jm_dbg_local(int task, int frame, uint8_t slot, JmValue *out);
/* 指令钩子内读当前执行帧的局部（tick 时 tasks[].fp 尚未保存，必须用这个）。 */
int jm_dbg_local_current(uint8_t slot, JmValue *out);
int jm_dbg_global(const char *name, JmValue *out);
int jm_dbg_code_name(uint8_t code, const char **out);

/* 值 → 文本：复用 jm_repr，写进 buf（NUL 结尾）。 */
int jm_dbg_value_text(JmValue v, char *buf, size_t cap);

/* 条件断点求值用的最小类型视图（宿主不碰 NaN-box 细节）。 */
enum {
    JM_DBG_KIND_OTHER = 0,
    JM_DBG_KIND_INT = 1,
    JM_DBG_KIND_TEXT = 2,
    JM_DBG_KIND_BOOL = 3
};
int jm_dbg_value_kind(JmValue v);
int32_t jm_dbg_value_int(JmValue v); /* 仅 KIND_INT 有效 */
int jm_dbg_value_bool(JmValue v);    /* 仅 KIND_BOOL 有效，真=1 假=0 */

/* 暂停/恢复：jm_execute 命中钩子返回 1 后，宿主可反复继续。
 * 返回与 jm_run 同口径：0=正常结束，1=再次暂停，-1=故障。 */
int jm_dbg_resume(void);
int jm_dbg_paused(void);

/* 运行时在暂停出口调用（记录被暂停的任务），宿主一般不直接调。 */
void jm_dbg_mark_paused(uint8_t task);

/* 每指令钩子，由端口（runtime/port/host/jm_main.c）实现。
 * 返回 0=继续，非 0=暂停（运行时已保存现场）。 */
int jm_host_dbg_tick(uint8_t code, uint16_t ip, uint8_t fp);

#endif /* JM_DEBUG_HOOKS */

#endif /* JM_DEBUG_H */

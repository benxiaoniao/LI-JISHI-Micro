#ifndef JMK_H
#define JMK_H

#include <stdint.h>

/*
 * 砾微内核（JMK）最小任务系统
 * 目标：STM32F103C8T6 上的抢占式固定优先级调度，后续可长成类 FreeRTOS 内核。
 */

#define JMK_MAX_TASKS        6      /* 用户任务槽位数（不含空闲任务） */
#define JMK_PRIORITIES       4      /* 0 最低（空闲任务）… 3 最高 */
#define JMK_TASK_STACK_BYTES 512    /* 默认任务栈（字节），8 字节对齐 */
#define JMK_IDLE_STACK_BYTES 256    /* 空闲任务栈 */
#define JMK_TICK_HZ          1000   /* SysTick 1 ms */

/*
 * 任务栈来源（二选一）：
 *   JMK_USE_STATIC_STACKS = 0（默认）：从内核独立栈池按 8 字节粒度动态分配
 *   JMK_USE_STATIC_STACKS = 1        ：沿用 6 × 512 B 静态数组（兼容路径）
 * 编译时使用 -DJMK_USE_STATIC_STACKS=1 切换。
 */
#ifndef JMK_USE_STATIC_STACKS
#define JMK_USE_STATIC_STACKS 0
#endif

/*
 * 独立栈池大小（字节，必须是 8 的倍数）。
 * 与砾运行时堆（JM_HEAP_SIZE）完全分离，互不挤占。
 * 3200 B 足够「6 个默认 512 B 任务（含 8 B 块头）」或少量大栈任务。
 */
#ifndef JMK_STACK_POOL_BYTES
#define JMK_STACK_POOL_BYTES 3200
#endif

/* 单次分配的最小栈（字节），防止过小栈立即溢出 */
#define JMK_STACK_MIN_BYTES  64

typedef void (*JmkTaskFn)(void *arg);

typedef struct JmkTcb {
    uint32_t *sp;              /* 保存的栈指针，必须是第 0 个字段（汇编按偏移 0 访问） */
    uint32_t wake_tick;        /* 阻塞任务的唤醒时刻 */
    struct JmkTcb *next;       /* 就绪队列链 */
    JmkTaskFn fn;              /* 入口 */
    void *arg;                 /* 入口参数 */
    uint32_t *stack_base;      /* 栈底（低地址） */
    uint32_t stack_words;      /* 栈总字数 */
    uint8_t prio;              /* 0..JMK_PRIORITIES-1；0xFF 表示空闲任务 */
    uint8_t state;
    uint8_t id;
    const char *name;
} JmkTcb;

/* 创建任务：成功返回槽位号（>=0），失败返回 -1。prio 越高越优先。 */
int jmk_task_create(JmkTaskFn fn, void *arg, uint8_t prio, const char *name);

/*
 * 创建任务并指定栈大小（字节，向上取整到 8 字节）。
 * 动态模式下从独立栈池分配；静态模式下只接受 <= JMK_TASK_STACK_BYTES 的请求。
 * 栈池不足时返回 -1，且不占用任务槽位。
 */
int jmk_task_create_ex(JmkTaskFn fn, void *arg, uint8_t prio,
                       const char *name, uint32_t stack_bytes);

/* 启动调度器：创建空闲任务、选最高优先级任务并切换过去；不返回。 */
void jmk_start(void) __attribute__((noreturn));

/* 延时（毫秒，基于 1 ms tick），期间让出 CPU。 */
void jmk_delay(uint32_t ms);

/* 主动让出一次 CPU（同优先级轮转）。 */
void jmk_yield(void);

/* SysTick 中断里调用；由端口层负责。 */
void jmk_tick(void);

/* 内核 tick 计数（1 ms 递增）。 */
uint32_t jmk_ticks(void);

/* 按槽位取任务（NULL 表示空槽）。 */
JmkTcb *jmk_task_at(uint8_t idx);

/* 栈峰值用量（字节），用 0xA5 哨兵扫描得出。 */
uint32_t jmk_stack_used(const JmkTcb *t);

/* 删除任务（可删自己）。返回 0 成功，-1 无效。 */
int jmk_task_delete(JmkTcb *t);

/* 栈池统计（动态模式为真实池数据；静态模式按固定槽位估算）。 */
uint32_t jmk_stack_pool_bytes(void);    /* 池总容量 */
uint32_t jmk_stack_pool_used(void);     /* 已分配（含 8 B 块头） */
uint32_t jmk_stack_pool_free(void);     /* 空闲总量（含块头） */
uint32_t jmk_stack_pool_largest(void);  /* 最大连续空闲块 */

/* ============================================================
 * 任务间通信（信号量 / 消息队列）
 *
 * 通用约定：
 *   - 返回 0 表示成功，负值为错误码；与创建/删除任务的 0 / -1 风格一致。
 *   - timeout_ms = 0 表示不等待（JMK_NO_WAIT），立即返回。
 *   - timeout_ms = JMK_WAIT_FOREVER 表示一直等，直到被给出/有数据。
 *   - 等待队列按优先级排序，同优先级按到达顺序（FIFO）。
 *   - 只有任务能“等”；中断里只允许“给出/发送”（*_from_isr 版本）。
 *   - 信号量不记录持有者，因此不做优先级继承（见 docs/调度模型.md）。
 * ============================================================ */

#define JMK_OK          0
#define JMK_E_TIMEOUT  (-1)   /* 等待超时 */
#define JMK_E_PARAM    (-2)   /* 参数非法 */
#define JMK_E_STATE    (-3)   /* 当前上下文不允许该操作 */
#define JMK_E_FULL     (-4)   /* 信号量计数已满 / 队列已满 */

#define JMK_NO_WAIT      0u
#define JMK_WAIT_FOREVER 0xFFFFFFFFu

/*
 * 信号量控制块（12 B，由调用方定义，可静态分配）。
 * 二值信号量：max = 1；计数信号量：max 为计数上限。
 * 计数在 0..max 之间，给出时若无人等待则 +1，超出上限返回 JMK_E_FULL。
 */
typedef struct JmkSem {
    int32_t count;
    int32_t max;
    struct JmkTcb *wait_head;
} JmkSem;

int jmk_sem_init(JmkSem *s, int32_t initial, int32_t max);
int jmk_sem_wait(JmkSem *s, uint32_t timeout_ms);
int jmk_sem_post(JmkSem *s);

/* 中断里给出信号量：不阻塞、不调度，只在需要时挂起 PendSV。
 * woken 非空时写入 1 表示“唤醒了更高优先级任务，返回后应尽快切换”。 */
int jmk_sem_post_from_isr(JmkSem *s, uint8_t *woken);

/*
 * 消息队列控制块（32 B，由调用方定义）。
 * 槽位内存 buf 完全由调用方提供：内核不预留任何队列 RAM。
 * item_size 必须是 4 的倍数（保证槽位对齐）；capacity 为槽数。
 */
typedef struct JmkQueue {
    uint8_t *buf;
    uint32_t item_size;
    uint32_t capacity;
    uint32_t head;          /* 读位置 */
    uint32_t tail;          /* 写位置 */
    uint32_t count;
    struct JmkTcb *wait_recv;
    struct JmkTcb *wait_send;
} JmkQueue;

int jmk_queue_init(JmkQueue *q, void *buf, uint32_t item_size, uint32_t capacity);
/* 销毁队列：摘下所有等待者（按“未成功” -1 唤醒），清空控制块；槽位缓冲由调用方保留。 */
int jmk_queue_destroy(JmkQueue *q);
int jmk_queue_send(JmkQueue *q, const void *item, uint32_t timeout_ms);
int jmk_queue_recv(JmkQueue *q, void *item, uint32_t timeout_ms);
int jmk_queue_send_from_isr(JmkQueue *q, const void *item, uint8_t *woken);

/* 当前队列内消息条数（只读快照，调试用）。 */
uint32_t jmk_queue_count(const JmkQueue *q);

/* 供汇编使用的内部符号。 */
extern JmkTcb *volatile jmk_current;
void jmk_schedule(void);

#endif

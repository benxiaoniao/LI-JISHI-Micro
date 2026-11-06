#ifndef JMK_H
#define JMK_H

#include <stdint.h>

/*
 * 砾微内核（JMK）最小任务系统
 * 目标：STM32F103C8T6 上的抢占式固定优先级调度，后续可长成类 FreeRTOS 内核。
 */

#define JMK_MAX_TASKS        6      /* 用户任务槽位数（不含空闲任务） */
#define JMK_PRIORITIES       4      /* 0 最低（空闲任务）… 3 最高 */
#define JMK_TASK_STACK_BYTES 512    /* 每个用户任务栈（字节），8 字节对齐 */
#define JMK_IDLE_STACK_BYTES 256    /* 空闲任务栈 */
#define JMK_TICK_HZ          1000   /* SysTick 1 ms */

typedef void (*JmkTaskFn)(void *arg);

typedef struct JmkTcb {
    uint32_t *sp;              /* 保存的栈指针，必须是第 0 个字段（汇编按偏移 0 访问） */
    uint32_t wake_tick;        /* 阻塞任务的唤醒时刻 */
    struct JmkTcb *next;       /* 就绪队列链 */
    JmkTaskFn fn;              /* 入口 */
    void *arg;                 /* 入口参数 */
    uint32_t *stack_base;      /* 栈底（低地址） */
    uint32_t stack_words;      /* 栈总字数 */
    uint8_t prio;              /* 0..JMK_PRIORITIES-1 */
    uint8_t state;
    uint8_t id;
    const char *name;
} JmkTcb;

/* 创建任务：成功返回槽位号（>=0），失败返回 -1。prio 越高越优先。 */
int jmk_task_create(JmkTaskFn fn, void *arg, uint8_t prio, const char *name);

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

/* 供汇编使用的内部符号。 */
extern JmkTcb *volatile jmk_current;
void jmk_schedule(void);

#endif

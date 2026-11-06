#include <stdint.h>
#include "jmk.h"

/*
 * 砾微内核（JMK）实现要点：
 *   - 抢占式固定优先级 + 同优先级轮转（就绪队列 FIFO）
 *   - 上下文切换在 PendSV 里做：保存 R4-R11 到任务栈，再交给 jmk_schedule
 *   - SysTick 只负责 tick、唤醒到期任务、按需挂起 PendSV
 *   - 临界区用 PRIMASK 关中断，临界区都很短（链表操作）
 */

#define SCB_ICSR   (*(volatile uint32_t *)0xE000ED04u)
#define SCB_SHPR3  (*(volatile uint32_t *)0xE000ED20u)

#define PENDSVSET  (1u << 28)
#define FILL_WORD  0xA5A5A5A5u

/* 任务状态 */
#define ST_FREE    0u
#define ST_READY   1u
#define ST_RUNNING 2u
#define ST_BLOCKED 3u
#define ST_DEAD    4u

JmkTcb *volatile jmk_current;

static JmkTcb jmk_tasks[JMK_MAX_TASKS];
static uint32_t jmk_stacks[JMK_MAX_TASKS][JMK_TASK_STACK_BYTES / 4u]
    __attribute__((aligned(8)));
static uint32_t jmk_idle_stack[JMK_IDLE_STACK_BYTES / 4u]
    __attribute__((aligned(8)));
static JmkTcb jmk_idle_tcb;

static JmkTcb *ready_head[JMK_PRIORITIES];
static JmkTcb *ready_tail[JMK_PRIORITIES];
static uint32_t ready_mask;
static volatile uint32_t jmk_tick_count;
static uint8_t jmk_started;

extern void jmk_first_switch(void) __attribute__((noreturn));
void jmk_task_exit(void) __attribute__((noreturn));

/* ---- 临界区：保存/恢复 PRIMASK ---- */
static uint32_t crit_enter(void)
{
    uint32_t primask;
    __asm volatile ("mrs %0, primask" : "=r" (primask));
    __asm volatile ("cpsid i" ::: "memory");
    return primask;
}

static void crit_exit(uint32_t primask)
{
    __asm volatile ("msr primask, %0" :: "r" (primask) : "memory");
}

/* ---- 就绪队列 ---- */
static void ready_push(JmkTcb *t)
{
    t->next = 0;
    if (ready_tail[t->prio]) {
        ready_tail[t->prio]->next = t;
    } else {
        ready_head[t->prio] = t;
    }
    ready_tail[t->prio] = t;
    ready_mask |= (1u << t->prio);
}

static JmkTcb *ready_pop(uint8_t prio)
{
    JmkTcb *t = ready_head[prio];
    if (!t) {
        return 0;
    }
    ready_head[prio] = t->next;
    if (!ready_head[prio]) {
        ready_tail[prio] = 0;
        ready_mask &= ~(1u << prio);
    }
    t->next = 0;
    return t;
}

static void ready_remove(JmkTcb *t)
{
    uint8_t p = t->prio;
    JmkTcb *prev = 0;
    JmkTcb *cur = ready_head[p];

    while (cur) {
        if (cur == t) {
            if (prev) {
                prev->next = cur->next;
            } else {
                ready_head[p] = cur->next;
            }
            if (ready_tail[p] == cur) {
                ready_tail[p] = prev;
            }
            if (!ready_head[p]) {
                ready_mask &= ~(1u << p);
            }
            cur->next = 0;
            return;
        }
        prev = cur;
        cur = cur->next;
    }
}

static JmkTcb *take_highest(void)
{
    uint8_t p;
    if (ready_mask == 0u) {
        return 0;
    }
    p = (uint8_t)(31u - (uint32_t)__builtin_clz(ready_mask));
    return ready_pop(p);
}

/* ---- 栈构造 ---- */
static void task_build(JmkTcb *t, uint32_t *base, uint32_t words,
                       JmkTaskFn fn, void *arg, uint8_t prio,
                       const char *name, uint8_t id)
{
    uint32_t i;
    uint32_t *top;
    uint32_t *sp;

    for (i = 0; i < words; i++) {
        base[i] = FILL_WORD;
    }

    top = base + words;
    top = (uint32_t *)(((uint32_t)top) & ~7u);   /* 8 字节对齐 */
    sp = top;

    /* 异常返回帧（由硬件在异常返回时弹出）：低地址→高地址 R0..xPSR */
    sp--; *sp = 0x01000000u;                      /* xPSR：置 Thumb 位 */
    sp--; *sp = (uint32_t)fn;                     /* PC */
    sp--; *sp = (uint32_t)jmk_task_exit;          /* LR：任务返回即自删 */
    sp--; *sp = 0u;                               /* R12 */
    sp--; *sp = 0u;                               /* R3 */
    sp--; *sp = 0u;                               /* R2 */
    sp--; *sp = 0u;                               /* R1 */
    sp--; *sp = (uint32_t)arg;                    /* R0 */

    /* 软件上下文：R4-R11（PendSV 弹出这 8 个字） */
    for (i = 0; i < 8u; i++) {
        sp--; *sp = 0u;
    }

    t->sp = sp;
    t->stack_base = base;
    t->stack_words = words;
    t->wake_tick = 0u;
    t->next = 0;
    t->fn = fn;
    t->arg = arg;
    t->prio = prio;
    t->state = ST_READY;
    t->id = id;
    t->name = name;
}

void jmk_task_exit(void)
{
    jmk_task_delete(jmk_current);
    for (;;) {
        __asm volatile ("wfi");
    }
}

static void idle_entry(void *arg)
{
    (void)arg;
    for (;;) {
        __asm volatile ("wfi");
    }
}

/* ---- 对外 API ---- */
int jmk_task_create(JmkTaskFn fn, void *arg, uint8_t prio, const char *name)
{
    uint32_t pm;
    uint8_t i;
    JmkTcb *t = 0;

    if (!fn || prio >= JMK_PRIORITIES) {
        return -1;
    }

    pm = crit_enter();
    for (i = 0; i < JMK_MAX_TASKS; i++) {
        if (jmk_tasks[i].state == ST_FREE || jmk_tasks[i].state == ST_DEAD) {
            t = &jmk_tasks[i];
            break;
        }
    }
    if (!t) {
        crit_exit(pm);
        return -1;
    }

    task_build(t, jmk_stacks[t - jmk_tasks], JMK_TASK_STACK_BYTES / 4u,
               fn, arg, prio, name, (uint8_t)(t - jmk_tasks));
    ready_push(t);

    if (jmk_started && jmk_current && prio > jmk_current->prio) {
        SCB_ICSR = PENDSVSET;   /* 新任务优先级更高：立刻抢占 */
    }
    crit_exit(pm);
    return (int)t->id;
}

void jmk_start(void)
{
    uint32_t pm = crit_enter();
    JmkTcb *first;

    /* 空闲任务：永远在就绪队列里兜底 */
    task_build(&jmk_idle_tcb, jmk_idle_stack, JMK_IDLE_STACK_BYTES / 4u,
               idle_entry, 0, 0u, "idle", 0xFFu);
    ready_push(&jmk_idle_tcb);

    /* SysTick 与 PendSV 都设为最低优先级（0xFF），互相不可抢占 */
    SCB_SHPR3 = (SCB_SHPR3 & 0x0000FFFFu) | (0xFFu << 16) | (0xFFu << 24);

    first = take_highest();
    if (!first) {
        first = &jmk_idle_tcb;
    }
    first->state = ST_RUNNING;
    jmk_current = first;
    jmk_started = 1;
    (void)pm;   /* 不再返回，直接带关中断进入首次切换 */
    jmk_first_switch();
}

void jmk_schedule(void)
{
    JmkTcb *prev = jmk_current;
    JmkTcb *next;

    if (prev && prev->state == ST_RUNNING) {
        prev->state = ST_READY;
        ready_push(prev);
    }

    next = take_highest();
    if (!next) {
        next = &jmk_idle_tcb;
    }
    next->state = ST_RUNNING;
    jmk_current = next;
}

void jmk_tick(void)
{
    JmkTcb *cur = jmk_current;
    uint8_t i;
    int need = 0;

    jmk_tick_count++;

    for (i = 0; i < JMK_MAX_TASKS; i++) {
        JmkTcb *t = &jmk_tasks[i];
        if (t->state == ST_BLOCKED &&
            (int32_t)(jmk_tick_count - t->wake_tick) >= 0) {
            t->state = ST_READY;
            ready_push(t);
            if (cur && t->prio > cur->prio) {
                need = 1;   /* 唤醒的任务优先级更高：抢占 */
            }
        }
    }

    if (!cur) {
        return;
    }
    if (cur->state == ST_BLOCKED) {
        need = 1;       /* 当前任务自己到期被唤醒，也该切走 */
    }
    if (need) {
        SCB_ICSR = PENDSVSET;
    }
}

void jmk_delay(uint32_t ms)
{
    uint32_t pm;
    JmkTcb *cur;

    if (ms == 0u) {
        jmk_yield();
        return;
    }

    pm = crit_enter();
    cur = jmk_current;
    if (!cur) {
        crit_exit(pm);
        return;
    }
    cur->state = ST_BLOCKED;
    cur->wake_tick = jmk_tick_count + ms;
    SCB_ICSR = PENDSVSET;
    crit_exit(pm);   /* 开中断后 PendSV 立即接管，任务在此“睡下” */
}

void jmk_yield(void)
{
    SCB_ICSR = PENDSVSET;
}

uint32_t jmk_ticks(void)
{
    return jmk_tick_count;
}

JmkTcb *jmk_task_at(uint8_t idx)
{
    if (idx >= JMK_MAX_TASKS) {
        return 0;
    }
    return jmk_tasks[idx].state == ST_FREE ? 0 : &jmk_tasks[idx];
}

uint32_t jmk_stack_used(const JmkTcb *t)
{
    uint32_t i;
    if (!t || !t->stack_base) {
        return 0u;
    }
    for (i = 0; i < t->stack_words; i++) {
        if (t->stack_base[i] != FILL_WORD) {
            break;
        }
    }
    return (t->stack_words - i) * 4u;
}

int jmk_task_delete(JmkTcb *t)
{
    uint32_t pm;

    if (!t || t->state == ST_FREE) {
        return -1;
    }

    pm = crit_enter();
    if (t->state != ST_DEAD) {
        ready_remove(t);
        t->state = ST_DEAD;
    }
    if (t == jmk_current) {
        SCB_ICSR = PENDSVSET;
    }
    crit_exit(pm);
    return 0;
}

#include <stdint.h>
#include "jmk.h"

/*
 * 砾微内核（JMK）实现要点：
 *   - 抢占式固定优先级 + 同优先级轮转（就绪队列 FIFO）
 *   - 上下文切换在 PendSV 里做：保存 R4-R11 到任务栈，再交给 jmk_schedule
 *   - SysTick 只负责 tick、唤醒到期任务、按需挂起 PendSV
 *   - 临界区用 PRIMASK 关中断，临界区都很短（链表操作）
 *
 * 任务栈两种来源（见 jmk.h 的 JMK_USE_STATIC_STACKS）：
 *   - 动态：内核独立栈池，8 字节粒度，first-fit + 前后邻合并，删除后回收
 *   - 静态：6 × 512 B + 空闲任务 256 B 固定数组（兼容旧行为）
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
static JmkTcb jmk_idle_tcb;

static JmkTcb *ready_head[JMK_PRIORITIES];
static JmkTcb *ready_tail[JMK_PRIORITIES];
static uint32_t ready_mask;
static volatile uint32_t jmk_tick_count;
static uint8_t jmk_started;

extern void jmk_first_switch(void) __attribute__((noreturn));
void jmk_task_exit(void) __attribute__((noreturn));

#if JMK_USE_STATIC_STACKS
/* ---- 兼容路径：固定槽位静态栈 ---- */
static uint32_t jmk_stacks[JMK_MAX_TASKS][JMK_TASK_STACK_BYTES / 4u]
    __attribute__((aligned(8)));
static uint32_t jmk_idle_stack[JMK_IDLE_STACK_BYTES / 4u]
    __attribute__((aligned(8)));
#else
/* ---- 动态路径：独立栈池 ----
 * 每个块 8 字节块头（2 字）：
 *   h[0] = (size_words << 1) | free_bit
 *   h[1] = 空闲时指向下一空闲块（按地址升序），已分配时无意义
 * 块大小、地址均为 8 字节粒度，因此载荷天然 8 字节对齐。
 */
#define POOL_WORDS   (JMK_STACK_POOL_BYTES / 4u)
#define POOL_HDR     2u

static uint32_t jmk_stack_pool[POOL_WORDS] __attribute__((aligned(8)));
static uint32_t *pool_free_head;
static uint8_t pool_ready;

static uint32_t blk_words(const uint32_t *h)
{
    return h[0] >> 1;
}

static uint32_t *blk_next(const uint32_t *h)
{
    return (uint32_t *)(uintptr_t)h[1];
}

static int blk_is_free(const uint32_t *h)
{
    return (int)(h[0] & 1u);
}

static void pool_init(void)
{
    if (pool_ready) {
        return;
    }
    jmk_stack_pool[0] = (POOL_WORDS << 1) | 1u;   /* 整池一个空闲块 */
    jmk_stack_pool[1] = 0u;
    pool_free_head = jmk_stack_pool;
    pool_ready = 1u;
}

/* 分配 bytes 字节栈，返回载荷（8 字节对齐）；失败返回 0。 */
static uint32_t *pool_alloc(uint32_t bytes, uint32_t *words_out)
{
    uint32_t words, total, sz, rem;
    uint32_t *prev, *cur, *rest;

    pool_init();
    if (bytes < JMK_STACK_MIN_BYTES) {
        bytes = JMK_STACK_MIN_BYTES;
    }
    words = ((bytes + 7u) / 8u) * 2u;    /* 向上取整到 8 字节，换成字 */
    total = words + POOL_HDR;

    prev = 0;
    cur = pool_free_head;
    while (cur) {
        if (blk_is_free(cur) && blk_words(cur) >= total) {
            break;
        }
        prev = cur;
        cur = blk_next(cur);
    }
    if (!cur) {
        return 0;                        /* 池不足：交给上层报 -1 */
    }

    sz = blk_words(cur);
    rem = sz - total;
    if (rem >= POOL_HDR + 2u) {          /* 剩余至少 8 B 载荷才值得拆分 */
        rest = cur + total;
        rest[0] = (rem << 1) | 1u;
        rest[1] = cur[1];
        if (prev) {
            prev[1] = (uint32_t)(uintptr_t)rest;
        } else {
            pool_free_head = rest;
        }
        cur[0] = (total << 1);           /* 已分配，保留原块头位置 */
    } else {
        if (prev) {
            prev[1] = cur[1];
        } else {
            pool_free_head = blk_next(cur);
        }
        cur[0] = (sz << 1);
        words = sz - POOL_HDR;           /* 整块给出，实际载荷更大 */
    }
    *words_out = words;
    return cur + POOL_HDR;
}

/* 回收栈：标空闲 → 按地址插入空闲链 → 前后合并。 */
static void pool_free(uint32_t *payload)
{
    uint32_t *h;
    uint32_t *prev, *cur;
    uint32_t sz;

    if (!payload) {
        return;
    }
    h = payload - POOL_HDR;
    if ((uint32_t)(h - jmk_stack_pool) >= POOL_WORDS) {
        return;                          /* 越界防御 */
    }
    h[0] |= 1u;                          /* 标空闲，size 不变 */

    prev = 0;
    cur = pool_free_head;
    while (cur && (uint32_t)(uintptr_t)cur < (uint32_t)(uintptr_t)h) {
        prev = cur;
        cur = blk_next(cur);
    }
    h[1] = (uint32_t)(uintptr_t)cur;
    if (prev) {
        prev[1] = (uint32_t)(uintptr_t)h;
    } else {
        pool_free_head = h;
    }

    /* 与后邻合并 */
    sz = blk_words(h);
    if (cur && blk_is_free(cur) && h + sz == cur) {
        h[0] = ((sz + blk_words(cur)) << 1) | 1u;
        h[1] = cur[1];
    }

    /* 与前邻合并 */
    if (prev && blk_is_free(prev) && prev + blk_words(prev) == h) {
        prev[0] = ((blk_words(prev) + blk_words(h)) << 1) | 1u;
        prev[1] = h[1];
    }
}

static void pool_stats(uint32_t *used, uint32_t *free_bytes, uint32_t *largest)
{
    uint32_t u = 0u, f = 0u, big = 0u;
    uint32_t n = 0u;
    uint32_t *p;

    pool_init();
    p = jmk_stack_pool;
    while (n < POOL_WORDS) {
        uint32_t sz = blk_words(p);
        if (sz == 0u || sz > POOL_WORDS - n) {
            break;                       /* 结构损坏则停止统计 */
        }
        if (blk_is_free(p)) {
            uint32_t b = sz * 4u;
            f += b;
            if (b > big) {
                big = b;
            }
        } else {
            u += sz * 4u;
        }
        p += sz;
        n += sz;
    }
    *used = u;
    *free_bytes = f;
    *largest = big;
}
#endif /* JMK_USE_STATIC_STACKS */

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

/* 释放任务栈并把 TCB 槽位归零。必须在临界区内或任务已不可能被调度时调用。 */
static void task_release(JmkTcb *t)
{
#if !JMK_USE_STATIC_STACKS
    if (t->stack_base) {
        pool_free(t->stack_base);
    }
#endif
    t->stack_base = 0;
    t->stack_words = 0u;
    t->sp = 0;
    t->fn = 0;
    t->arg = 0;
    t->name = 0;
    t->state = ST_FREE;
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
int jmk_task_create_ex(JmkTaskFn fn, void *arg, uint8_t prio,
                       const char *name, uint32_t stack_bytes)
{
    uint32_t pm;
    uint8_t i;
    JmkTcb *t = 0;
    uint32_t *base = 0;
    uint32_t words = 0;

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

#if JMK_USE_STATIC_STACKS
    if (stack_bytes > JMK_TASK_STACK_BYTES) {
        crit_exit(pm);
        return -1;                       /* 静态路径不支持大于默认值的栈 */
    }
    base = jmk_stacks[t - jmk_tasks];
    words = JMK_TASK_STACK_BYTES / 4u;
#else
    base = pool_alloc(stack_bytes, &words);
    if (!base) {
        crit_exit(pm);
        return -1;                       /* 池不足：不占用槽位，直接失败 */
    }
#endif

    task_build(t, base, words, fn, arg, prio, name, (uint8_t)(t - jmk_tasks));
    ready_push(t);

    if (jmk_started && jmk_current && prio > jmk_current->prio) {
        SCB_ICSR = PENDSVSET;   /* 新任务优先级更高：立刻抢占 */
    }
    crit_exit(pm);
    return (int)t->id;
}

int jmk_task_create(JmkTaskFn fn, void *arg, uint8_t prio, const char *name)
{
    return jmk_task_create_ex(fn, arg, prio, name, JMK_TASK_STACK_BYTES);
}

void jmk_start(void)
{
    uint32_t pm = crit_enter();
    JmkTcb *first;

    /* 空闲任务：永远在就绪队列里兜底 */
#if JMK_USE_STATIC_STACKS
    task_build(&jmk_idle_tcb, jmk_idle_stack, JMK_IDLE_STACK_BYTES / 4u,
               idle_entry, 0, 0u, "idle", 0xFFu);
#else
    {
        uint32_t words = 0u;
        uint32_t *base = pool_alloc(JMK_IDLE_STACK_BYTES, &words);
        if (!base) {
            /* 池连空闲任务都放不下：配置错误，停在这里等调试器 */
            crit_exit(pm);
            for (;;) {
                __asm volatile ("wfi");
            }
        }
        task_build(&jmk_idle_tcb, base, words, idle_entry, 0, 0u, "idle", 0xFFu);
    }
#endif
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
    uint32_t pm;

    if (prev && prev->state == ST_RUNNING) {
        prev->state = ST_READY;
        ready_push(prev);
    }

    /*
     * 自删任务：PendSV 已把上下文存进它的栈，之后不会再恢复执行，
     * 此刻回收栈最安全（槽位同时回到 FREE，可立即复用于新任务）。
     */
    if (prev && prev->state == ST_DEAD) {
        pm = crit_enter();
        task_release(prev);
        crit_exit(pm);
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
    if (t == &jmk_idle_tcb) {
        return -1;       /* 空闲任务由内核持有，不允许删除 */
    }

    pm = crit_enter();
    if (t->state != ST_DEAD) {
        ready_remove(t);
        t->state = ST_DEAD;
    }
    if (t == jmk_current) {
        SCB_ICSR = PENDSVSET;   /* 延迟回收：等 PendSV 存完上下文 */
    } else {
        task_release(t);        /* 已不在运行：立即回收栈 */
    }
    crit_exit(pm);
    return 0;
}

uint32_t jmk_stack_pool_bytes(void)
{
#if JMK_USE_STATIC_STACKS
    return (uint32_t)JMK_MAX_TASKS * JMK_TASK_STACK_BYTES + JMK_IDLE_STACK_BYTES;
#else
    return JMK_STACK_POOL_BYTES;
#endif
}

uint32_t jmk_stack_pool_used(void)
{
#if JMK_USE_STATIC_STACKS
    uint32_t i, used = 0u;
    for (i = 0; i < JMK_MAX_TASKS; i++) {
        if (jmk_tasks[i].state != ST_FREE) {
            used += JMK_TASK_STACK_BYTES;
        }
    }
    if (jmk_idle_tcb.state != ST_FREE) {
        used += JMK_IDLE_STACK_BYTES;
    }
    return used;
#else
    uint32_t used, free_bytes, largest;
    uint32_t pm = crit_enter();
    pool_stats(&used, &free_bytes, &largest);
    crit_exit(pm);
    return used;
#endif
}

uint32_t jmk_stack_pool_free(void)
{
#if JMK_USE_STATIC_STACKS
    return jmk_stack_pool_bytes() - jmk_stack_pool_used();
#else
    uint32_t used, free_bytes, largest;
    uint32_t pm = crit_enter();
    pool_stats(&used, &free_bytes, &largest);
    crit_exit(pm);
    return free_bytes;
#endif
}

uint32_t jmk_stack_pool_largest(void)
{
#if JMK_USE_STATIC_STACKS
    uint32_t i;
    for (i = 0; i < JMK_MAX_TASKS; i++) {
        if (jmk_tasks[i].state == ST_FREE) {
            return JMK_TASK_STACK_BYTES;
        }
    }
    return 0u;
#else
    uint32_t used, free_bytes, largest;
    uint32_t pm = crit_enter();
    pool_stats(&used, &free_bytes, &largest);
    crit_exit(pm);
    return largest;
#endif
}

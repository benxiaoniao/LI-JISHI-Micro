/*
 * LI029 延迟测量（只进 JM_LATENCY_DEMO=1 的取证构建）
 *
 * 量程：
 *   1. PendSV 入口 → 出口（含插桩 C 调用，未扣除），按 (出发优先级 → 到达优先级) 分组；
 *   2. 同一量程里入口处 SysTick 挂起（PENDSTSET）的子集；
 *   3. SysTick 入口 → 下一任务恢复（PendSV 出口）；
 *   4. EXTI 入口 → 消费任务拿到信号量后的第一条语句。
 *
 * 不含：外部信号到 EXTI 入口的异常入栈、PendSV 出口后的硬件出栈、任务到 GPIO 的输出延迟。
 * PC13 在 (4) 的两端各翻转一次，留给逻辑分析仪核对管脚级端到端；本构建只用 CYCCNT 报数。
 */
#include "jishim.h"
#include "stm32f103_port.h"
#include "jmk.h"
#include "jmk_latency.h"

#define JM_DEMCR       (*(volatile uint32_t *)0xE000EDFCu)
#define JM_DWT_CTRL    (*(volatile uint32_t *)0xE0001000u)
#define JM_DWT_CYCCNT  (*(volatile uint32_t *)0xE0001004u)
#define JM_SCB_ICSR    (*(volatile uint32_t *)0xE000ED04u)

#define ICSR_PENDSVSET  (1u << 28)

#define LAT_ROUNDS 200u

typedef struct {
    uint32_t n;
    uint32_t sum;
    uint32_t max;
} LatStat;

static LatStat g_sw[4][4];       /* PendSV 入口→出口，按优先级对 */
static LatStat g_sw_tick[4][4];  /* 由 SysTick 触发切换的子集 */
static LatStat g_tick_to_task;   /* SysTick 入口 → PendSV 出口 */
static LatStat g_isr_to_task;    /* EXTI 入口 → 消费任务首条语句 */

static uint32_t g_sw_t0;
static uint8_t g_sw_from;
static uint8_t g_sw_tick_pending;
static uint32_t g_tick_t0;
static uint8_t g_tick_armed;
static uint32_t g_isr_t0;
static uint8_t g_isr_armed;
static volatile uint8_t g_active = 1u;

static JmkSem g_lat_sem;
static volatile uint32_t g_isr_count;

static uint8_t prio_of(const JmkTcb *t)
{
    if (!t || t->prio >= JMK_PRIORITIES) {
        return 0xFFu;
    }
    return t->prio;
}

static void add_stat(LatStat *s, uint32_t dt)
{
    s->n++;
    s->sum += dt;
    if (dt > s->max) {
        s->max = dt;
    }
}

void jmk_lat_switch_enter(void)
{
    if (!g_active) {
        return;
    }
    g_sw_t0 = JM_DWT_CYCCNT;
    g_sw_from = prio_of(jmk_current);
    /* SysTick 入口已把「本次 tick 会触发切换」标记在 g_tick_armed 上；
     * PENDSTSET 在异常入口就被硬件清掉，不能用来分类。 */
    g_sw_tick_pending = g_tick_armed;
}

void jmk_lat_switch_exit(void)
{
    uint32_t dt;
    uint8_t to;

    if (!g_active) {
        return;
    }
    dt = JM_DWT_CYCCNT - g_sw_t0;
    to = prio_of(jmk_current);
    if (g_sw_from < JMK_PRIORITIES && to < JMK_PRIORITIES) {
        add_stat(&g_sw[g_sw_from][to], dt);
        if (g_sw_tick_pending) {
            add_stat(&g_sw_tick[g_sw_from][to], dt);
        }
    }
    if (g_tick_armed) {
        add_stat(&g_tick_to_task, JM_DWT_CYCCNT - g_tick_t0);
        g_tick_armed = 0u;
    }
}

void jmk_lat_tick_enter(void)
{
    if (!g_active) {
        return;
    }
    g_tick_t0 = JM_DWT_CYCCNT;
    g_tick_armed = 1u;
}

/* SysTick 处理完若没有挂起 PendSV，说明这次 tick 没触发切换，丢掉本次计时。 */
void jmk_lat_tick_exit(void)
{
    if (g_tick_armed && (JM_SCB_ICSR & ICSR_PENDSVSET) == 0u) {
        g_tick_armed = 0u;
    }
}

void jmk_lat_isr_enter(void)
{
    if (!g_active) {
        return;
    }
    port_gpio_write_fast(13u, 0);   /* 管脚标记：ISR 入口（低） */
    g_isr_t0 = JM_DWT_CYCCNT;
    g_isr_armed = 1u;
}

/* 消费任务拿到信号量后的第一条语句调用。 */
static void lat_task_resume(void)
{
    if (!g_active || !g_isr_armed) {
        return;
    }
    add_stat(&g_isr_to_task, JM_DWT_CYCCNT - g_isr_t0);
    g_isr_armed = 0u;
    port_gpio_write_fast(13u, 1);   /* 管脚标记：任务恢复（高） */
}

static void lat_exti_hook(uint32_t line)
{
    uint8_t woken = 0u;
    (void)line;

    g_isr_count++;
    (void)jmk_sem_post_from_isr(&g_lat_sem, &woken);
}

static void task_consumer(void *arg)
{
    (void)arg;
    for (;;) {
        if (jmk_sem_wait(&g_lat_sem, JMK_WAIT_FOREVER) == JMK_OK) {
            lat_task_resume();
        }
    }
}

static void task_high(void *arg)
{
    uint32_t i;
    (void)arg;

    for (i = 0u; i < LAT_ROUNDS; i++) {
        jmk_delay(1u);   /* 高优先级阻塞 → PendSV 切到低优先级 */
    }
    for (;;) {
        jmk_delay(100u);
    }
}

static void task_low(void *arg)
{
    uint32_t i;
    (void)arg;

    for (i = 0u; i < LAT_ROUNDS; i++) {
        port_exti_sw_trigger(0u);   /* 软件触发 EXTI0：ISR 给出信号量，消费任务立即被唤醒 */
        jmk_delay(1u);
    }
    jmk_delay(5u);   /* 等 3→1 的最后一轮计数落完 */
    g_active = 0u;

    port_log("\r\n[延迟] 72 MHz，DWT CYCCNT；量程含插桩调用，未扣除\r\n");
    {
        uint8_t from;
        uint8_t to;
        for (from = 0u; from < JMK_PRIORITIES; from++) {
            for (to = 0u; to < JMK_PRIORITIES; to++) {
                const LatStat *s = &g_sw[from][to];
                const LatStat *t = &g_sw_tick[from][to];
                if (s->n == 0u && t->n == 0u) {
                    continue;
                }
                port_log("[延迟] 切换 ");
                port_log_u32(from);
                port_log("→");
                port_log_u32(to);
                port_log(": n=");
                port_log_u32(s->n);
                port_log(" avg=");
                port_log_u32(s->n ? s->sum / s->n : 0u);
                port_log(" max=");
                port_log_u32(s->max);
                if (t->n != 0u) {
                    port_log("　tick 触发: n=");
                    port_log_u32(t->n);
                    port_log(" avg=");
                    port_log_u32(t->sum / t->n);
                    port_log(" max=");
                    port_log_u32(t->max);
                }
                port_log("\r\n");
            }
        }
    }
    port_log("[延迟] tick→任务恢复: n=");
    port_log_u32(g_tick_to_task.n);
    port_log(" avg=");
    port_log_u32(g_tick_to_task.n ? g_tick_to_task.sum / g_tick_to_task.n : 0u);
    port_log(" max=");
    port_log_u32(g_tick_to_task.max);
    port_log("\r\n");
    port_log("[延迟] EXTI→任务恢复: n=");
    port_log_u32(g_isr_to_task.n);
    port_log(" avg=");
    port_log_u32(g_isr_to_task.n ? g_isr_to_task.sum / g_isr_to_task.n : 0u);
    port_log(" max=");
    port_log_u32(g_isr_to_task.max);
    port_log("（ISR 触发=");
    port_log_u32(g_isr_count);
    port_log(" 次，消费任务首条语句）\r\n");
    port_log("[延迟] 说明：不含外部信号→ISR 入栈、PendSV 出口后的硬件出栈、任务→GPIO 输出\r\n");

    for (;;) {
        jmk_delay(1000u);
    }
}

void jmk_latency_demo(void)
{
    JM_DEMCR |= (1u << 24);   /* TRCENA */
    JM_DWT_CYCCNT = 0u;
    JM_DWT_CTRL |= 1u;        /* CYCCNTENA */

    if (jmk_sem_init(&g_lat_sem, 0, 4) != JMK_OK) {
        port_log("[延迟] 信号量初始化失败\r\n");
    }
    port_exti_bind(0u, 2u);   /* PA0，双沿；实际用软件触发 */
    port_exti_set_hook(lat_exti_hook);

    port_log("\r\n[延迟] 演示：high(3)/consumer(2)/low(1)，");
    port_log_u32(LAT_ROUNDS);
    port_log(" 轮\r\n");

    if (jmk_task_create(task_consumer, 0, 2u, "latc") < 0 ||
        jmk_task_create(task_high, 0, 3u, "lath") < 0 ||
        jmk_task_create(task_low, 0, 1u, "latl") < 0) {
        port_log("[延迟] 任务创建失败\r\n");
    }
    jmk_start();
}

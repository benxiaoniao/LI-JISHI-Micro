#include "jishim.h"
#include "stm32f103_port.h"
#include "jmk.h"

/* 栈动态分配演示：默认开启，可 -DJMK_STACK_CHURN_DEMO=0 关闭 */
#ifndef JMK_STACK_CHURN_DEMO
#define JMK_STACK_CHURN_DEMO 1
#endif

/* 由构建脚本从 .jmb 生成 */
extern const unsigned char jm_app_jmb[];
extern const uint32_t jm_app_jmb_len;

/* 任务一：闪 PC13，250 ms 一次 */
static void task_led(void *arg)
{
    (void)arg;
    for (;;) {
        port_led_toggle();
        jmk_delay(250u);
    }
}

static void log_stacks(void)
{
    uint8_t i;

    port_log("[内核] 栈峰值(字节): ");
    for (i = 0; i < JMK_MAX_TASKS; i++) {
        JmkTcb *t = jmk_task_at(i);
        uint32_t used;
        uint32_t cap;
        if (!t || !t->name) {
            continue;
        }
        used = jmk_stack_used(t);
        cap = t->stack_words * 4u;
        port_log(t->name);
        port_log("=");
        port_log_u32(used);
        port_log(" ");
        if (cap && used > cap - cap / 5u) {
            port_log("(接近溢出!) ");
        }
    }
    port_log("\r\n");
}

/* 任务二：每 1000 ms 打印一次计数与 uptime，每 5 次附带栈峰值 */
static void task_log(void *arg)
{
    uint32_t n = 0;
    (void)arg;

    for (;;) {
        n++;
        port_log("[任务] 秒计数: ");
        port_log_u32(n);
        port_log("  uptime(ms): ");
        port_log_u32(port_uptime_ms());
        port_log("  tick: ");
        port_log_u32(jmk_ticks());
        port_log("\r\n");

        if ((n % 5u) == 0u) {
            log_stacks();
        }
        jmk_delay(1000u);
    }
}

#if JMK_STACK_CHURN_DEMO
/* ---- 栈动态分配示例：反复创建/删除任务，验证栈不泄漏 ---- */
#define CHURN_ROUNDS     6u
#define CHURN_SELF_BYTES 384u   /* 子任务 A：跑完自己退出 → 延迟回收 */
#define CHURN_KILL_BYTES 256u   /* 子任务 B：被父任务删除 → 立即回收 */

/* A：闪 3 次灯后返回（返回即自删，栈在下次切换时回收） */
static void churn_self(void *arg)
{
    uint32_t i;
    (void)arg;

    for (i = 0; i < 3u; i++) {
        port_led_toggle();
        jmk_delay(40u);
    }
    port_log("[栈池] 子任务A 退出前 栈峰值=");
    port_log_u32(jmk_stack_used(jmk_current));
    port_log(" 字节\r\n");
}

/* B：一直睡，等父任务把它删掉 */
static void churn_sleep(void *arg)
{
    (void)arg;
    for (;;) {
        jmk_delay(1000u);
    }
}

static void log_pool(const char *tag, uint32_t round)
{
    port_log("[栈池] 第 ");
    port_log_u32(round);
    port_log(" 轮 ");
    port_log(tag);
    port_log(": 已用=");
    port_log_u32(jmk_stack_pool_used());
    port_log(" 空闲=");
    port_log_u32(jmk_stack_pool_free());
    port_log(" 最大空闲块=");
    port_log_u32(jmk_stack_pool_largest());
    port_log(" 砾堆已用=");
    port_log_u32(jm_heap_used());
    port_log("\r\n");
}

static void task_churn(void *arg)
{
    uint32_t round;
    int id;
    (void)arg;

    port_log("[栈池] 动态栈示例开始：栈池容量=");
    port_log_u32(jmk_stack_pool_bytes());
    port_log(" 字节，每轮建 2 个任务（1 自退出 / 1 被删除）\r\n");

    for (round = 1u; round <= CHURN_ROUNDS; round++) {
        log_pool("创建前", round);

        if (jmk_task_create_ex(churn_self, 0, 2u, "churnA", CHURN_SELF_BYTES) < 0) {
            port_log("[栈池] 子任务A 创建失败（池不足）\r\n");
        }

        id = jmk_task_create_ex(churn_sleep, 0, 2u, "churnB", CHURN_KILL_BYTES);
        if (id < 0) {
            port_log("[栈池] 子任务B 创建失败（池不足）\r\n");
        } else if (jmk_task_delete(jmk_task_at((uint8_t)id)) == 0) {
            port_log("[栈池] 子任务B 已删除（立即回收 256 B）\r\n");
        }

        log_pool("建删后", round);   /* 此时只应多出 A 的 392 B（含块头） */
        jmk_delay(400u);             /* 让 A 跑完自删，栈在切换时回收 */
        log_pool("回收后", round);   /* 应回到「创建前」基线 */

        if (round < CHURN_ROUNDS) {
            jmk_delay(200u);
        }
    }

    port_log("[栈池] 示例结束：各轮「回收后」与「创建前」一致即证明栈不泄漏\r\n");
    for (;;) {
        jmk_delay(2000u);
    }
}
#endif /* JMK_STACK_CHURN_DEMO */

int main(void)
{
    port_init();

    port_log("\r\n砾（LI）运行时 · STM32F103C8T6\r\n");
    port_log("系统时钟: ");
    port_log_u32(port_sysclk());
    port_log(" Hz\r\n");
    port_log("内嵌程序: ");
    port_log_u32(jm_app_jmb_len);
    port_log(" 字节\r\n\r\n");

    if (jm_run(jm_app_jmb, (size_t)jm_app_jmb_len) == 0) {
        port_log("\r\n[砾] 程序正常结束\r\n");
    } else {
        port_log("\r\n[砾] 运行失败: ");
        port_log(jm_last_error());
        port_log("\r\n");
    }

#if JMK_STACK_CHURN_DEMO
    port_log("\r\n[内核] 启动调度器：churn(优先3) / led(优先2) / log(优先1)\r\n");
#else
    port_log("\r\n[内核] 启动调度器：led(优先2) / log(优先1)\r\n");
#endif

    if (jmk_task_create(task_led, 0, 2u, "led") < 0) {
        port_log("[内核] 创建 led 任务失败\r\n");
    }
    if (jmk_task_create(task_log, 0, 1u, "log") < 0) {
        port_log("[内核] 创建 log 任务失败\r\n");
    }
#if JMK_STACK_CHURN_DEMO
    if (jmk_task_create(task_churn, 0, 3u, "churn") < 0) {
        port_log("[内核] 创建 churn 任务失败\r\n");
    }
#endif

    jmk_start();

    for (;;) {
    }
}

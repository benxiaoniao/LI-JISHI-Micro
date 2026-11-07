#include "jishim.h"
#include "stm32f103_port.h"
#include "f103_native.h"
#include "jmk.h"

/*
 * 示例开关（两个示例各建 2 个任务，6 个槽位放不下同时开）：
 *   JMK_IPC_DEMO         任务间通信示例（led 生产 / log 消费），默认开启
 *   JMK_STACK_CHURN_DEMO 栈池建删演示（已上板验证 6 轮），默认关闭
 * 切换：-DJMK_IPC_DEMO=0 -DJMK_STACK_CHURN_DEMO=1
 */
#ifndef JMK_IPC_DEMO
#define JMK_IPC_DEMO 1
#endif

#ifndef JMK_STACK_CHURN_DEMO
#define JMK_STACK_CHURN_DEMO 0
#endif

/* 让出证据开关（默认关）：与 vm 同优先级建一个对端任务，每轮打印后 jmk_yield()；
 * 配合 examples/让出.jsh（循环里 让出()）观察两个任务是否交替执行。 */
#ifndef JMK_YIELD_DEMO
#define JMK_YIELD_DEMO 0
#endif

/* 由构建脚本从 .jmb 生成 */
extern const unsigned char jm_app_jmb[];
extern const uint32_t jm_app_jmb_len;

/* 砾程序任务的栈：VM 自身用固定状态区，C 栈只留函数帧与格式化缓冲。 */
#define VM_TASK_STACK_BYTES 1024u

/* vm 栈诊断：程序结束后用 0xA5 哨兵峰值与耗时打一行；默认开，可 -DJM_STACK_DIAG=0 关掉。 */
#ifndef JM_STACK_DIAG
#define JM_STACK_DIAG 1
#endif

#if JMK_IPC_DEMO
/* ---- 任务间通信示例：led 生产，log 消费 ----
 * 数据路径：led 每 200 ms 产生一个序号 → 入队（调用方提供 8 × 4 B 缓冲）
 * 事件路径：入队成功后给出计数信号量（上限 8），log 靠它知道“有数据可取”
 * 各司其职：队列搬数据，信号量只做事件计数；队列满就丢弃并打印，绝不阻塞生产者。
 */
#define IPC_SLOTS 8u

static JmkQueue g_log_q;                  /* 32 B 控制块 */
static uint32_t g_log_buf[IPC_SLOTS];     /* 32 B 槽位：调用方提供，内核不占 */
static JmkSem g_evt_sem;                  /* 12 B，计数上限 = 槽数 */
static JmkSem g_probe_sem;                /* 12 B，永不给出，只用来验证超时路径 */

/* 生产者：闪灯 + 发一条消息 */
static void task_led(void *arg)
{
    uint32_t seq = 0;
    (void)arg;
    for (;;) {
        port_led_toggle();
        seq++;
        if (jmk_queue_send(&g_log_q, &seq, JMK_NO_WAIT) == JMK_OK) {
            (void)jmk_sem_post(&g_evt_sem);
        } else {
            port_log("[同步] 队列满，丢弃 seq=");
            port_log_u32(seq);
            port_log("\r\n");
        }
        jmk_delay(200u);
    }
}

/* 消费者：先等到事件，再取数据 */
static void task_log(void *arg)
{
    uint32_t got = 0;
    (void)arg;

    /* 超时自检：g_probe_sem 永远不会被给出，应返回 -1 */
    if (jmk_sem_wait(&g_probe_sem, 100u) == JMK_E_TIMEOUT) {
        port_log("[同步] 超时自检通过：100 ms 无事件返回 -1\r\n");
    } else {
        port_log("[同步] 超时自检异常\r\n");
    }

    for (;;) {
        if (jmk_sem_wait(&g_evt_sem, JMK_WAIT_FOREVER) == JMK_OK) {
            uint32_t seq;
            if (jmk_queue_recv(&g_log_q, &seq, JMK_NO_WAIT) == JMK_OK) {
                got++;
                port_log("[同步] 收到 seq=");
                port_log_u32(seq);
                port_log(" 累计=");
                port_log_u32(got);
                port_log(" 栈峰值=");
                port_log_u32(jmk_stack_used(jmk_current));
                port_log(" 队列余量=");
                port_log_u32(jmk_queue_count(&g_log_q));
                port_log("\r\n");
            }
        }
    }
}
#else
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
#endif /* JMK_IPC_DEMO */

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

#if JMK_YIELD_DEMO
/* 让出对端：与 vm 同优先级（2），每轮打印一次就主动让出。
 * vm 里的 .jsh 也每轮 让出()，两个任务应交替拿到 CPU。 */
static void task_peer(void *arg)
{
    uint32_t n = 0;
    (void)arg;
    for (;;) {
        n++;
        port_log("[让出] 乙 ");
        port_log_u32(n);
        port_log("\r\n");
        jmk_yield();
    }
}
#endif

/* 砾程序作为独立 JMK 任务运行：让语言层 让出()/延时() 真正落到调度器上。 */
static void task_vm(void *arg)
{
#if JM_STACK_DIAG
    uint32_t t0 = jmk_ticks();
#endif

    (void)arg;
    if (jm_run(jm_app_jmb, (size_t)jm_app_jmb_len) == 0) {
        port_log("\r\n[砾] 程序正常结束\r\n");
    } else {
        port_log("\r\n[砾] 运行失败: ");
        port_log(jm_last_error());
        port_log("\r\n");
    }

#if JM_STACK_DIAG
    /* 栈峰值：栈创建时已用 0xA5 填满，jmk_stack_used 扫描未被覆盖的部分 */
    if (jmk_current) {
        uint32_t cap = jmk_current->stack_words * 4u;
        uint32_t used = jmk_stack_used(jmk_current);
        port_log("[砾] vm 栈峰值=");
        port_log_u32(used);
        port_log(" B / ");
        port_log_u32(cap);
        port_log(" B，余量=");
        port_log_u32(cap - used);
        port_log(" B，耗时=");
        port_log_u32(jmk_ticks() - t0);
        port_log(" ms\r\n");
    }
#endif
}

int main(void)
{
    port_init();

    port_log("\r\n砾（LI）运行时 · STM32F103C8T6\r\n");
    port_log("系统时钟: ");
    port_log_u32(port_sysclk());
    port_log(" Hz\r\n");
    port_log("内嵌程序: ");
    port_log_u32(jm_app_jmb_len);
    port_log(" 字节\r\n");

    if (f103_native_register() != 0) {
        port_log("[原生] 登记失败：板级函数表\r\n");
    } else {
        port_log("[原生] 板级函数表已登记：");
        port_log(F103_NATIVE_TABLE_NAME);
        port_log(", ");
        port_log(F103_ADC_TABLE_NAME);
        port_log(", ");
        port_log(F103_TIMER_TABLE_NAME);
        port_log(", ");
        port_log(JM_TASK_TABLE_NAME);
        port_log(", ");
        port_log(JM_QUEUE_TABLE_NAME);
        port_log(", ");
        port_log(JM_SEM_TABLE_NAME);
        port_log(", ");
        port_log(JM_IRQ_TABLE_NAME);
        port_log("\r\n");
    }
    port_log("\r\n");

#if JMK_IPC_DEMO
    port_log("[同步] 队列缓冲: ");
    port_log_u32(IPC_SLOTS);
    port_log(" 槽 × ");
    port_log_u32(sizeof(g_log_buf[0]));
    port_log(" B = ");
    port_log_u32(sizeof(g_log_buf));
    port_log(" B（调用方提供，内核只占 32 B 控制块）\r\n");
    if (jmk_queue_init(&g_log_q, g_log_buf, sizeof(g_log_buf[0]), IPC_SLOTS) != JMK_OK ||
        jmk_sem_init(&g_evt_sem, 0, (int32_t)IPC_SLOTS) != JMK_OK ||
        jmk_sem_init(&g_probe_sem, 0, 1) != JMK_OK) {
        port_log("[同步] 初始化失败\r\n");
    }
#endif

#if JMK_YIELD_DEMO
    port_log("\r\n[内核] 启动调度器：vm(优先2) / peer(让出对端,优先2) / led(优先2) / log(优先1)\r\n");
#elif JMK_IPC_DEMO
    port_log("\r\n[内核] 启动调度器：vm(优先2) / led(生产,优先2) / log(消费,优先1)\r\n");
#elif JMK_STACK_CHURN_DEMO
    port_log("\r\n[内核] 启动调度器：churn(优先3) / vm(优先2) / led(优先2) / log(优先1)\r\n");
#else
    port_log("\r\n[内核] 启动调度器：vm(优先2) / led(优先2) / log(优先1)\r\n");
#endif

    if (jmk_task_create_ex(task_vm, 0, 2u, "vm", VM_TASK_STACK_BYTES) < 0) {
        port_log("[内核] 创建 vm 任务失败（栈池不足），退回主上下文运行\r\n");
        task_vm(0);
    }
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
#if JMK_YIELD_DEMO
    if (jmk_task_create(task_peer, 0, 2u, "peer") < 0) {
        port_log("[内核] 创建 peer 任务失败\r\n");
    }
#endif

    jmk_start();

    for (;;) {
    }
}

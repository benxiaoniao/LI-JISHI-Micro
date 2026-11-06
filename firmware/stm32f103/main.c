#include "jishim.h"
#include "stm32f103_port.h"
#include "jmk.h"

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
        if (!t) {
            continue;
        }
        port_log(t->name);
        port_log("=");
        port_log_u32(jmk_stack_used(t));
        port_log(" ");
        if (jmk_stack_used(t) > (JMK_TASK_STACK_BYTES * 4u) / 5u) {
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

    port_log("\r\n[内核] 启动调度器：led(优先2) / log(优先1)\r\n");

    if (jmk_task_create(task_led, 0, 2u, "led") < 0) {
        port_log("[内核] 创建 led 任务失败\r\n");
    }
    if (jmk_task_create(task_log, 0, 1u, "log") < 0) {
        port_log("[内核] 创建 log 任务失败\r\n");
    }

    jmk_start();

    for (;;) {
    }
}

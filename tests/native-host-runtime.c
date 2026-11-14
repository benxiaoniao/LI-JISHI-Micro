/*
 * LI021：native 表运行期主机回归。
 * 目标：用主机同名替身表实际走一遍「登记 → 绑定 → 分派 → 参数/错误码」，
 * 不依赖 .jsh 示例，也不上板。
 */

#include <stdio.h>
#include <string.h>

#include "jm_internal.h"
#include "jm_native.h"

/* 主机端口登记入口（runtime/port/host/jm_host_native.c）。 */
int jm_host_native_register(void);

static int g_fail;

static void check(const char *title, int ok)
{
    printf(ok ? "[通过] %s\n" : "[不通过] %s\n", title);
    if (!ok) {
        g_fail++;
    }
}

/* 每次调用前清掉上一次的错误，避免误判。 */
static int call(const char *name, JmValue *args, uint8_t nargs, JmValue *out)
{
    uint8_t id = jm_native_find(name);

    jm_state.has_err = 0;
    jm_state.err.code = 0;
    jm_state.err.detail = NULL;
    return jm_native_call(&jm_state, id, args, nargs, out);
}

static void expect_ok(const char *title, const char *name, JmValue *args,
                      uint8_t nargs)
{
    JmValue out = JM_NIL;
    int rc = call(name, args, nargs, &out);
    check(title, rc == 0 && !jm_state.has_err);
}

static void expect_nil(const char *title, const char *name, JmValue *args,
                       uint8_t nargs)
{
    JmValue out = jm_value_int(-1);
    int rc = call(name, args, nargs, &out);
    check(title, rc == 0 && !jm_state.has_err && out == jm_value_nil());
}

static void expect_bool(const char *title, const char *name, JmValue *args,
                        uint8_t nargs, int want)
{
    JmValue out = jm_value_nil();
    int rc = call(name, args, nargs, &out);
    check(title, rc == 0 && !jm_state.has_err && out == jm_value_bool(want));
}

static void expect_int(const char *title, const char *name, JmValue *args,
                       uint8_t nargs, int32_t want)
{
    JmValue out = jm_value_nil();
    int rc = call(name, args, nargs, &out);
    check(title, rc == 0 && !jm_state.has_err &&
                jm_value_kind(out) == JM_VK_INT &&
                jm_value_get_int(out) == want);
}

static void expect_err(const char *title, const char *name, JmValue *args,
                       uint8_t nargs, int code, const char *needle)
{
    JmValue out = JM_NIL;
    int rc = call(name, args, nargs, &out);
    int ok = (rc != 0) && jm_state.has_err && jm_state.err.code == code;
    if (ok && needle != NULL) {
        ok = jm_state.err.detail != NULL && strstr(jm_state.err.detail, needle) != NULL;
    }
    check(title, ok);
    if (!ok) {
        printf("        期望错误 %d%s%s，实际 rc=%d code=%d detail=%s\n",
               code, needle ? "，包含 " : "", needle ? needle : "",
               rc, jm_state.has_err ? jm_state.err.code : -1,
               jm_state.err.detail ? jm_state.err.detail : "(无)");
    }
}

static int dummy_fn(JmStateRef st, const JmValue *args, uint8_t nargs,
                    JmValue *out)
{
    (void)st;
    (void)args;
    (void)nargs;
    (void)out;
    return JM_NATIVE_OK;
}

static const char *EXPECTED_NAMES[] = {
    /* f103_board */
    "引脚写", "引脚读", "引脚翻转", "串口写", "等待毫秒",
    /* f103_adc */
    "ADC初始化", "ADC读",
    /* f103_timer */
    "定时器初始化", "定时器延时", "定时器计数",
    "PWM启动", "PWM占空比", "PWM停止",
    /* ipc_queue */
    "队列创建", "队列发送", "队列接收", "队列数量", "队列销毁",
    /* ipc_sem */
    "信号量创建", "信号量等待", "信号量给出",
    /* f103_exti */
    "中断绑定", "中断触发", "中断计数", "中断解除",
    /* f103_bus */
    "I2C初始化", "I2C写", "I2C写块", "I2C探测",
    "SPI初始化", "SPI收发", "SPI写块",
};

int main(void)
{
    uint32_t i;
    int all_bound = 1;
    int32_t queue_handle;

    /* ---------- 登记与绑定 ---------- */
    check("主机同名替身表登记成功", jm_host_native_register() == 0);
    check("ABI 版本为 2", jm_native_abi_version() == 2u);

    for (i = 0; i < (uint32_t)(sizeof(EXPECTED_NAMES) / sizeof(EXPECTED_NAMES[0])); i++) {
        if (jm_native_find(EXPECTED_NAMES[i]) == 0 ||
            jm_native_bind_name(&jm_state, (uint8_t)i, EXPECTED_NAMES[i]) != 1) {
            printf("[不通过] 名字不可绑定：%s\n", EXPECTED_NAMES[i]);
            g_fail++;
            all_bound = 0;
        }
    }
    check("32 个原生名字全部可绑定", all_bound);
    check("未知名字绑定返回 0",
          jm_native_bind_name(&jm_state, 0, "不存在的原生函数") == 0);

    {
        const JmNativeEntry dup[] = {
            {"引脚写", dummy_fn, 0, 0, 0, 0},
        };
        check("重复名字登记被拒绝", jm_native_register(dup, 1) == -1);
    }

    /* ---------- 分派层实参个数 ---------- */
    expect_err("引脚写 只给 1 个实参：E012", "引脚写",
               (JmValue[]){jm_value_int(0)}, 1, E_ARGC, NULL);
    expect_err("引脚写 给 3 个实参：E012", "引脚写",
               (JmValue[]){jm_value_int(0), jm_value_int(0), jm_value_int(0)}, 3,
               E_ARGC, NULL);
    expect_err("队列创建 0 个实参：E012", "队列创建", NULL, 0, E_ARGC, NULL);
    expect_err("I2C写块 只给 2 个实参：E012", "I2C写块",
               (JmValue[]){jm_value_int(0x3C), jm_value_int(0x40)}, 2,
               E_ARGC, NULL);

    /* ---------- 引脚 ---------- */
    expect_nil("引脚写(13, 1)", "引脚写",
               (JmValue[]){jm_value_int(13), jm_value_int(1)}, 2);
    expect_int("引脚读(13) = 1", "引脚读", (JmValue[]){jm_value_int(13)}, 1, 1);
    expect_nil("引脚翻转(13)", "引脚翻转", (JmValue[]){jm_value_int(13)}, 1);
    expect_int("翻转后 引脚读(13) = 0", "引脚读",
               (JmValue[]){jm_value_int(13)}, 1, 0);
    expect_err("引脚写 引脚 48：E030", "引脚写",
               (JmValue[]){jm_value_int(48), jm_value_int(0)}, 2,
               JM_NE_PARAM, "引脚编码");
    expect_err("引脚写 电平 2：E030", "引脚写",
               (JmValue[]){jm_value_int(0), jm_value_int(2)}, 2,
               JM_NE_PARAM, "电平");
    expect_err("引脚写 电平传布尔：E031", "引脚写",
               (JmValue[]){jm_value_int(0), jm_value_bool(1)}, 2,
               JM_NE_TYPE, "整数");
    expect_err("引脚读 引脚 48：E030", "引脚读",
               (JmValue[]){jm_value_int(48)}, 1, JM_NE_PARAM, "引脚编码");

    /* ---------- ADC ---------- */
    expect_bool("ADC初始化()", "ADC初始化", NULL, 0, 1);
    expect_ok("ADC读(0)", "ADC读", (JmValue[]){jm_value_int(0)}, 1);
    expect_err("ADC读 通道 8：E030", "ADC读",
               (JmValue[]){jm_value_int(8)}, 1, JM_NE_PARAM, "通道");
    expect_err("ADC读 通道传布尔：E031", "ADC读",
               (JmValue[]){jm_value_bool(1)}, 1, JM_NE_TYPE, "整数");

    /* ---------- 定时器 / PWM ---------- */
    expect_bool("定时器初始化()", "定时器初始化", NULL, 0, 1);
    expect_ok("定时器计数()", "定时器计数", NULL, 0);
    expect_bool("定时器延时(0)", "定时器延时",
                (JmValue[]){jm_value_int(0)}, 1, 1);
    expect_err("PWM占空比 未启动：E033", "PWM占空比",
               (JmValue[]){jm_value_int(500)}, 1, JM_NE_STATE, "未启动");
    expect_err("PWM启动 引脚 21：E030", "PWM启动",
               (JmValue[]){jm_value_int(21), jm_value_int(1000), jm_value_int(500)}, 3,
               JM_NE_PARAM, "22–25");
    expect_err("PWM启动 频率 0：E030", "PWM启动",
               (JmValue[]){jm_value_int(22), jm_value_int(0), jm_value_int(500)}, 3,
               JM_NE_PARAM, "频率");
    expect_err("PWM启动 占空比 1001：E030", "PWM启动",
               (JmValue[]){jm_value_int(22), jm_value_int(1000), jm_value_int(1001)}, 3,
               JM_NE_PARAM, "占空比");
    expect_nil("PWM启动(22, 1000, 500)", "PWM启动",
               (JmValue[]){jm_value_int(22), jm_value_int(1000), jm_value_int(500)}, 3);
    expect_err("PWM占空比 1001：E030", "PWM占空比",
               (JmValue[]){jm_value_int(1001)}, 1, JM_NE_PARAM, "占空比");
    expect_nil("PWM占空比(500)", "PWM占空比",
               (JmValue[]){jm_value_int(500)}, 1);
    expect_nil("PWM停止()", "PWM停止", NULL, 0);
    expect_err("PWM停止 再次：E033", "PWM停止", NULL, 0, JM_NE_STATE, "未启动");

    /* ---------- 等待 / 串口 ---------- */
    expect_nil("等待毫秒(0)", "等待毫秒", (JmValue[]){jm_value_int(0)}, 1);
    expect_err("等待毫秒 60001：E030", "等待毫秒",
               (JmValue[]){jm_value_int(60001)}, 1, JM_NE_PARAM, "0–60000");
    expect_err("串口写 传整数：E031", "串口写",
               (JmValue[]){jm_value_int(1)}, 1, JM_NE_TYPE, "文本");

    /* ---------- I2C ---------- */
    expect_err("I2C探测 未初始化：E033", "I2C探测",
               (JmValue[]){jm_value_int(0x3C)}, 1, JM_NE_STATE, "未初始化");
    expect_err("I2C初始化 SCL=SDA：E030", "I2C初始化",
               (JmValue[]){jm_value_int(26), jm_value_int(26), jm_value_int(400)}, 3,
               JM_NE_PARAM, "同一个引脚");
    expect_err("I2C初始化 9 kHz：E030", "I2C初始化",
               (JmValue[]){jm_value_int(26), jm_value_int(27), jm_value_int(9)}, 3,
               JM_NE_PARAM, "速率");
    expect_bool("I2C初始化(26, 27, 400) 走硬件", "I2C初始化",
                (JmValue[]){jm_value_int(26), jm_value_int(27), jm_value_int(400)}, 3, 1);
    expect_bool("I2C探测(0x3C) = 真", "I2C探测",
                (JmValue[]){jm_value_int(0x3C)}, 1, 1);
    expect_bool("I2C探测(0x3D) = 假", "I2C探测",
                (JmValue[]){jm_value_int(0x3D)}, 1, 0);
    expect_nil("I2C写(0x3C, 0x40, 0)", "I2C写",
               (JmValue[]){jm_value_int(0x3C), jm_value_int(0x40), jm_value_int(0)}, 3);
    expect_err("I2C写(0x3D) 无应答：E032", "I2C写",
               (JmValue[]){jm_value_int(0x3D), jm_value_int(0x40), jm_value_int(0)}, 3,
               JM_NE_HW, "未应答");
    expect_err("I2C写 地址 128：E030", "I2C写",
               (JmValue[]){jm_value_int(128), jm_value_int(0x40), jm_value_int(0)}, 3,
               JM_NE_PARAM, "0–127");
    expect_nil("I2C写块(0x3C, 0x40, 1)", "I2C写块",
               (JmValue[]){jm_value_int(0x3C), jm_value_int(0x40), jm_value_int(1)}, 3);

    /* ---------- SPI ---------- */
    expect_err("SPI收发 未初始化：E033", "SPI收发",
               (JmValue[]){jm_value_int(1)}, 1, JM_NE_STATE, "未初始化");
    expect_err("SPI初始化 分频 3：E030", "SPI初始化",
               (JmValue[]){jm_value_int(5), jm_value_int(6), jm_value_int(7), jm_value_int(3)}, 4,
               JM_NE_PARAM, "分频");
    expect_bool("SPI初始化(5, 6, 7, 2) 走硬件", "SPI初始化",
                (JmValue[]){jm_value_int(5), jm_value_int(6), jm_value_int(7), jm_value_int(2)}, 4, 1);
    expect_int("SPI收发(165) 回环 = 165", "SPI收发",
               (JmValue[]){jm_value_int(165)}, 1, 165);
    expect_err("SPI初始化 重复引脚：E030", "SPI初始化",
               (JmValue[]){jm_value_int(5), jm_value_int(5), jm_value_int(7), jm_value_int(2)}, 4,
               JM_NE_PARAM, "不能重复");

    /* ---------- 队列 ---------- */
    expect_err("队列创建 槽数 9：E030", "队列创建",
               (JmValue[]){jm_value_int(9)}, 1, JM_NE_PARAM, "1–8");
    expect_int("队列创建(4) = 1", "队列创建",
               (JmValue[]){jm_value_int(4)}, 1, 1);
    queue_handle = 1;
    expect_bool("队列发送(1, 42)", "队列发送",
                (JmValue[]){jm_value_int(queue_handle), jm_value_int(42)}, 2, 1);
    expect_int("队列数量(1) = 1", "队列数量",
               (JmValue[]){jm_value_int(queue_handle)}, 1, 1);
    expect_int("队列接收(1) = 42", "队列接收",
               (JmValue[]){jm_value_int(queue_handle)}, 1, 42);
    expect_nil("队列接收(1) 空 = 空", "队列接收",
               (JmValue[]){jm_value_int(queue_handle)}, 1);
    expect_err("队列数量(99)：E030", "队列数量",
               (JmValue[]){jm_value_int(99)}, 1, JM_NE_PARAM, "句柄");
    expect_bool("队列销毁(1) = 真", "队列销毁",
                (JmValue[]){jm_value_int(queue_handle)}, 1, 1);
    expect_err("销毁后 队列数量(1)：E030", "队列数量",
               (JmValue[]){jm_value_int(queue_handle)}, 1, JM_NE_PARAM, "句柄");

    /* ---------- 信号量 ---------- */
    expect_err("信号量创建 上限 0：E030", "信号量创建",
               (JmValue[]){jm_value_int(0), jm_value_int(0)}, 2,
               JM_NE_PARAM, "上限");
    expect_int("信号量创建(0, 1) = 1", "信号量创建",
               (JmValue[]){jm_value_int(0), jm_value_int(1)}, 2, 1);
    expect_bool("信号量等待 0 超时 = 假", "信号量等待",
                (JmValue[]){jm_value_int(1)}, 1, 0);
    expect_bool("信号量给出 = 真", "信号量给出",
                (JmValue[]){jm_value_int(1)}, 1, 1);
    expect_bool("信号量等待 = 真", "信号量等待",
                (JmValue[]){jm_value_int(1)}, 1, 1);
    expect_bool("信号量等待 再次 = 假", "信号量等待",
                (JmValue[]){jm_value_int(1)}, 1, 0);
    expect_bool("信号量给出 再次 = 真", "信号量给出",
                (JmValue[]){jm_value_int(1)}, 1, 1);
    expect_bool("信号量给出 已满 = 假", "信号量给出",
                (JmValue[]){jm_value_int(1)}, 1, 0);
    expect_err("信号量等待 句柄 99：E030", "信号量等待",
               (JmValue[]){jm_value_int(99)}, 1, JM_NE_PARAM, "句柄");

    /* ---------- 外部中断 ---------- */
    expect_int("重建队列 = 1", "队列创建",
               (JmValue[]){jm_value_int(4)}, 1, 1);
    queue_handle = 1;
    expect_err("中断绑定 引脚 48：E030", "中断绑定",
               (JmValue[]){jm_value_int(48), jm_value_int(queue_handle)}, 2,
               JM_NE_PARAM, "引脚范围");
    expect_bool("中断绑定(0, 1) = 真", "中断绑定",
                (JmValue[]){jm_value_int(0), jm_value_int(queue_handle)}, 2, 1);
    expect_bool("中断触发(0) = 真", "中断触发",
                (JmValue[]){jm_value_int(0)}, 1, 1);
    expect_int("中断计数(0) = 1", "中断计数",
               (JmValue[]){jm_value_int(0)}, 1, 1);
    expect_bool("中断解除(0) = 真", "中断解除",
                (JmValue[]){jm_value_int(0)}, 1, 1);
    expect_bool("解除后 中断触发(0) = 假", "中断触发",
                (JmValue[]){jm_value_int(0)}, 1, 0);
    expect_bool("清理队列 = 真", "队列销毁",
                (JmValue[]){jm_value_int(queue_handle)}, 1, 1);

    if (g_fail != 0) {
        printf("native 表运行期主机回归失败：%d 项\n", g_fail);
        return 1;
    }
    printf("native 表运行期主机回归：全部通过\n");
    return 0;
}

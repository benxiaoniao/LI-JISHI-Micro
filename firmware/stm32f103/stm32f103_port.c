#include <stdint.h>
#include "stm32f103_port.h"
#include "jm_port.h"
#include "jmk.h"

/* ---------------- 寄存器定义（F103 中容量，最小集） ---------------- */

#define REG32(addr) (*(volatile uint32_t *)(addr))

/* RCC */
#define RCC_CR          REG32(0x40021000u)
#define RCC_CFGR        REG32(0x40021004u)
#define RCC_APB2ENR     REG32(0x40021018u)
#define RCC_APB1ENR     REG32(0x4002101Cu)

#define RCC_CR_HSEON    (1u << 16)
#define RCC_CR_HSERDY   (1u << 17)
#define RCC_CR_PLLON    (1u << 24)
#define RCC_CR_PLLRDY   (1u << 25)

#define RCC_CFGR_SW_PLL     (2u << 0)
#define RCC_CFGR_PPRE1_DIV2 (4u << 8)
#define RCC_CFGR_PLLSRC     (1u << 16)   /* HSE 作为 PLL 输入 */
#define RCC_CFGR_PLLMUL9    (7u << 18)   /* 8 MHz × 9 = 72 MHz */

#define RCC_APB2ENR_IOPAEN  (1u << 2)
#define RCC_APB2ENR_IOPBEN  (1u << 3)
#define RCC_APB2ENR_IOPCEN  (1u << 4)
#define RCC_APB2ENR_ADC1EN  (1u << 9)
#define RCC_APB2ENR_USART1EN (1u << 14)
#define RCC_APB2ENR_AFIOEN  (1u << 0)

#define RCC_APB1ENR_TIM2EN  (1u << 0)
#define RCC_APB1ENR_TIM4EN  (1u << 2)
#define RCC_APB1ENR_I2C2EN  (1u << 22)

/* ADC 预分频：时钟 / 6（72 MHz → 12 MHz，上限 14 MHz） */
#define RCC_CFGR_ADCPRE_MASK  (3u << 14)
#define RCC_CFGR_ADCPRE_DIV6  (2u << 14)

/* FLASH */
#define FLASH_ACR       REG32(0x40022000u)
#define FLASH_ACR_LATENCY2 (2u << 0)
#define FLASH_ACR_PRFTBE   (1u << 4)

/* GPIOA / GPIOB / GPIOC */
#define GPIOA_CRL       REG32(0x40010800u)
#define GPIOA_CRH       REG32(0x40010804u)
#define GPIOA_IDR       REG32(0x40010808u)
#define GPIOA_ODR       REG32(0x4001080Cu)
#define GPIOB_CRL       REG32(0x40010C00u)
#define GPIOB_CRH       REG32(0x40010C04u)
#define GPIOB_IDR       REG32(0x40010C08u)
#define GPIOB_ODR       REG32(0x40010C0Cu)
#define GPIOC_CRL       REG32(0x40011000u)
#define GPIOC_CRH       REG32(0x40011004u)
#define GPIOC_IDR       REG32(0x40011008u)
#define GPIOC_ODR       REG32(0x4001100Cu)
#define GPIOC_BSRR      REG32(0x40011010u)
#define GPIOC_BRR       REG32(0x40011014u)

#define LED_PIN         13u

/* USART1 */
#define USART1_SR       REG32(0x40013800u)
#define USART1_DR       REG32(0x40013804u)
#define USART1_BRR      REG32(0x40013808u)
#define USART1_CR1      REG32(0x4001380Cu)

#define USART_SR_TXE    (1u << 7)
#define USART_CR1_RE    (1u << 2)
#define USART_CR1_TE    (1u << 3)
#define USART_CR1_UE    (1u << 13)

/* SysTick */
#define STK_CTRL        REG32(0xE000E010u)
#define STK_LOAD        REG32(0xE000E014u)
#define STK_VAL         REG32(0xE000E018u)

/* ADC1（PA0–PA7 = 通道 0–7） */
#define ADC1_SR         REG32(0x40012400u)
#define ADC1_CR1        REG32(0x40012404u)
#define ADC1_CR2        REG32(0x40012408u)
#define ADC1_SMPR2      REG32(0x40012410u)
#define ADC1_SQR1       REG32(0x4001242Cu)
#define ADC1_SQR3       REG32(0x40012434u)
#define ADC1_DR         REG32(0x4001244Cu)

#define ADC_SR_EOC         (1u << 1)
#define ADC_CR2_ADON       (1u << 0)
#define ADC_CR2_CAL        (1u << 2)
#define ADC_CR2_RSTCAL     (1u << 3)
#define ADC_CR2_EXTTRIG    (1u << 20)
#define ADC_CR2_EXTSEL_SWSTART (7u << 17)
#define ADC_CR2_SWSTART    (1u << 22)

/* TIM2（1 kHz 自由运行） */
#define TIM2_CR1        REG32(0x40000000u)
#define TIM2_EGR        REG32(0x40000014u)
#define TIM2_CNT        REG32(0x40000024u)
#define TIM2_PSC        REG32(0x40000028u)
#define TIM2_ARR        REG32(0x4000002Cu)

/* TIM4（PB6–PB9 = CH1–CH4，默认映射，无需 AFIO 重映射）
 * APB1 预分频 /2 时定时器时钟 = 2 × PCLK1 = 72 MHz（与 g_sysclk 一致）。 */
#define TIM4_CR1        REG32(0x40000800u)
#define TIM4_EGR        REG32(0x40000814u)
#define TIM4_CCMR1      REG32(0x40000818u)
#define TIM4_CCMR2      REG32(0x4000081Cu)
#define TIM4_CCER       REG32(0x40000820u)
#define TIM4_CNT        REG32(0x40000824u)
#define TIM4_PSC        REG32(0x40000828u)
#define TIM4_ARR        REG32(0x4000082Cu)
#define TIM4_CCR1       REG32(0x40000834u)
#define TIM4_CCR2       REG32(0x40000838u)
#define TIM4_CCR3       REG32(0x4000083Cu)
#define TIM4_CCR4       REG32(0x40000840u)

/* I2C2（PB10 = SCL / PB11 = SDA，默认映射，无需 AFIO 重映射）
 * I2C2 挂在 APB1：72 MHz 主频时 PCLK1 = 36 MHz（PPRE1=/2），
 * 所以 CCR / TRISE 必须按 36 MHz 算；若 HSE 缺席退回 HSI 8 MHz，则按 8 MHz 算。 */
#define I2C2_CR1        REG32(0x40005800u)
#define I2C2_CR2        REG32(0x40005804u)
#define I2C2_DR         REG32(0x40005810u)
#define I2C2_SR1        REG32(0x40005814u)
#define I2C2_SR2        REG32(0x40005818u)
#define I2C2_CCR        REG32(0x4000581Cu)
#define I2C2_TRISE      REG32(0x40005820u)

#define I2C_CR1_PE      (1u << 0)
#define I2C_CR1_START   (1u << 8)
#define I2C_CR1_STOP    (1u << 9)
#define I2C_CR1_SWRST   (1u << 15)
#define I2C_SR1_SB      (1u << 0)
#define I2C_SR1_ADDR    (1u << 1)
#define I2C_SR1_BTF     (1u << 2)
#define I2C_SR1_TXE     (1u << 7)
#define I2C_SR1_AF      (1u << 10)
#define I2C_CCR_FS      (1u << 15)

/* SPI1（APB2） */
#define SPI1_CR1        REG32(0x40013000u)
#define SPI1_SR         REG32(0x40013008u)
#define SPI1_DR         REG32(0x4001300Cu)

#define RCC_APB2ENR_SPI1EN  (1u << 12)
#define SPI_CR1_MSTR    (1u << 2)
#define SPI_CR1_SPE     (1u << 6)
#define SPI_CR1_SSI     (1u << 8)
#define SPI_CR1_SSM     (1u << 9)
#define SPI_SR_RXNE     (1u << 0)
#define SPI_SR_TXE      (1u << 1)

/* EXTI：编码引脚 0–15 = PA0–PA15，16–31 = PB0–PB15，32–47 = PC0–PC15；
 * EXTI 线号 = 引脚 % 16，端口选择写 AFIO_EXTICR1–4。 */
#define AFIO_EXTICR1    REG32(0x40010008u)
#define AFIO_EXTICR2    REG32(0x4001000Cu)
#define AFIO_EXTICR3    REG32(0x40010010u)
#define AFIO_EXTICR4    REG32(0x40010014u)
#define EXTI_IMR        REG32(0x40010400u)
#define EXTI_EMR        REG32(0x40010404u)
#define EXTI_RTSR       REG32(0x40010408u)
#define EXTI_FTSR       REG32(0x4001040Cu)
#define EXTI_SWIER      REG32(0x40010410u)
#define EXTI_PR         REG32(0x40010414u)

#define NVIC_ISER0      REG32(0xE000E100u)
#define NVIC_ISER1      REG32(0xE000E104u)
#define NVIC_ICER0      REG32(0xE000E100u + 0x80u)
#define NVIC_ICER1      REG32(0xE000E100u + 0x84u)
#define NVIC_IPR_BYTE(irq) (*(volatile uint8_t *)(0xE000E400u + (irq)))

#define EXTI_LINES          48u   /* 3 个端口 × 16 条线 */
#define EXTI_PINS_PER_PORT  16u
#define EXTI_IRQ_FIRST      6u    /* EXTI0 的 IRQ 号；EXTI1–4 依次 +1 */
#define EXTI_IRQ_5_9        23u   /* EXTI9_5 */
#define EXTI_IRQ_10_15      40u   /* EXTI15_10 */
#define EXTI_IRQ_PRIO       PORT_PRIO_NORMAL /* 普通层：低于紧急层，高于慢速层与内核 tick */

/* 内核向量优先级寄存器：SVC 在 SHPR2 最高字节；PendSV/SysTick 在 SHPR3 高两字节。 */
#define SCB_SHPR2       REG32(0xE000ED1Cu)
#define SCB_SHPR3       REG32(0xE000ED20u)
/* 故障状态寄存器；HardFault 现场打印用。 */
#define SCB_CFSR        REG32(0xE000ED28u)
#define SCB_HFSR        REG32(0xE000ED2Cu)
#define SCB_MMFAR       REG32(0xE000ED34u)
#define SCB_BFAR        REG32(0xE000ED38u)

#define UART_BAUD       115200u

/* ---------------- 状态 ---------------- */

static uint32_t g_sysclk = 8000000u;
static uint32_t g_pclk1 = 8000000u;   /* APB1 时钟：I2C2 的 CCR/TRISE 按它算 */
static volatile uint32_t g_ms = 0;
static volatile uint8_t g_adc_ready = 0u;
static volatile uint8_t g_timer_ready = 0u;

/* EXTI：中断里只写计数并回调 hook，不进入 VM */
static volatile uint32_t g_exti_count[EXTI_LINES];
static void (*g_exti_hook)(uint32_t line) = 0;
static uint8_t g_exti_bound[EXTI_LINES];
/* 每条 EXTI 线当前绑定的编码引脚 + 1；0 = 未绑定（静态零初始化即可） */
static uint8_t g_exti_line_pin[EXTI_PINS_PER_PORT];

/* ---------------- 时钟 ---------------- */

static uint32_t clock_init(void)
{
    uint32_t timeout = 0x40000u;

    /* 尝试 HSE 8 MHz：8 × 9 = 72 MHz */
    RCC_CR |= RCC_CR_HSEON;
    while ((RCC_CR & RCC_CR_HSERDY) == 0u && timeout > 0u) {
        timeout--;
    }

    if ((RCC_CR & RCC_CR_HSERDY) != 0u) {
        /* 72 MHz 需要 2 个等待周期 */
        FLASH_ACR = FLASH_ACR_LATENCY2 | FLASH_ACR_PRFTBE;
        RCC_CFGR = RCC_CFGR_PPRE1_DIV2 | RCC_CFGR_PLLSRC | RCC_CFGR_PLLMUL9;

        RCC_CR |= RCC_CR_PLLON;
        while ((RCC_CR & RCC_CR_PLLRDY) == 0u) {
        }

        RCC_CFGR |= RCC_CFGR_SW_PLL;
        while (((RCC_CFGR >> 2) & 0x3u) != 0x2u) {
        }

        g_sysclk = 72000000u;
        g_pclk1 = 36000000u;   /* PPRE1 = /2 */
    } else {
        /* HSE 缺席（无外部晶振）：退回内部 HSI 8 MHz */
        g_sysclk = 8000000u;
        g_pclk1 = 8000000u;    /* 兜底路径没有改 PPRE1，APB1 = 8 MHz */
    }
    return g_sysclk;
}

/* ---------------- UART ---------------- */

static void uart_putc(char c)
{
    while ((USART1_SR & USART_SR_TXE) == 0u) {
    }
    USART1_DR = (uint32_t)(unsigned char)c;
}

static void uart_init(uint32_t pclk2, uint32_t baud)
{
    /* 打开 GPIOA / USART1 时钟 */
    RCC_APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_USART1EN;

    /* PA9 = 复用推挽 50 MHz，PA10 = 浮空输入 */
    GPIOA_CRH = (GPIOA_CRH & ~((0xFu << 4) | (0xFu << 8)))
              | (0xBu << 4)
              | (0x4u << 8);

    USART1_BRR = (pclk2 + baud / 2u) / baud;
    USART1_CR1 = USART_CR1_UE | USART_CR1_TE | USART_CR1_RE;
}

/* ---------------- 中断优先级 ---------------- */

int port_irq_set_priority(uint32_t irq, uint8_t prio)
{
    /* 外部 IRQ 只允许三层；0x00 留给故障/SVC，0xFF 留给 SysTick/PendSV。 */
    if (prio != PORT_PRIO_CRITICAL && prio != PORT_PRIO_NORMAL &&
        prio != PORT_PRIO_SLOW) {
        return -1;
    }
    if (irq >= 68u) {
        return -1;
    }
    NVIC_IPR_BYTE(irq) = (uint8_t)(prio & 0xF0u);
    return 0;
}

uint8_t port_irq_get_priority(uint32_t irq)
{
    if (irq >= 68u) {
        return 0xFFu;
    }
    return (uint8_t)(NVIC_IPR_BYTE(irq) & 0xF0u);
}

/* 启动期先设好内核向量，再使能 tick；避免 SysTick 以复位默认 0x00 运行。 */
static void irq_init_priorities(void)
{
    SCB_SHPR2 = (SCB_SHPR2 & ~(0xFFu << 24)) | ((uint32_t)PORT_PRIO_FAULT << 24);
    SCB_SHPR3 = (SCB_SHPR3 & ~((0xFFu << 16) | (0xFFu << 24)))
              | ((uint32_t)PORT_PRIO_KERNEL << 16)
              | ((uint32_t)PORT_PRIO_KERNEL << 24);
#if JM_IRQ_DEBUG
    port_log("[中断] SysTick/PendSV 优先级 = ");
    port_log_hex((SCB_SHPR3 >> 16) & 0xFFu);
    port_log(" / ");
    port_log_hex((SCB_SHPR3 >> 24) & 0xFFu);
    port_log("，SVC = ");
    port_log_hex((SCB_SHPR2 >> 24) & 0xFFu);
    port_log("\r\n");
#endif
}

/* ---------------- MSP 哨兵水位 ---------------- */

#if JM_MSP_WATERMARK
extern uint32_t _ebss;
extern uint32_t _estack;

#define MSP_PATTERN 0xA5A5A5A5u
#define MSP_GUARD   256u   /* 给填充函数自身与可能的中断留出的安全区 */

static uint32_t *g_msp_lo;
static uint32_t *g_msp_hi;
static uint8_t g_msp_ready;

void port_msp_watermark_init(void)
{
    uint32_t sp;
    uint32_t *lo = &_ebss;
    uint32_t *hi;
    uint32_t *p;

    __asm volatile ("mrs %0, msp" : "=r"(sp));
    hi = (uint32_t *)((sp - MSP_GUARD) & ~3u);
    if ((uint32_t)hi <= (uint32_t)lo) {
        return;
    }
    g_msp_lo = lo;
    g_msp_hi = hi;
    for (p = lo; p < hi; p++) {
        *p = MSP_PATTERN;
    }
    g_msp_ready = 1u;
}

uint32_t port_msp_watermark_used(void)
{
    uint32_t top = (uint32_t)&_estack;
    uint32_t *p;

    if (!g_msp_ready) {
        return 0u;
    }
    /* 栈向低地址生长：从低往高扫，第一个被改写的字就是最深水位。 */
    for (p = g_msp_lo; p < g_msp_hi; p++) {
        if (*p != MSP_PATTERN) {
            return top - (uint32_t)p;
        }
    }
    /* 填充区内没被用到：返回可测区上界（含 256 B 保护带，偏保守）。 */
    return top - (uint32_t)g_msp_hi;
}

uint32_t port_msp_watermark_total(void)
{
    return (uint32_t)&_estack - (uint32_t)&_ebss;
}
#endif

/* ---------------- SysTick ---------------- */

static void systick_init(uint32_t clk)
{
    STK_LOAD = (clk / 1000u) - 1u;
    STK_VAL = 0u;
    STK_CTRL = 0x7u; /* CLKSOURCE=CPU / TICKINT / ENABLE */
}

/* SysTick 中断：1 ms 一次，由向量表直接引用 */
void SysTick_Handler(void)
{
    g_ms++;
    jmk_tick();
}

/* ---------------- 对外接口 ---------------- */

void port_init(void)
{
    uint32_t clk;
#if JM_MSP_WATERMARK
    port_msp_watermark_init();   /* 最早调用：从第一条栈用起就能被哨兵记录 */
#endif
    clk = clock_init();
    uart_init(clk, UART_BAUD);
    irq_init_priorities();   /* 必须先于 systick_init：tick 从第一条起就是最低优先级 */
    systick_init(clk);

    /* PC13 推挽输出，默认熄灭（高电平） */
    RCC_APB2ENR |= RCC_APB2ENR_IOPCEN;
    GPIOC_CRH = (GPIOC_CRH & ~(0xFu << 20)) | (0x2u << 20);
    GPIOC_BSRR = (1u << LED_PIN);
}

uint32_t port_sysclk(void)
{
    return g_sysclk;
}

void port_log(const char *s)
{
    while (*s != '\0') {
        uart_putc(*s);
        s++;
    }
}

void port_log_u32(uint32_t v)
{
    char buf[11];
    int i = 0;

    if (v == 0u) {
        uart_putc('0');
        return;
    }
    while (v > 0u && i < (int)sizeof(buf)) {
        buf[i++] = (char)('0' + (v % 10u));
        v /= 10u;
    }
    while (i > 0) {
        uart_putc(buf[--i]);
    }
}

void port_log_hex(uint32_t v)
{
    int i;
    port_log("0x");
    for (i = 28; i >= 0; i -= 4) {
        uint32_t d = (v >> (uint32_t)i) & 0xFu;
        uart_putc((char)(d < 10u ? ('0' + d) : ('a' + (d - 10u))));
    }
}

/*
 * HardFault 现场打印：由 startup_stm32f103c8.s 的 HardFault_Handler 调用，不返回。
 * EXC_RETURN 的 bit2=1 表示异常帧在 PSP（任务栈），否则在 MSP（主栈）。
 */
void hardfault_report(uint32_t exc_return, uint32_t msp, uint32_t psp)
{
    uint32_t frame = (exc_return & 4u) ? psp : msp;
    uint32_t *f = (uint32_t *)frame;
    uint32_t cfsr;

    /* UART 尚未初始化时不要卡在 uart_putc 的等待里。 */
    if ((USART1_CR1 & USART_CR1_UE) == 0u) {
        for (;;) {
        }
    }

    cfsr = SCB_CFSR;
    port_log("\r\n[故障] HardFault\r\n");
    port_log("EXC_RETURN=");
    port_log_hex(exc_return);
    port_log(" MSP=");
    port_log_hex(msp);
    port_log(" PSP=");
    port_log_hex(psp);
    port_log("\r\n");
    port_log("CFSR=");
    port_log_hex(cfsr);
    port_log(" HFSR=");
    port_log_hex(SCB_HFSR);
    port_log(" MMFAR=");
    if ((cfsr & (1u << 7)) != 0u) {   /* MMARVALID */
        port_log_hex(SCB_MMFAR);
    } else {
        port_log("-");
    }
    port_log(" BFAR=");
    if ((cfsr & (1u << 15)) != 0u) {  /* BFARVALID */
        port_log_hex(SCB_BFAR);
    } else {
        port_log("-");
    }
    port_log("\r\n");
    port_log("PC=");
    port_log_hex(f[6]);
    port_log(" LR=");
    port_log_hex(f[5]);
    port_log(" xPSR=");
    port_log_hex(f[7]);
    port_log("\r\n");
    port_log("[故障] 已停机，复位后重新运行\r\n");

    for (;;) {
    }
}

/* newlib 的 printf / puts 最终会走到这里，统一从 USART1 输出 */
int _write(int file, char *ptr, int len)
{
    int i;
    (void)file;
    for (i = 0; i < len; i++) {
        uart_putc(ptr[i]);
    }
    return len;
}

uint32_t port_uptime_ms(void)
{
    return g_ms;
}

void port_led_set(int on)
{
    if (on) {
        GPIOC_BRR = (1u << LED_PIN);   /* 低电平点亮 */
    } else {
        GPIOC_BSRR = (1u << LED_PIN);
    }
}

void port_led_toggle(void)
{
    static int state = 0;
    state = !state;
    port_led_set(state);
}

/* ---------------- 板级 GPIO：引脚编码 0–47（0–15=PA, 16–31=PB, 32–47=PC） ---------------- */

/* gpio_reg / gpio_set_mode 定义在后面的 I2C/SPI 段，这里先声明，保证前面可用。 */
static volatile uint32_t *gpio_reg(uint32_t port, uint32_t off);
static void gpio_set_mode(uint32_t pin, uint32_t mode);

/* 最后一次写入的电平（按引脚编码索引）；从未写过按 0 处理。 */
static uint8_t g_pin_out[48];

void port_gpio_write(uint32_t pin, int level)
{
    volatile uint32_t *set;
    volatile uint32_t *reset;
    uint32_t bit;

    if (pin > 47u) {
        return;
    }
    gpio_set_mode(pin, 0x2u);            /* 推挽输出 2 MHz（够 CS/普通 GPIO） */
    set = gpio_reg(pin / 16u, 0x10u);    /* BSRR */
    reset = gpio_reg(pin / 16u, 0x14u);  /* BRR */
    bit = 1u << (pin % 16u);
    if (level) {
        *set = bit;
        g_pin_out[pin] = 1u;
    } else {
        *reset = bit;
        g_pin_out[pin] = 0u;
    }
}

int port_gpio_read(uint32_t pin)
{
    if (pin > 47u) {
        return -1;
    }
    return (int)((*gpio_reg(pin / 16u, 0x08u) >> (pin % 16u)) & 1u);
}

int port_gpio_toggle(uint32_t pin)
{
    if (pin > 47u) {
        return -1;
    }
    port_gpio_write(pin, g_pin_out[pin] ? 0 : 1);
    return (int)g_pin_out[pin];
}

/* ---- ISR 快路径：不改引脚配置，只写 BSRR/BRR 与 g_pin_out ----
 * 前置条件是任务上下文已把该脚配成通用推挽输出；未配置时由调用方返回 E033。
 * 这里只做单寄存器写与单字节读写，可在中断里与任务并发。
 */
int port_gpio_is_output(uint32_t pin)
{
    uint32_t line;
    uint32_t nibble;

    if (pin > 47u) {
        return 0;
    }
    line = pin % 16u;
    nibble = (*gpio_reg(pin / 16u, (line < 8u) ? 0x00u : 0x04u)
              >> ((line & 7u) * 4u)) & 0xFu;
    return ((nibble & 0x3u) != 0u) && ((nibble & 0xCu) == 0u);
}

void port_gpio_write_fast(uint32_t pin, int level)
{
    uint32_t bit;

    if (pin > 47u) {
        return;
    }
    bit = 1u << (pin % 16u);
    if (level) {
        *gpio_reg(pin / 16u, 0x10u) = bit;   /* BSRR */
        g_pin_out[pin] = 1u;
    } else {
        *gpio_reg(pin / 16u, 0x14u) = bit;   /* BRR */
        g_pin_out[pin] = 0u;
    }
}

void port_gpio_toggle_fast(uint32_t pin)
{
    if (pin > 47u) {
        return;
    }
    port_gpio_write_fast(pin, g_pin_out[pin] ? 0 : 1);
}

void port_delay_ms(uint32_t ms)
{
    uint32_t start = g_ms;
    while ((uint32_t)(g_ms - start) < ms) {
        /* 忙等：SysTick 中断在跑，g_ms 会自增 */
    }
}

/* ---------------- ADC1（PA0–PA7 = 通道 0–7） ---------------- */

void port_adc_init(void)
{
    uint32_t i;

    RCC_APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_ADC1EN;
    /* ADC 时钟 = PCLK2 / 6：72 MHz 时 12 MHz，未超过 14 MHz 上限 */
    RCC_CFGR = (RCC_CFGR & ~RCC_CFGR_ADCPRE_MASK) | RCC_CFGR_ADCPRE_DIV6;

    /* PA0–PA7 全部设为模拟输入（每个引脚 4 位，0x8 = 输入/模拟） */
    GPIOA_CRL = 0x88888888u;

    ADC1_CR1 = 0u;
    ADC1_CR2 = ADC_CR2_ADON;
    for (i = 0u; i < 1000u; i++) {
        /* 上电稳定：ADON 置位后需要一段建立时间 */
    }

    /* 先复位校准，再做校准；两位都由硬件清零 */
    ADC1_CR2 |= ADC_CR2_RSTCAL;
    while ((ADC1_CR2 & ADC_CR2_RSTCAL) != 0u) {
    }
    ADC1_CR2 |= ADC_CR2_CAL;
    while ((ADC1_CR2 & ADC_CR2_CAL) != 0u) {
    }

    /* 采样时间 239.5 周期（通道 0–9 全设），提高高阻信号源的稳定性 */
    ADC1_SMPR2 = 0x3FFFFFFFu;
    ADC1_SQR1 = 0u;                       /* 规则组长度 1（L=0） */
    /* 软件触发：SWSTART + 外部触发使能，避免 ADON 直接启动转换 */
    ADC1_CR2 |= ADC_CR2_EXTTRIG | ADC_CR2_EXTSEL_SWSTART;

    g_adc_ready = 1u;
}

int port_adc_read(uint32_t channel, uint32_t *raw)
{
    if (g_adc_ready == 0u || raw == 0) {
        return -1;
    }
    if (channel > 7u) {
        return -1;
    }

    ADC1_SQR3 = channel & 0x1Fu;
    ADC1_CR2 |= ADC_CR2_SWSTART;
    while ((ADC1_SR & ADC_SR_EOC) == 0u) {
    }
    *raw = ADC1_DR & 0x0FFFu;   /* 读 DR 同时清 EOC */
    return 0;
}

/* ---------------- 定时器（毫秒基准） ----------------
 * 注意：TIM2 的 PSC 是 16 位，72 MHz 下分到 1 kHz 需要 71999，超出 16 位——
 * 原来的写法写进寄存器后只剩低 16 位（6463），实际计数约 11.1 kHz，「毫秒」是错的。
 * 毫秒统一改用 SysTick 的 1 ms 计数 g_ms；TIM2 只留作 1 MHz 自由运行（PSC=71）。
 */

void port_timer_init(void)
{
    RCC_APB1ENR |= RCC_APB1ENR_TIM2EN;
    TIM2_CR1 = 0u;
    TIM2_PSC = 71u;       /* 72 MHz / 72 = 1 MHz：每计数 1 µs */
    TIM2_ARR = 0xFFFFu;   /* 16 位自由运行，约 65.5 ms 回绕（不用于毫秒） */
    TIM2_EGR = 1u;        /* UG：立即把 PSC/ARR 装进影子寄存器 */
    TIM2_CNT = 0u;
    TIM2_CR1 = 1u;        /* CEN = 1 */

    g_timer_ready = 1u;
}

int port_timer_delay_ms(uint32_t ms)
{
    uint32_t start;

    if (g_timer_ready == 0u || ms > 60000u) {
        return -1;
    }

    start = g_ms;
    while ((uint32_t)(g_ms - start) < ms) {
    }
    return 0;
}

int port_timer_count(uint32_t *ms)
{
    if (g_timer_ready == 0u || ms == 0) {
        return -1;
    }
    *ms = g_ms & 0xFFFFu;
    return 0;
}

/* ---------------- TIM4 PWM（PB6–PB9 = CH1–CH4） ----------------
 * 毫秒基准是 SysTick（g_ms），TIM2 只做 1 MHz 计数；这里用 TIM4，互不影响。
 * ARR 固定 999 → 一个周期正好 1000 个计数，CCR 直接写千分比（0–1000）。
 * PC13 只有 GPIO，没有定时器通道，不能做 PWM；PB6–PB9 上板可用。
 */

#define PWM_PIN_FIRST  22u    /* PB6 = TIM4_CH1 */
#define PWM_PIN_LAST   25u    /* PB9 = TIM4_CH4 */
#define PWM_STEPS      1000u  /* 常规档位每周期 1000 个计数（千分比直写） */

static uint8_t g_pwm_on = 0u;
static uint32_t g_pwm_ch = 0u;   /* 0–3 = CH1–CH4 */
static uint32_t g_pwm_arr1 = PWM_STEPS;   /* ARR+1：低频档会放大，占空比按它换算 */

static void pwm_write_ccr(uint32_t ch, uint32_t duty_permille)
{
    /* 占空比对外始终是千分比；CCR 按当前 ARR+1 换算，1 Hz 档 ARR 不是 999 */
    uint32_t ccr = (duty_permille * g_pwm_arr1) / PWM_STEPS;

    if (ccr > g_pwm_arr1) {
        ccr = g_pwm_arr1;      /* 1000‰ = 整周期常高（PWM 模式 1 下 CCR≥ARR+1） */
    }
    if (ccr > 65535u) {
        ccr = 65535u;
    }
    switch (ch) {
    case 0u: TIM4_CCR1 = ccr; break;
    case 1u: TIM4_CCR2 = ccr; break;
    case 2u: TIM4_CCR3 = ccr; break;
    default: TIM4_CCR4 = ccr; break;
    }
}

/* 引脚配成复用推挽 50 MHz（CR nibble = 0xB）；PB6/PB7 在 CRL，PB8/PB9 在 CRH */
static void pwm_pin_af(uint32_t line)
{
    uint32_t shift = (line & 7u) * 4u;

    RCC_APB2ENR |= RCC_APB2ENR_IOPBEN;
    if (line < 8u) {
        GPIOB_CRL = (GPIOB_CRL & ~(0xFu << shift)) | (0xBu << shift);
    } else {
        GPIOB_CRH = (GPIOB_CRH & ~(0xFu << shift)) | (0xBu << shift);
    }
}

int port_pwm_start(uint32_t pin, uint32_t freq_hz, uint32_t duty_permille)
{
    uint32_t line;
    uint32_t ch;
    uint32_t psc1;
    uint32_t arr1;
    uint32_t shift;

    if (pin < PWM_PIN_FIRST || pin > PWM_PIN_LAST || freq_hz == 0u ||
        freq_hz > 10000u || duty_permille > 1000u) {
        return -1;
    }

    line = pin % 16u;
    ch = line - 6u;   /* PB6–PB9 → CH1–CH4 */

    /* 周期 = 时钟 / 频率；PSC 粗分，ARR 细分。
     * 常规档位（PSC+1 ≤ 65536）保持旧行为：ARR+1 = 1000，千分比直写。
     * 低频端 1 Hz 需要 72000 个分频，PSC 装不下，才把 ARR+1 放大到
     * 一个 ≥1000 且能整除周期的因子（1 Hz → 1125，PSC+1 = 64000）。 */
    psc1 = g_sysclk / (freq_hz * PWM_STEPS);
    arr1 = PWM_STEPS;
    if (psc1 == 0u) {
        return -1;                    /* 8 MHz 兜底时 10 kHz 无解，保持原有失败 */
    }
    if (psc1 > 65536u) {
        uint32_t period = g_sysclk / freq_hz;   /* > 65 536 000，必然 > 0 */
        uint32_t d;

        arr1 = 0u;
        for (d = PWM_STEPS; d <= 65536u; d++) {
            if (period % d == 0u && period / d <= 65536u) {
                arr1 = d;
                break;
            }
        }
        if (arr1 == 0u) {
            /* 无整除解：PSC 取最大，ARR 取最近的整数细分（会有量化误差） */
            psc1 = (period + 65535u) / 65536u;
            arr1 = (period + psc1 / 2u) / psc1;
            if (arr1 < PWM_STEPS) {
                arr1 = PWM_STEPS;
            }
        } else {
            psc1 = period / arr1;
        }
    }
    if (psc1 == 0u || psc1 > 65536u || arr1 > 65536u) {
        return -1;
    }

    pwm_pin_af(line);
    RCC_APB1ENR |= RCC_APB1ENR_TIM4EN;

    g_pwm_arr1 = arr1;
    TIM4_CR1 = 0u;
    TIM4_PSC = psc1 - 1u;
    TIM4_ARR = arr1 - 1u;

    /* CH1/CH3 用低 8 位，CH2/CH4 用高 8 位；OCxM=110（PWM 模式 1），OCxPE=1 */
    shift = (ch & 1u) * 8u + 4u;
    if (ch < 2u) {
        TIM4_CCMR1 = (TIM4_CCMR1 & ~(0x7u << shift)) | (0x6u << shift)
                   | (1u << (shift - 1u));
    } else {
        TIM4_CCMR2 = (TIM4_CCMR2 & ~(0x7u << shift)) | (0x6u << shift)
                   | (1u << (shift - 1u));
    }

    TIM4_CCER |= (1u << (ch * 4u));   /* CCxE=1，CCxP=0：高电平有效 */
    pwm_write_ccr(ch, duty_permille);

    TIM4_EGR = 1u;   /* UG：立即装载 PSC/ARR */
    TIM4_CNT = 0u;
    TIM4_CR1 = 1u;   /* CEN=1 */

    g_pwm_on = 1u;
    g_pwm_ch = ch;
    return 0;
}

int port_pwm_set_duty(uint32_t duty_permille)
{
    if (g_pwm_on == 0u || duty_permille > 1000u) {
        return -1;
    }
    pwm_write_ccr(g_pwm_ch, duty_permille);
    return 0;
}

int port_pwm_stop(void)
{
    uint32_t line;
    uint32_t shift;

    if (g_pwm_on == 0u) {
        return -1;
    }

    TIM4_CCER &= ~(1u << (g_pwm_ch * 4u));
    TIM4_CR1 = 0u;

    /* 引脚从复用态改回普通推挽输出并拉低，避免停在高电平 */
    line = (PWM_PIN_FIRST + g_pwm_ch) % 16u;
    shift = (line & 7u) * 4u;
    if (line < 8u) {
        GPIOB_CRL = (GPIOB_CRL & ~(0xFu << shift)) | (0x2u << shift);
    } else {
        GPIOB_CRH = (GPIOB_CRH & ~(0xFu << shift)) | (0x2u << shift);
    }
    GPIOB_ODR &= ~(1u << line);

    g_pwm_on = 0u;
    return 0;
}

/* ---------------- I2C：硬件 I2C2 + 软件位翻转 ----------------
 * 一次调用 = 一次完整事务（START … STOP），不跨调用保持总线。
 * SSD1306 只要「地址 + 控制字节 + 1–6 个数据字节」，所以不做缓冲队列。
 * 引脚编码：0–15 = PA0–PA15，16–31 = PB0–PB15，32–47 = PC0–PC15。
 */

#define I2C_PIN_HW_SCL 26u   /* PB10 */
#define I2C_PIN_HW_SDA 27u   /* PB11 */

static uint8_t g_i2c_ready = 0u;
static uint8_t g_i2c_hw = 0u;
static uint32_t g_i2c_scl = 0u;
static uint32_t g_i2c_sda = 0u;

static volatile uint32_t *gpio_reg(uint32_t port, uint32_t off)
{
    return (volatile uint32_t *)(0x40010800u + port * 0x400u + off);
}

/* 通用开漏输出 50 MHz（CR nibble = 0x7），依赖外部上拉 */
static void i2c_gpio_open_drain(uint32_t pin)
{
    uint32_t port = pin / 16u;
    uint32_t line = pin % 16u;
    uint32_t shift = (line & 7u) * 4u;
    volatile uint32_t *cr = gpio_reg(port, (line < 8u) ? 0x00u : 0x04u);

    RCC_APB2ENR |= (1u << (2u + port));   /* IOPAEN / IOPBEN / IOPCEN */
    *cr = (*cr & ~(0xFu << shift)) | (0x7u << shift);
}

static void bb_write(uint32_t pin, uint32_t level)
{
    volatile uint32_t *odr = gpio_reg(pin / 16u, 0x0Cu);
    if (level != 0u) {
        *odr |= (1u << (pin % 16u));
    } else {
        *odr &= ~(1u << (pin % 16u));
    }
}

static uint32_t bb_read(uint32_t pin)
{
    return (*gpio_reg(pin / 16u, 0x08u) >> (pin % 16u)) & 1u;
}

/* 约 1–2 µs 的半位延时：几十 kHz 量级，不求精确 */
static void bb_delay(void)
{
    volatile uint32_t i;
    for (i = 0u; i < 30u; i++) {
    }
}

static void bb_start(void)
{
    bb_write(g_i2c_sda, 1u);
    bb_write(g_i2c_scl, 1u);
    bb_delay();
    bb_write(g_i2c_sda, 0u);
    bb_delay();
    bb_write(g_i2c_scl, 0u);
    bb_delay();
}

static void bb_stop(void)
{
    bb_write(g_i2c_sda, 0u);
    bb_delay();
    bb_write(g_i2c_scl, 1u);
    bb_delay();
    bb_write(g_i2c_sda, 1u);
    bb_delay();
}

/* 返回 0 = 收到 ACK */
static uint32_t bb_write_byte(uint32_t byte)
{
    uint32_t i;
    uint32_t ack;

    for (i = 0u; i < 8u; i++) {
        bb_write(g_i2c_sda, (byte & 0x80u) ? 1u : 0u);
        bb_delay();
        bb_write(g_i2c_scl, 1u);
        bb_delay();
        bb_write(g_i2c_scl, 0u);
        bb_delay();
        byte <<= 1;
    }

    bb_write(g_i2c_sda, 1u);
    bb_delay();
    bb_write(g_i2c_scl, 1u);
    bb_delay();
    ack = bb_read(g_i2c_sda);
    bb_write(g_i2c_scl, 0u);
    bb_delay();
    return ack;
}

static int i2c2_wait(uint32_t flag, uint32_t fail_flag)
{
    uint32_t timeout = 200000u;

    while ((I2C2_SR1 & flag) == 0u) {
        if (fail_flag != 0u && (I2C2_SR1 & fail_flag) != 0u) {
            return -1;
        }
        if (--timeout == 0u) {
            return -1;
        }
    }
    return 0;
}

static void i2c2_clear_addr(void)
{
    (void)I2C2_SR1;
    (void)I2C2_SR2;
}

static int i2c2_init(uint32_t speed_khz)
{
    uint32_t pclk1_mhz = g_pclk1 / 1000000u;
    uint32_t pclk1_khz = g_pclk1 / 1000u;   /* CCR 的分频必须按 kHz 算，别把 Hz 直接除 */
    uint32_t ccr;

    RCC_APB2ENR |= RCC_APB2ENR_IOPBEN;
    RCC_APB1ENR |= RCC_APB1ENR_I2C2EN;

    /* PB10 / PB11 配成复用开漏 50 MHz：CRH 的 bit8–11 / bit12–15 */
    GPIOB_CRH = (GPIOB_CRH & ~((0xFu << 8) | (0xFu << 12)))
              | (0xFu << 8) | (0xFu << 12);

    I2C2_CR1 = I2C_CR1_SWRST;   /* 软复位，清掉可能残留的忙状态 */
    I2C2_CR1 = 0u;

    /* FREQ[5:0] 必须写 APB1 的 MHz 数：36 MHz → 36，8 MHz → 8 */
    I2C2_CR2 = pclk1_mhz;

    if (speed_khz <= 100u) {
        ccr = pclk1_khz / (2u * speed_khz);          /* 标准模式：Thigh = Tlow = CCR × Tpclk1 */
        if (ccr < 4u) {
            ccr = 4u;
        }
        I2C2_CCR = ccr;
        I2C2_TRISE = pclk1_mhz + 1u;                 /* 1000 ns × fPCLK1 + 1 */
    } else {
        ccr = pclk1_khz / (3u * speed_khz);          /* 快速模式：Tlow = 2 × Thigh */
        if (ccr < 1u) {
            ccr = 1u;
        }
        I2C2_CCR = I2C_CCR_FS | ccr;
        I2C2_TRISE = (pclk1_mhz * 3u) / 10u + 1u;    /* 300 ns × fPCLK1 + 1 */
    }

    I2C2_CR1 = I2C_CR1_PE;
    return 0;
}

static int i2c2_begin(uint32_t addr)
{
    I2C2_CR1 |= I2C_CR1_START;
    if (i2c2_wait(I2C_SR1_SB, 0u) != 0) {
        I2C2_CR1 |= I2C_CR1_STOP;
        return -1;
    }
    I2C2_DR = addr << 1;                 /* 写方向：R/W = 0 */
    if (i2c2_wait(I2C_SR1_ADDR, I2C_SR1_AF) != 0) {
        I2C2_SR1 &= ~I2C_SR1_AF;         /* 清 AF */
        I2C2_CR1 |= I2C_CR1_STOP;
        return -1;
    }
    i2c2_clear_addr();
    return 0;
}

static int i2c2_send(uint32_t byte)
{
    if (i2c2_wait(I2C_SR1_TXE, I2C_SR1_AF) != 0) {
        return -1;
    }
    I2C2_DR = byte;
    return 0;
}

static int i2c2_write(uint32_t addr, uint32_t ctrl, const uint32_t *data, uint32_t len)
{
    uint32_t i;

    if (i2c2_begin(addr) != 0) {
        return -1;
    }
    if (i2c2_send(ctrl) != 0) {
        I2C2_CR1 |= I2C_CR1_STOP;
        return -1;
    }
    for (i = 0u; i < len; i++) {
        if (i2c2_send(data[i]) != 0) {
            I2C2_CR1 |= I2C_CR1_STOP;
            return -1;
        }
    }
    if (i2c2_wait(I2C_SR1_BTF, 0u) != 0) {
        I2C2_CR1 |= I2C_CR1_STOP;
        return -1;
    }
    I2C2_CR1 |= I2C_CR1_STOP;
    return 0;
}

static int i2c2_probe(uint32_t addr)
{
    I2C2_CR1 |= I2C_CR1_START;
    if (i2c2_wait(I2C_SR1_SB, 0u) != 0) {
        I2C2_CR1 |= I2C_CR1_STOP;
        return 0;
    }
    I2C2_DR = addr << 1;
    if (i2c2_wait(I2C_SR1_ADDR, I2C_SR1_AF) != 0) {
        I2C2_SR1 &= ~I2C_SR1_AF;
        I2C2_CR1 |= I2C_CR1_STOP;
        return 0;
    }
    i2c2_clear_addr();
    I2C2_CR1 |= I2C_CR1_STOP;
    return 1;
}

int port_i2c_init(uint32_t scl_pin, uint32_t sda_pin, uint32_t speed_khz)
{
    if (scl_pin > 47u || sda_pin > 47u || speed_khz < 10u || speed_khz > 400u ||
        scl_pin == sda_pin) {
        return -1;
    }

    g_i2c_ready = 0u;
    g_i2c_scl = scl_pin;
    g_i2c_sda = sda_pin;

    if (scl_pin == I2C_PIN_HW_SCL && sda_pin == I2C_PIN_HW_SDA) {
        if (i2c2_init(speed_khz) != 0) {
            return -1;
        }
        g_i2c_hw = 1u;
    } else {
        i2c_gpio_open_drain(scl_pin);
        i2c_gpio_open_drain(sda_pin);
        bb_write(sda_pin, 1u);           /* 释放总线，靠外部上拉拉高 */
        bb_write(scl_pin, 1u);
        g_i2c_hw = 0u;
    }

    g_i2c_ready = 1u;
    return (int)g_i2c_hw;                /* 1 = 硬件 I2C2，0 = 软件位翻转 */
}

int port_i2c_write(uint32_t addr, uint32_t ctrl, const uint32_t *data, uint32_t len)
{
    uint32_t i;

    if (g_i2c_ready == 0u || addr > 127u || ctrl > 255u || len == 0u) {
        return -1;
    }

    if (g_i2c_hw != 0u) {
        return i2c2_write(addr, ctrl, data, len);
    }

    bb_start();
    if (bb_write_byte(addr << 1) != 0u) {
        bb_stop();
        return -1;
    }
    if (bb_write_byte(ctrl) != 0u) {
        bb_stop();
        return -1;
    }
    for (i = 0u; i < len; i++) {
        if (bb_write_byte(data[i]) != 0u) {
            bb_stop();
            return -1;
        }
    }
    bb_stop();
    return 0;
}

int port_i2c_probe(uint32_t addr)
{
    if (g_i2c_ready == 0u || addr > 127u) {
        return -1;
    }

    if (g_i2c_hw != 0u) {
        return i2c2_probe(addr);
    }

    {
        uint32_t ack;
        bb_start();
        ack = bb_write_byte(addr << 1);
        bb_stop();
        return (ack == 0u) ? 1 : 0;
    }
}

/* ---------------- SPI（PA5/PA6/PA7 → 硬件 SPI1，其它引脚 → 软件位翻转） ---------------- */

#define SPI_PIN_HW_SCK  5u    /* PA5 */
#define SPI_PIN_HW_MISO 6u    /* PA6 */
#define SPI_PIN_HW_MOSI 7u    /* PA7 */

/* GPIO CR nibble 的常用模式（F103 参考手册 9.1.11） */
#define GPIO_MODE_OUT_PP_50M  0x3u
#define GPIO_MODE_IN_FLOAT    0x4u
#define GPIO_MODE_AF_PP_50M   0xBu

static uint8_t g_spi_ready = 0u;
static uint8_t g_spi_hw = 0u;
static uint32_t g_spi_sck = 0u;
static uint32_t g_spi_miso = 0u;
static uint32_t g_spi_mosi = 0u;
static uint32_t g_spi_div = 2u;

static void gpio_set_mode(uint32_t pin, uint32_t mode)
{
    uint32_t port = pin / 16u;
    uint32_t line = pin % 16u;
    uint32_t shift = (line & 7u) * 4u;
    volatile uint32_t *cr = gpio_reg(port, (line < 8u) ? 0x00u : 0x04u);

    RCC_APB2ENR |= (1u << (2u + port));   /* IOPAEN / IOPBEN / IOPCEN */
    *cr = (*cr & ~(0xFu << shift)) | ((mode & 0xFu) << shift);
}

/* SPI_CR1.BR = log2(分频) - 1；只接受 2 的幂，非法返回 -1 */
static int spi_encode_br(uint32_t divider)
{
    uint32_t br = 0u;
    uint32_t d = 2u;

    while (d < divider) {
        d <<= 1;
        br++;
    }
    return (d == divider) ? (int)br : -1;
}

static void spi_bitbang_delay(void)
{
    volatile uint32_t i;
    uint32_t n = g_spi_div * 4u;
    for (i = 0u; i < n; i++) {
    }
}

/* 模式 0（CPOL=0/CPHA=0）、MSB 先行：上升沿采样 */
static uint32_t spi_bitbang_byte(uint32_t byte)
{
    uint32_t i;
    uint32_t got = 0u;

    for (i = 0u; i < 8u; i++) {
        bb_write(g_spi_mosi, (byte & 0x80u) ? 1u : 0u);
        spi_bitbang_delay();
        bb_write(g_spi_sck, 1u);
        spi_bitbang_delay();
        got = (got << 1) | bb_read(g_spi_miso);
        bb_write(g_spi_sck, 0u);
        spi_bitbang_delay();
        byte <<= 1;
    }
    return got;
}

int port_spi_init(uint32_t sck_pin, uint32_t miso_pin, uint32_t mosi_pin,
                  uint32_t divider)
{
    int br;

    if (sck_pin > 47u || miso_pin > 47u || mosi_pin > 47u) {
        return -1;
    }
    if (sck_pin == miso_pin || sck_pin == mosi_pin || miso_pin == mosi_pin) {
        return -1;
    }
    br = spi_encode_br(divider);
    if (br < 0) {
        return -1;
    }

    g_spi_sck = sck_pin;
    g_spi_miso = miso_pin;
    g_spi_mosi = mosi_pin;
    g_spi_div = divider;
    g_spi_hw = 0u;

    if (sck_pin == SPI_PIN_HW_SCK && miso_pin == SPI_PIN_HW_MISO &&
        mosi_pin == SPI_PIN_HW_MOSI) {
        RCC_APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_SPI1EN;
        gpio_set_mode(SPI_PIN_HW_SCK, GPIO_MODE_AF_PP_50M);
        gpio_set_mode(SPI_PIN_HW_MISO, GPIO_MODE_IN_FLOAT);
        gpio_set_mode(SPI_PIN_HW_MOSI, GPIO_MODE_AF_PP_50M);

        SPI1_CR1 = 0u;
        /* 主机 + NSS 软件（SSM/SSI）+ 模式 0 + MSB 先行 */
        SPI1_CR1 = SPI_CR1_MSTR | SPI_CR1_SSM | SPI_CR1_SSI |
                   ((uint32_t)br << 3) | SPI_CR1_SPE;
        g_spi_hw = 1u;
    } else {
        gpio_set_mode(g_spi_sck, GPIO_MODE_OUT_PP_50M);
        gpio_set_mode(g_spi_mosi, GPIO_MODE_OUT_PP_50M);
        gpio_set_mode(g_spi_miso, GPIO_MODE_IN_FLOAT);
        bb_write(g_spi_sck, 0u);
        bb_write(g_spi_mosi, 0u);
    }

    g_spi_ready = 1u;
    return (int)g_spi_hw;                /* 1 = 硬件 SPI1，0 = 软件位翻转 */
}

int port_spi_transfer(uint32_t byte, uint32_t *got)
{
    uint32_t timeout;
    uint32_t value;

    if (g_spi_ready == 0u) {
        return -1;
    }
    if (g_spi_hw == 0u) {
        value = spi_bitbang_byte(byte & 0xFFu);
        if (got != 0) {
            *got = value;
        }
        return 0;
    }

    timeout = 200000u;
    while ((SPI1_SR & SPI_SR_TXE) == 0u) {
        if (--timeout == 0u) {
            return -1;
        }
    }
    SPI1_DR = byte & 0xFFu;
    timeout = 200000u;
    while ((SPI1_SR & SPI_SR_RXNE) == 0u) {
        if (--timeout == 0u) {
            return -1;
        }
    }
    value = SPI1_DR & 0xFFu;
    if (got != 0) {
        *got = value;
    }
    return 0;
}

int port_spi_write(const uint32_t *data, uint32_t len)
{
    uint32_t i;
    uint32_t got = 0u;

    if (g_spi_ready == 0u || data == 0 || len == 0u) {
        return -1;
    }
    for (i = 0u; i < len; i++) {
        if (port_spi_transfer(data[i], &got) != 0) {
            return -1;
        }
    }
    return 0;
}

/* ---------------- EXTI（PA0–PA4 → EXTI0–4） ---------------- */

void port_exti_set_hook(void (*hook)(uint32_t line))
{
    g_exti_hook = hook;
}

static uint32_t exti_irq_of_line(uint32_t line)
{
    if (line <= 4u) {
        return EXTI_IRQ_FIRST + line;
    }
    if (line <= 9u) {
        return EXTI_IRQ_5_9;
    }
    return EXTI_IRQ_10_15;
}

/* 引脚配成输入 + 下拉（CNF=10, MODE=00 → 0x8），悬空时不会被噪声误触发 */
static void exti_gpio_pulldown(uint32_t pin)
{
    uint32_t port = pin / EXTI_PINS_PER_PORT;
    uint32_t line = pin % EXTI_PINS_PER_PORT;
    uint32_t shift = (line & 7u) * 4u;

    if (port == 0u) {
        RCC_APB2ENR |= RCC_APB2ENR_IOPAEN;
        if (line < 8u) {
            GPIOA_CRL = (GPIOA_CRL & ~(0xFu << shift)) | (0x8u << shift);
        } else {
            GPIOA_CRH = (GPIOA_CRH & ~(0xFu << shift)) | (0x8u << shift);
        }
        GPIOA_ODR &= ~(1u << line);
    } else if (port == 1u) {
        RCC_APB2ENR |= RCC_APB2ENR_IOPBEN;
        if (line < 8u) {
            GPIOB_CRL = (GPIOB_CRL & ~(0xFu << shift)) | (0x8u << shift);
        } else {
            GPIOB_CRH = (GPIOB_CRH & ~(0xFu << shift)) | (0x8u << shift);
        }
        GPIOB_ODR &= ~(1u << line);
    } else {
        RCC_APB2ENR |= RCC_APB2ENR_IOPCEN;
        if (line < 8u) {
            GPIOC_CRL = (GPIOC_CRL & ~(0xFu << shift)) | (0x8u << shift);
        } else {
            GPIOC_CRH = (GPIOC_CRH & ~(0xFu << shift)) | (0x8u << shift);
        }
        GPIOC_ODR &= ~(1u << line);
    }
}

/* AFIO 负责把 EXTI 线接到端口：每线 4 位，0=PA 1=PB 2=PC */
static void exti_afio_select(uint32_t port, uint32_t line)
{
    uint32_t shift = (line & 3u) * 4u;
    uint32_t field = port;

    if (line < 4u) {
        AFIO_EXTICR1 = (AFIO_EXTICR1 & ~(0xFu << shift)) | (field << shift);
    } else if (line < 8u) {
        AFIO_EXTICR2 = (AFIO_EXTICR2 & ~(0xFu << shift)) | (field << shift);
    } else if (line < 12u) {
        AFIO_EXTICR3 = (AFIO_EXTICR3 & ~(0xFu << shift)) | (field << shift);
    } else {
        AFIO_EXTICR4 = (AFIO_EXTICR4 & ~(0xFu << shift)) | (field << shift);
    }
}

void port_exti_bind(uint32_t pin, uint32_t edge)
{
    uint32_t line;
    uint32_t bit;
    uint32_t irq;
    uint8_t prev;

    if (pin >= EXTI_LINES) {
        return;
    }
    line = pin % EXTI_PINS_PER_PORT;

    /* 一条 EXTI 线同一时刻只能选一个端口：换引脚前先解绑同线的旧引脚 */
    prev = g_exti_line_pin[line];
    if (prev != 0u && ((uint32_t)prev - 1u) != pin) {
        port_exti_unbind((uint32_t)prev - 1u);
    }

    RCC_APB2ENR |= RCC_APB2ENR_AFIOEN;
    exti_gpio_pulldown(pin);
    exti_afio_select(pin / EXTI_PINS_PER_PORT, line);

    bit = 1u << line;
    if (edge == 0u || edge == 2u) {
        EXTI_RTSR |= bit;
    } else {
        EXTI_RTSR &= ~bit;
    }
    if (edge == 1u || edge == 2u) {
        EXTI_FTSR |= bit;
    } else {
        EXTI_FTSR &= ~bit;
    }

    EXTI_PR = bit;          /* 清掉绑定前可能残留的挂起位 */
    EXTI_SWIER &= ~bit;
    EXTI_IMR |= bit;
    EXTI_EMR &= ~bit;

    irq = exti_irq_of_line(line);
    (void)port_irq_set_priority(irq, EXTI_IRQ_PRIO);
#if JM_IRQ_DEBUG
    port_log("[中断] EXTI 线 ");
    port_log_u32(line);
    port_log(" IRQ=");
    port_log_u32(irq);
    port_log(" 优先级=");
    port_log_hex(port_irq_get_priority(irq));
    port_log("\r\n");
#endif
    if (irq < 32u) {
        NVIC_ISER0 = (1u << irq);
    } else {
        NVIC_ISER1 = (1u << (irq - 32u));
    }

    g_exti_count[pin] = 0u;
    g_exti_bound[pin] = 1u;
    g_exti_line_pin[line] = (uint8_t)(pin + 1u);
}

void port_exti_unbind(uint32_t pin)
{
    uint32_t line;
    uint32_t bit;
    uint32_t irq;
    uint32_t first;
    uint32_t last;
    uint32_t i;
    uint8_t any = 0u;

    if (pin >= EXTI_LINES || g_exti_bound[pin] == 0u) {
        return;
    }
    line = pin % EXTI_PINS_PER_PORT;
    bit = 1u << line;

    EXTI_IMR &= ~bit;
    EXTI_RTSR &= ~bit;
    EXTI_FTSR &= ~bit;
    EXTI_SWIER &= ~bit;
    EXTI_PR = bit;
    g_exti_bound[pin] = 0u;
    if (g_exti_line_pin[line] == (uint8_t)(pin + 1u)) {
        g_exti_line_pin[line] = 0u;
    }

    /* 同一组向量里还有别的线在用，就不要关 NVIC */
    first = (line <= 4u) ? 0u : ((line <= 9u) ? 5u : 10u);
    last = (line <= 4u) ? 4u : ((line <= 9u) ? 9u : 15u);
    for (i = first; i <= last; i++) {
        if (g_exti_line_pin[i] != 0u) {
            any = 1u;
            break;
        }
    }
    if (any == 0u) {
        irq = exti_irq_of_line(line);
        if (irq < 32u) {
            NVIC_ICER0 = (1u << irq);
        } else {
            NVIC_ICER1 = (1u << (irq - 32u));
        }
    }
}

void port_exti_sw_trigger(uint32_t pin)
{
    uint32_t line;

    if (pin >= EXTI_LINES || g_exti_bound[pin] == 0u) {
        return;
    }
    line = pin % EXTI_PINS_PER_PORT;
    EXTI_SWIER |= (1u << line);   /* 置挂起位，下一指令边界进中断 */
}

uint32_t port_exti_count(uint32_t pin)
{
    if (pin >= EXTI_LINES) {
        return 0u;
    }
    return g_exti_count[pin];
}

#if JM_IRQ_NEST_DEMO
/* 跨层抢占自检：只在上板自检构建里启用。
 * trace 值：低 7 位是线号+1，最高位 0=进入 / 1=退出。 */
static volatile uint8_t g_irq_depth;
static volatile uint8_t g_irq_depth_max;
static volatile uint8_t g_irq_trace[16];
static volatile uint8_t g_irq_trace_len;
static volatile uint8_t g_irq_nest_demo_active;
#endif

/* 三组中断共用同一套处理：只处理本组的挂起位，避免误清别组事件。
 * 这里只允许「计数 + 回调」，回调由板级层实现，只做 jmk_queue_send_from_isr；
 * 任何情况下都不进 VM、不跑砾字节码。 */
static void exti_serve(uint32_t first, uint32_t last)
{
    uint32_t pr = EXTI_PR;
    uint32_t line;

    for (line = first; line <= last; line++) {
        uint32_t bit = 1u << line;
        if ((pr & bit) != 0u) {
            uint8_t v = g_exti_line_pin[line];
            EXTI_PR = bit;      /* 写 1 清挂起，同时清掉 SWIER 对应位 */
            if (v != 0u) {
#if JM_IRQ_NEST_DEMO
                if (g_irq_depth < 0xFFu) {
                    g_irq_depth++;
                }
                if (g_irq_depth > g_irq_depth_max) {
                    g_irq_depth_max = g_irq_depth;
                }
                if (g_irq_trace_len < (uint8_t)sizeof(g_irq_trace)) {
                    g_irq_trace[g_irq_trace_len++] = (uint8_t)(line + 1u);
                }
                if (line == 0u && g_irq_nest_demo_active) {
                    /* PA1/IRQ7 已设为 0x10：应在这里立即抢占当前 0x40 的 ISR。
                     * 自检结束后 active=0，不影响后续砾程序的中断语义。 */
                    port_exti_sw_trigger(1u);
                }
#endif
                g_exti_count[(uint32_t)v - 1u]++;
                if (g_exti_hook) {
                    g_exti_hook(line);
                }
#if JM_IRQ_NEST_DEMO
                if (g_irq_trace_len < (uint8_t)sizeof(g_irq_trace)) {
                    g_irq_trace[g_irq_trace_len++] = (uint8_t)(0x80u | (line + 1u));
                }
                if (g_irq_depth > 0u) {
                    g_irq_depth--;
                }
#endif
            }
        }
    }
}

void EXTI_IRQHandler(void)
{
    exti_serve(0u, 4u);
}

void EXTI9_5_IRQHandler(void)
{
    exti_serve(5u, 9u);
}

void EXTI15_10_IRQHandler(void)
{
    exti_serve(10u, 15u);
}

#if JM_IRQ_NEST_DEMO
void port_irq_nest_demo_run(void)
{
    uint8_t i;
    uint8_t ok;

    g_irq_depth = 0u;
    g_irq_depth_max = 0u;
    g_irq_trace_len = 0u;
    g_irq_nest_demo_active = 0u;

    port_exti_bind(0u, 0u);   /* PA0 -> IRQ6，普通层 0x40 */
    port_exti_bind(1u, 0u);   /* PA1 -> IRQ7，先按普通层绑定 */
    if (port_irq_set_priority(EXTI_IRQ_FIRST + 1u, PORT_PRIO_CRITICAL) != 0) {
        port_log("[嵌套] 设置 PA1 为紧急层失败\r\n");
        port_exti_unbind(0u);
        port_exti_unbind(1u);
        return;
    }

    g_irq_nest_demo_active = 1u;
#if JM_MSP_WATERMARK
    {
        uint32_t r;
        for (r = 0u; r < 10000u; r++) {
            port_exti_sw_trigger(0u);   /* 10 000 次嵌套压力，供 MSP 水位扫描 */
        }
    }
#else
    port_exti_sw_trigger(0u);   /* PA0 处理中会触发 PA1，应被抢占 */
#endif
    g_irq_nest_demo_active = 0u;

    port_log("[嵌套] 深度峰值=");
    port_log_u32(g_irq_depth_max);
    port_log(" 顺序=");
    for (i = 0u; i < g_irq_trace_len && i < 4u; i++) {
        if (i != 0u) {
            port_log(",");
        }
        port_log_u32(g_irq_trace[i]);
    }
    if (g_irq_trace_len > 4u) {
        port_log(",...");
    }
    port_log("\r\n");

    ok = (g_irq_depth_max == 2u && g_irq_trace_len >= 4u &&
          g_irq_trace[0] == 1u && g_irq_trace[1] == 2u &&
          g_irq_trace[2] == (uint8_t)(0x80u | 2u) &&
          g_irq_trace[3] == (uint8_t)(0x80u | 1u)) ? 1u : 0u;
    port_log(ok ? "[嵌套] 通过：PA1 抢占 PA0\r\n" : "[嵌套] 不通过\r\n");

    port_exti_unbind(0u);
    port_exti_unbind(1u);
}
#endif

#if JM_FAULT_DEMO
void port_fault_demo_trigger(void)
{
    port_log("[故障] 自检：执行 UDF 触发 HardFault\r\n");
    __asm volatile ("udf #0");
}
#endif

/* ---------------- 运行时端口（jm_port.h） ---------------- */

void jm_port_write(const char *s, uint32_t n)
{
    uint32_t i;
    for (i = 0u; i < n; i++) {
        uart_putc(s[i]);
    }
}

uint32_t jm_port_millis(void)
{
    return g_ms;
}

void jm_port_sleep_ms(uint32_t ms)
{
    if (ms == 0u) {
        return;
    }
    jmk_delay(ms);
}

void jm_port_yield(void)
{
    jmk_yield();
}

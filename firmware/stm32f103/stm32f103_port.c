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
#define RCC_APB2ENR_IOPCEN  (1u << 4)
#define RCC_APB2ENR_ADC1EN  (1u << 9)
#define RCC_APB2ENR_USART1EN (1u << 14)
#define RCC_APB2ENR_AFIOEN  (1u << 0)

#define RCC_APB1ENR_TIM2EN  (1u << 0)

/* ADC 预分频：时钟 / 6（72 MHz → 12 MHz，上限 14 MHz） */
#define RCC_CFGR_ADCPRE_MASK  (3u << 14)
#define RCC_CFGR_ADCPRE_DIV6  (2u << 14)

/* FLASH */
#define FLASH_ACR       REG32(0x40022000u)
#define FLASH_ACR_LATENCY2 (2u << 0)
#define FLASH_ACR_PRFTBE   (1u << 4)

/* GPIOA / GPIOC */
#define GPIOA_CRL       REG32(0x40010800u)
#define GPIOA_CRH       REG32(0x40010804u)
#define GPIOA_IDR       REG32(0x40010808u)
#define GPIOA_ODR       REG32(0x4001080Cu)
#define GPIOC_CRH       REG32(0x40011004u)
#define GPIOC_IDR       REG32(0x40011008u)
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

/* EXTI（PA0–PA4 → EXTI0–4，IRQ 6–10） */
#define AFIO_EXTICR1    REG32(0x40010008u)
#define EXTI_IMR        REG32(0x40010400u)
#define EXTI_EMR        REG32(0x40010404u)
#define EXTI_RTSR       REG32(0x40010408u)
#define EXTI_FTSR       REG32(0x4001040Cu)
#define EXTI_SWIER      REG32(0x40010410u)
#define EXTI_PR         REG32(0x40010414u)

#define NVIC_ISER0      REG32(0xE000E100u)
#define NVIC_ICER0      REG32(0xE000E100u + 0x80u)
#define NVIC_IPR_BYTE(irq) (*(volatile uint8_t *)(0xE000E400u + (irq)))

#define EXTI_LINES      5u
#define EXTI_IRQ_FIRST  6u      /* EXTI0 的 IRQ 号；EXTI1–4 依次 +1 */
#define EXTI_IRQ_PRIO   0x40u   /* 低于 SysTick（默认 0），不比内核 tick 更急 */

#define UART_BAUD       115200u

/* ---------------- 状态 ---------------- */

static uint32_t g_sysclk = 8000000u;
static volatile uint32_t g_ms = 0;
static volatile uint8_t g_adc_ready = 0u;
static volatile uint8_t g_timer_ready = 0u;

/* EXTI：中断里只写计数并回调 hook，不进入 VM */
static volatile uint32_t g_exti_count[EXTI_LINES];
static void (*g_exti_hook)(uint32_t line) = 0;
static uint8_t g_exti_bound[EXTI_LINES];

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
    } else {
        /* HSE 缺席（无外部晶振）：退回内部 HSI 8 MHz */
        g_sysclk = 8000000u;
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
    uint32_t clk = clock_init();
    uart_init(clk, UART_BAUD);
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

/* ---------------- 板级 GPIO（v0：PC13 一个脚） ---------------- */

void port_gpio_write(uint32_t pin, int level)
{
    if (pin != LED_PIN) {
        return;
    }
    if (level) {
        GPIOC_BSRR = (1u << LED_PIN);   /* 高电平 */
    } else {
        GPIOC_BRR = (1u << LED_PIN);    /* 低电平（板载 LED 点亮） */
    }
}

int port_gpio_read(uint32_t pin)
{
    if (pin != LED_PIN) {
        return -1;
    }
    return (int)((GPIOC_IDR >> LED_PIN) & 1u);
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

/* ---------------- TIM2（1 kHz 毫秒计数） ---------------- */

void port_timer_init(void)
{
    uint32_t psc = g_sysclk / 1000u;

    if (psc == 0u) {
        psc = 1u;
    }
    psc -= 1u;   /* 计数频率 = TIM2CLK / (PSC + 1) = 1 kHz */

    RCC_APB1ENR |= RCC_APB1ENR_TIM2EN;
    TIM2_CR1 = 0u;
    TIM2_PSC = psc;
    TIM2_ARR = 0xFFFFu;   /* 16 位自由运行，约 65.5 秒回绕 */
    TIM2_EGR = 1u;        /* UG：立即把 PSC/ARR 装进影子寄存器 */
    TIM2_CNT = 0u;
    TIM2_CR1 = 1u;        /* CEN = 1 */

    g_timer_ready = 1u;
}

int port_timer_delay_ms(uint32_t ms)
{
    uint16_t last;
    uint32_t elapsed = 0u;

    if (g_timer_ready == 0u || ms > 60000u) {
        return -1;
    }

    last = (uint16_t)(TIM2_CNT & 0xFFFFu);
    while (elapsed < ms) {
        uint16_t now = (uint16_t)(TIM2_CNT & 0xFFFFu);
        elapsed += (uint16_t)(now - last);   /* 16 位差值，天然处理回绕 */
        last = now;
    }
    return 0;
}

int port_timer_count(uint32_t *ms)
{
    if (g_timer_ready == 0u || ms == 0) {
        return -1;
    }
    *ms = TIM2_CNT & 0xFFFFu;
    return 0;
}

/* ---------------- EXTI（PA0–PA4 → EXTI0–4） ---------------- */

void port_exti_set_hook(void (*hook)(uint32_t line))
{
    g_exti_hook = hook;
}

void port_exti_bind(uint32_t pin, uint32_t edge)
{
    uint32_t shift;
    uint32_t bit;
    uint32_t irq;

    if (pin >= EXTI_LINES) {
        return;
    }
    bit = 1u << pin;
    irq = EXTI_IRQ_FIRST + pin;

    /* AFIO 负责把 EXTI 线接到端口；EXTICR 复位值即 PA，这里显式写 0 更直观 */
    RCC_APB2ENR |= RCC_APB2ENR_AFIOEN | RCC_APB2ENR_IOPAEN;

    /* PAx = 输入 + 下拉（CNF=10, MODE=00 → 0x8），悬空时不会被噪声误触发 */
    shift = (pin & 7u) * 4u;
    GPIOA_CRL = (GPIOA_CRL & ~(0xFu << shift)) | (0x8u << shift);
    GPIOA_ODR &= ~bit;
    if (pin < 4u) {
        AFIO_EXTICR1 &= ~(0xFu << (shift));
    }

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

    NVIC_IPR_BYTE(irq) = EXTI_IRQ_PRIO;
    NVIC_ISER0 = (1u << irq);

    g_exti_count[pin] = 0u;
    g_exti_bound[pin] = 1u;
}

void port_exti_unbind(uint32_t pin)
{
    uint32_t bit;
    uint32_t irq;

    if (pin >= EXTI_LINES) {
        return;
    }
    bit = 1u << pin;
    irq = EXTI_IRQ_FIRST + pin;

    EXTI_IMR &= ~bit;
    EXTI_RTSR &= ~bit;
    EXTI_FTSR &= ~bit;
    EXTI_SWIER &= ~bit;
    EXTI_PR = bit;
    NVIC_ICER0 = (1u << irq);
    g_exti_bound[pin] = 0u;
}

void port_exti_sw_trigger(uint32_t pin)
{
    if (pin >= EXTI_LINES || g_exti_bound[pin] == 0u) {
        return;
    }
    EXTI_SWIER |= (1u << pin);   /* 置挂起位，下一指令边界进中断 */
}

uint32_t port_exti_count(uint32_t pin)
{
    if (pin >= EXTI_LINES) {
        return 0u;
    }
    return g_exti_count[pin];
}

/* EXTI0–4 共用入口：向量表 5 条线都指到这里，按挂起位逐条处理。
 * 这里只允许「计数 + 回调」，回调由板级层实现，只做 jmk_queue_send_from_isr；
 * 任何情况下都不进 VM、不跑砾字节码。 */
void EXTI_IRQHandler(void)
{
    uint32_t pr = EXTI_PR;
    uint32_t i;

    for (i = 0u; i < EXTI_LINES; i++) {
        if ((pr & (1u << i)) != 0u) {
            EXTI_PR = (1u << i);    /* 写 1 清挂起，同时清掉 SWIER 对应位 */
            g_exti_count[i]++;
            if (g_exti_hook) {
                g_exti_hook(i);
            }
        }
    }
}

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

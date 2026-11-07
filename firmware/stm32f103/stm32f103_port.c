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

#define UART_BAUD       115200u

/* ---------------- 状态 ---------------- */

static uint32_t g_sysclk = 8000000u;
static volatile uint32_t g_ms = 0;
static volatile uint8_t g_adc_ready = 0u;
static volatile uint8_t g_timer_ready = 0u;

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

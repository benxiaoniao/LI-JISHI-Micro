/*
 * 砾（LI）STM32F103C8T6 启动文件（Cortex-M3）
 * 职责：建立向量表、复制 .data、清零 .bss、跳转 main。
 */

    .syntax unified
    .cpu cortex-m3
    .thumb

    .global g_pfnVectors
    .global Reset_Handler

    .word _sidata
    .word _sdata
    .word _edata
    .word _sbss
    .word _ebss

    .section .isr_vector, "a", %progbits
    .type g_pfnVectors, %object
g_pfnVectors:
    .word _estack                 /* 0x00 初始主栈指针 */
    .word Reset_Handler           /* 0x04 复位 */
    .word Default_Handler         /* 0x08 NMI */
    .word HardFault_Handler       /* 0x0C HardFault */
    .word Default_Handler         /* 0x10 MemManage */
    .word Default_Handler         /* 0x14 BusFault */
    .word Default_Handler         /* 0x18 UsageFault */
    .word 0                       /* 0x1C 保留 */
    .word 0                       /* 0x20 保留 */
    .word 0                       /* 0x24 保留 */
    .word 0                       /* 0x28 保留 */
    .word SVC_Handler             /* 0x2C SVC */
    .word Default_Handler         /* 0x30 DebugMon */
    .word 0                       /* 0x34 保留 */
    .word PendSV_Handler          /* 0x38 PendSV */
    .word SysTick_Handler         /* 0x3C SysTick */
    /* 外部中断 IRQ0–4（WWDG/PVD/TAMPER/RTC/FLASH/RCC）未使用，仍指向默认死循环 */
    .word Default_Handler         /* 0x40 IRQ0  WWDG */
    .word Default_Handler         /* 0x44 IRQ1  PVD */
    .word Default_Handler         /* 0x48 IRQ2  TAMPER */
    .word Default_Handler         /* 0x4C IRQ3  RTC */
    .word Default_Handler         /* 0x50 IRQ4  FLASH */
    .word Default_Handler         /* 0x54 IRQ5  RCC */
    /* EXTI0–4 共用一个入口，按 EXTI_PR 分派 */
    .word EXTI_IRQHandler         /* 0x58 IRQ6  EXTI0（PA0） */
    .word EXTI_IRQHandler         /* 0x5C IRQ7  EXTI1（PA1） */
    .word EXTI_IRQHandler         /* 0x60 IRQ8  EXTI2（PA2） */
    .word EXTI_IRQHandler         /* 0x64 IRQ9  EXTI3（PA3） */
    .word EXTI_IRQHandler         /* 0x68 IRQ10 EXTI4（PA4） */
    /* 其余 IRQ 未使用，补齐到 IRQ42，避免开启中断后取到向量表外 */
    .word Default_Handler         /* 0x6C IRQ11 DMA1_Channel1 */
    .word Default_Handler         /* 0x70 IRQ12 DMA1_Channel2 */
    .word Default_Handler         /* 0x74 IRQ13 DMA1_Channel3 */
    .word Default_Handler         /* 0x78 IRQ14 DMA1_Channel4 */
    .word Default_Handler         /* 0x7C IRQ15 DMA1_Channel5 */
    .word Default_Handler         /* 0x80 IRQ16 DMA1_Channel6 */
    .word Default_Handler         /* 0x84 IRQ17 DMA1_Channel7 */
    .word Default_Handler         /* 0x88 IRQ18 ADC1_2 */
    .word Default_Handler         /* 0x8C IRQ19 USB_HP_CAN1_TX */
    .word Default_Handler         /* 0x90 IRQ20 USB_LP_CAN1_RX0 */
    .word Default_Handler         /* 0x94 IRQ21 CAN1_RX1 */
    .word Default_Handler         /* 0x98 IRQ22 CAN1_SCE */
    /* 引脚 5–9（PA/PB/PC 的线 5–9）共用 EXTI9_5 */
    .word EXTI9_5_IRQHandler      /* 0x9C IRQ23 EXTI9_5 */
    .word Default_Handler         /* 0xA0 IRQ24 TIM1_BRK */
    .word Default_Handler         /* 0xA4 IRQ25 TIM1_UP */
    .word Default_Handler         /* 0xA8 IRQ26 TIM1_TRG_COM */
    .word Default_Handler         /* 0xAC IRQ27 TIM1_CC */
    .word Default_Handler         /* 0xB0 IRQ28 TIM2 */
    .word Default_Handler         /* 0xB4 IRQ29 TIM3 */
    .word Default_Handler         /* 0xB8 IRQ30 TIM4 */
    .word Default_Handler         /* 0xBC IRQ31 I2C1_EV */
    .word Default_Handler         /* 0xC0 IRQ32 I2C1_ER */
    .word Default_Handler         /* 0xC4 IRQ33 I2C2_EV */
    .word Default_Handler         /* 0xC8 IRQ34 I2C2_ER */
    .word Default_Handler         /* 0xCC IRQ35 SPI1 */
    .word Default_Handler         /* 0xD0 IRQ36 SPI2 */
    .word Default_Handler         /* 0xD4 IRQ37 USART1 */
    .word Default_Handler         /* 0xD8 IRQ38 USART2 */
    .word Default_Handler         /* 0xDC IRQ39 USART3 */
    /* 引脚 10–15（PA/PB/PC 的线 10–15）共用 EXTI15_10 */
    .word EXTI15_10_IRQHandler    /* 0xE0 IRQ40 EXTI15_10 */
    .word Default_Handler         /* 0xE4 IRQ41 RTCAlarm */
    .word Default_Handler         /* 0xE8 IRQ42 USBWakeUp */
    .size g_pfnVectors, .-g_pfnVectors

    .text
    .thumb_func
    .type Reset_Handler, %function
Reset_Handler:
    ldr r0, =_estack
    mov sp, r0

    /* 复制 .data：源在 Flash，目标在 RAM */
    ldr r0, =_sidata
    ldr r1, =_sdata
    ldr r2, =_edata
copy_data:
    cmp r1, r2
    bhs clear_bss
    ldr r3, [r0], #4
    str r3, [r1], #4
    b copy_data

    /* 清零 .bss */
clear_bss:
    ldr r0, =_sbss
    ldr r1, =_ebss
    movs r2, #0
zero_bss:
    cmp r0, r1
    bhs call_main
    str r2, [r0], #4
    b zero_bss

call_main:
    bl main
halt:
    b halt
    .size Reset_Handler, .-Reset_Handler

    .thumb_func
    .type Default_Handler, %function
Default_Handler:
    b .
    .size Default_Handler, .-Default_Handler

    /*
     * HardFault：把 EXC_RETURN / MSP / PSP 交给 C 侧 hardfault_report 打印现场，
     * 打印完在 C 里死循环，不返回。
     */
    .global HardFault_Handler
    .thumb_func
    .type HardFault_Handler, %function
HardFault_Handler:
    mov r0, lr          /* EXC_RETURN */
    mrs r1, msp
    mrs r2, psp
    bl hardfault_report
    b .
    .size HardFault_Handler, .-HardFault_Handler

    .end

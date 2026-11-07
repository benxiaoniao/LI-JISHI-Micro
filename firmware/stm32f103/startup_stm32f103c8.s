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
    .word Default_Handler         /* 0x0C HardFault */
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

    .end

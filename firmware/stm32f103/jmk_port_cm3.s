/*
 * 砾微内核 Cortex-M3 切换代码
 *   - jmk_first_switch：启动第一个任务（设置 PSP/CONTROL 后走 SVC）
 *   - SVC_Handler：首次进入任务时弹出 R4-R11 与异常帧
 *   - PendSV_Handler：任务间上下文切换（保存 R4-R11 到 PSP 任务栈）
 */

    .syntax unified
    .cpu cortex-m3
    .thumb
    .text

    .global jmk_first_switch
    .thumb_func
    .type jmk_first_switch, %function
jmk_first_switch:
    ldr r0, =jmk_current
    ldr r0, [r0]
    ldr r1, [r0]            /* r1 = tcb->sp（第一个字段） */
    msr psp, r1
    isb
    movs r1, #2
    msr control, r1         /* 线程模式改用 PSP */
    isb
    cpsie i                 /* 必须先开中断：SVC 被 PRIMASK 屏蔽会升级为 HardFault */
    svc 0                   /* 进入 SVC_Handler，完成后直接跑第一个任务 */
    b .
    .size jmk_first_switch, .-jmk_first_switch

    .global SVC_Handler
    .thumb_func
    .type SVC_Handler, %function
SVC_Handler:
    ldr r3, =jmk_current
    ldr r1, [r3]
    ldr r0, [r1]            /* r0 = tcb->sp，指向 R4 */
    ldmia r0!, {r4-r11}     /* 弹出软件上下文 */
    msr psp, r0             /* PSP 指向 R0 槽（硬件异常帧） */
    isb
    movs r0, #0
    msr basepri, r0
    cpsie i                 /* 允许中断；待处理的 SysTick 会在这里先跑 */
    orr lr, lr, #0xD        /* EXC_RETURN：线程模式 + PSP */
    bx lr
    .size SVC_Handler, .-SVC_Handler

    .global PendSV_Handler
    .thumb_func
    .type PendSV_Handler, %function
PendSV_Handler:
    mrs r0, psp
    isb
    stmdb r0!, {r4-r11}     /* 保存软件上下文到当前任务栈 */
    ldr r2, =jmk_current
    ldr r3, [r2]
    str r0, [r3]            /* tcb->sp = r0 */
    push {r4, lr}
    bl jmk_schedule         /* C 侧选下一个任务并更新 jmk_current */
    pop {r4, lr}
    ldr r2, =jmk_current
    ldr r3, [r2]
    ldr r0, [r3]
    ldmia r0!, {r4-r11}     /* 恢复下一个任务的软件上下文 */
    msr psp, r0
    isb
    bx lr                   /* 异常返回，硬件弹 R0-R3/R12/LR/PC/xPSR */
    .size PendSV_Handler, .-PendSV_Handler

    .end

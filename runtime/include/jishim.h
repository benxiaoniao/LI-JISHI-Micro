#ifndef JISHIM_H
#define JISHIM_H

#include <stddef.h>
#include <stdint.h>

/* ---- 可配置的资源上限（改宏即可适配不同芯片） ---- */
#ifndef JM_HEAP_SIZE
#define JM_HEAP_SIZE (12 * 1024)
#endif
#ifndef JM_STACK_SLOTS
#define JM_STACK_SLOTS 512
#endif
#ifndef JM_MAX_FRAMES
#define JM_MAX_FRAMES 32
#endif
#ifndef JM_MAX_GLOBALS
#define JM_MAX_GLOBALS 256
#endif

/*
 * 运行一段微字节码（.jmb）。
 * 成功返回 0；失败返回 -1，具体错误用 jm_last_error() 取。
 * 可以反复调用：每次调用都会重置堆与执行状态。
 */
int jm_run(const void *jmb, size_t len);

/* 最近一次运行的错误说明（静态缓冲区，短文案）。 */
const char *jm_last_error(void);

#endif

#include <stdio.h>
#include "jm_port.h"

#ifdef _WIN32
#include <windows.h>
#else
#include <sched.h>
#include <time.h>
#endif

void jm_port_write(const char *s, uint32_t n) {
    for (uint32_t i = 0; i < n; i++) {
        fputc((int)s[i], stdout);
    }
}

/* 单调毫秒计数。此前恒返回 0，导致主机侧任何「等到某个 tick」的逻辑都不会到期。 */
uint32_t jm_port_millis(void) {
#ifdef _WIN32
    return (uint32_t)GetTickCount();
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint32_t)((uint64_t)ts.tv_sec * 1000u + (uint64_t)(ts.tv_nsec / 1000000));
#endif
}

void jm_port_sleep_ms(uint32_t ms) {
    if (ms == 0u) {
        return;
    }
#ifdef _WIN32
    Sleep((DWORD)ms);
#else
    struct timespec req;
    req.tv_sec = (time_t)(ms / 1000u);
    req.tv_nsec = (long)((ms % 1000u) * 1000000u);
    nanosleep(&req, NULL);
#endif
}

void jm_port_yield(void) {
#ifdef _WIN32
    SwitchToThread();
#else
    sched_yield();
#endif
}

/*
 * 砾的 Arduino 端口：只实现运行时要的四个钩子（见 jm_port.h）。
 * 串口输出走 Arduino 的 Serial，时间与让出走 core 的 millis/delay/yield，
 * 这样不与 STM32duino 的时钟、串口初始化抢资源。
 */
#include <Arduino.h>

extern "C" {
#include "jm_port.h"
}

void jm_port_write(const char *s, uint32_t n) {
    Serial.write(reinterpret_cast<const uint8_t *>(s), n);
}

uint32_t jm_port_millis(void) {
    return (uint32_t)millis();
}

void jm_port_sleep_ms(uint32_t ms) {
    delay(ms);
}

void jm_port_yield(void) {
    yield();
}

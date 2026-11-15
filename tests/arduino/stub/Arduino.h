/*
 * 主机验收用的 Arduino 兼容桩。
 *
 * 只声明砾的 Arduino 端口用到的部分（Serial / millis / delay / yield），
 * 让 `integrations/arduino/src` 里的代码能在 PC 上原样编译、链接、运行。
 * 真机上的 Arduino.h 由 STM32duino core 提供。
 */
#ifndef LI_ARDUINO_STUB_H
#define LI_ARDUINO_STUB_H

#include <stddef.h>
#include <stdint.h>

class LiSerialStub {
public:
    void begin(unsigned long baud);
    size_t write(const uint8_t *buf, size_t n);
    size_t write(uint8_t b);
    void print(const char *s);
    void println(const char *s);
};

extern LiSerialStub Serial;

unsigned long millis();
void delay(unsigned long ms);
void yield();

#endif

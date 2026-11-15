#include "Arduino.h"

#include <chrono>
#include <cstdio>
#include <thread>

LiSerialStub Serial;

void LiSerialStub::begin(unsigned long) {
}

size_t LiSerialStub::write(const uint8_t *buf, size_t n) {
    size_t w = fwrite(buf, 1, n, stdout);
    fflush(stdout);
    return w;
}

size_t LiSerialStub::write(uint8_t b) {
    return write(&b, 1);
}

void LiSerialStub::print(const char *s) {
    fputs(s, stdout);
    fflush(stdout);
}

void LiSerialStub::println(const char *s) {
    fputs(s, stdout);
    fputc('\n', stdout);
    fflush(stdout);
}

static std::chrono::steady_clock::time_point g_start = std::chrono::steady_clock::now();

unsigned long millis() {
    auto d = std::chrono::steady_clock::now() - g_start;
    return (unsigned long)std::chrono::duration_cast<std::chrono::milliseconds>(d).count();
}

void delay(unsigned long ms) {
    std::this_thread::sleep_for(std::chrono::milliseconds(ms));
}

void yield() {
}

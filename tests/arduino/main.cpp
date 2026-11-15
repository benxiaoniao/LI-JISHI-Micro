/*
 * 主机验收：把 Arduino 库当成普通 C/C++ 代码编一遍，跑内嵌的 .jmb，逐字节比对输出。
 * 另外验证 li::run(jmb, len) 对坏字节码返回 -1 且 lastError() 有文案。
 */
#include <cstdio>

#include "Arduino.h"
#include "LiJishim.h"

int main() {
    Serial.begin(115200);
    if (li::run() != 0) {
        Serial.print("砾运行失败：");
        Serial.println(li::lastError());
        return 1;
    }

    const unsigned char bad[4] = {0x4A, 0x4D, 0x42, 0x00};
    if (li::run(bad, sizeof bad) == 0 || li::lastError()[0] == '\0') {
        fprintf(stderr, "坏字节码没有报错\n");
        return 2;
    }
    return 0;
}

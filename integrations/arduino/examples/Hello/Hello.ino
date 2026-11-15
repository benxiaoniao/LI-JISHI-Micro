// 砾的最小 Arduino 用法：
//   1. 用 micc 把 .jsh 编成 .jmb，再用 --emit-blob 生成 src/jm_blob.c（tools/pack-arduino.ps1 会自动做）
//   2. setup() 里调 li::run()，loop() 保持空转，避免重复执行
#include <LiJishim.h>

void setup() {
    Serial.begin(115200);
    delay(200);  // 等串口就绪

    int rc = li::run();
    if (rc != 0) {
        Serial.print("砾运行失败：");
        Serial.println(li::lastError());
    }
}

void loop() {
}

/*
 * Arduino 库的资源上限。
 *
 * F103C8T6 只有 20 KB RAM，STM32duino core 自己要用掉一部分；
 * 默认 12 KB 堆在 BluePill 上会直接链接溢出（实测 overflowed by 1232 bytes）。
 * 6 KB 与裸机固件（build-firmware.ps1 的 -DJM_HEAP_SIZE=6144）保持一致。
 */
#ifndef LI_JISHIM_JM_CONFIG_H
#define LI_JISHIM_JM_CONFIG_H

#ifndef JM_HEAP_SIZE
#define JM_HEAP_SIZE (6 * 1024)
#endif

#endif

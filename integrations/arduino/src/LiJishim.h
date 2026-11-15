#ifndef LI_JISHIM_ARDUINO_H
#define LI_JISHIM_ARDUINO_H

#include <stddef.h>

namespace li {

/* 运行库内嵌的 .jmb（`jm_blob.c` 里的 `jm_app_jmb`）。
   返回 0 成功；-1 失败，用 lastError() 取原因。可反复调用：每次都会重置堆与执行状态。 */
int run();

/* 运行指定的字节码，语义同 jm_run()。 */
int run(const unsigned char *jmb, size_t len);

/* 最近一次错误的短文案；没有错误时返回空串。 */
const char *lastError();

} /* namespace li */

#endif

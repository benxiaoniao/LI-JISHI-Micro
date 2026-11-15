#include <Arduino.h>
#include <stdint.h>

#include "LiJishim.h"

extern "C" {
#include "jishim.h"

/* `micc --emit-blob` 的固定符号名。 */
extern const unsigned char jm_app_jmb[];
extern const uint32_t jm_app_jmb_len;
}

namespace li {

int run() {
    return jm_run(jm_app_jmb, (size_t)jm_app_jmb_len);
}

int run(const unsigned char *jmb, size_t len) {
    return jm_run(jmb, len);
}

const char *lastError() {
    return jm_last_error();
}

} /* namespace li */

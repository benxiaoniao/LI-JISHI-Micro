#include <stdio.h>
#include "jm_port.h"

void jm_port_write(const char *s, uint32_t n) {
    for (uint32_t i = 0; i < n; i++) {
        fputc((int)s[i], stdout);
    }
}

uint32_t jm_port_millis(void) {
    return 0;
}

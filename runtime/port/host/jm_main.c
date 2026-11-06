#include <stdio.h>
#include <stdlib.h>
#include "jishim.h"

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "用法：jmrun <文件.jmb>\n");
        return 2;
    }
    FILE *f = fopen(argv[1], "rb");
    if (!f) {
        fprintf(stderr, "打不开 %s\n", argv[1]);
        return 2;
    }
    long n = 0;
    if (fseek(f, 0, SEEK_END) != 0) {
        fclose(f);
        return 2;
    }
    n = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (n <= 0) {
        fclose(f);
        fprintf(stderr, "文件是空的\n");
        return 2;
    }
    unsigned char *buf = (unsigned char *)malloc((size_t)n);
    if (!buf) {
        fclose(f);
        return 2;
    }
    if (fread(buf, 1, (size_t)n, f) != (size_t)n) {
        fclose(f);
        free(buf);
        return 2;
    }
    fclose(f);

    int rc = jm_run(buf, (size_t)n);
    free(buf);
    if (rc != 0) {
        fprintf(stderr, "%s\n", jm_last_error());
        return 1;
    }
    return 0;
}

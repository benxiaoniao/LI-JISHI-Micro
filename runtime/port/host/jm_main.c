#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "jishim.h"

/* 主机端口登记的原生表（见 jm_host_native.c） */
int jm_host_native_register(void);

#ifdef _WIN32
#include <windows.h>
/* argv 是系统 ANSI（中文 Windows 为 GBK）；输出前转成 UTF-8，避免中文路径乱码。 */
static const char *u8arg(const char *s) {
    static char buf[4][1024];
    static int slot = 0;
    char *out = buf[slot];
    int wlen;
    wchar_t *w;
    int n;
    slot = (slot + 1) & 3;
    wlen = MultiByteToWideChar(CP_ACP, 0, s, -1, NULL, 0);
    if (wlen <= 0) {
        return s;
    }
    w = (wchar_t *)malloc(sizeof(wchar_t) * (size_t)wlen);
    if (!w) {
        return s;
    }
    MultiByteToWideChar(CP_ACP, 0, s, -1, w, wlen);
    n = WideCharToMultiByte(CP_UTF8, 0, w, -1, out, (int)sizeof buf[0], NULL, NULL);
    free(w);
    return n > 0 ? out : s;
}
#else
#define u8arg(s) (s)
#endif

#ifdef JM_HOST_DEBUG
/* 只在主机端口做调试接入：运行时本身不解析调试段，解析全部交给 micc。 */
static const char *g_jmb = NULL;
static int g_bp_code = -1;
static int g_bp_off = 0;

static const char *micc_path(void) {
    const char *e = getenv("MICC");
    if (e && *e) {
        return e;
    }
#ifdef _WIN32
    return "compiler\\target\\release\\micc.exe";
#else
    return "compiler/target/release/micc.exe";
#endif
}

/* 调 micc 并读第一行输出；退出码非 0（例如没有调试段）返回 -1。 */
static int micc_query(const char *args, char *out, size_t cap) {
    char cmd[1024];
    const char *mp = micc_path();
    /* cmd.exe 对「多个引号开头」的命令行会剥掉首尾引号，因此可执行路径不套引号；
       路径含空格时用 call 包一层，避开这个解析陷阱。 */
    if (strchr(mp, ' ') != NULL) {
        snprintf(cmd, sizeof cmd, "call \"%s\" %s 2>&1", mp, args);
    } else {
        snprintf(cmd, sizeof cmd, "%s %s 2>&1", mp, args);
    }
    FILE *p = popen(cmd, "r");
    if (!p) {
        return -1;
    }
    size_t n = fread(out, 1, cap - 1, p);
    out[n] = '\0';
    int rc = pclose(p);
    return rc == 0 ? 0 : -1;
}

/* 代码对象 + 偏移 → `文件:行`。没有调试段时返回 0，不报错。 */
static int lookup_line(int code, int off, char *out, size_t cap) {
    char args[512];
    char raw[512];
    snprintf(args, sizeof args, "--line-of \"%s\" %d %d", g_jmb, code, off);
    if (micc_query(args, raw, sizeof raw) != 0) {
        return 0;
    }
    size_t n = strlen(raw);
    while (n > 0 && (raw[n - 1] == '\n' || raw[n - 1] == '\r')) {
        raw[--n] = '\0';
    }
    if (n == 0 || n >= cap) {
        return 0;
    }
    memcpy(out, raw, n + 1);
    return 1;
}

/* jm_vm.c 在每个指令前调用；命中则打印源码行并让 VM 正常收尾退出。 */
int jm_host_break_hit(uint8_t code, uint16_t ip) {
    if (g_bp_code < 0 || (int)code != g_bp_code || (int)ip != g_bp_off) {
        return 0;
    }
    char loc[512];
    if (lookup_line(code, ip, loc, sizeof loc)) {
        printf("断点：%s\n", loc);
    } else {
        printf("断点：代码 %u 偏移 %u\n", (unsigned)code, (unsigned)ip);
    }
    fflush(stdout);
    return 1;
}

/* `文件:行` → 首个指令位置；没有调试段或该行没有指令时返回 -1。 */
static int resolve_break(const char *spec) {
    const char *colon = strrchr(spec, ':');
    if (!colon) {
        return -1;
    }
    size_t flen = (size_t)(colon - spec);
    if (flen == 0 || flen >= 512) {
        return -1;
    }
    char file[512];
    memcpy(file, spec, flen);
    file[flen] = '\0';
    int line = atoi(colon + 1);
    if (line <= 0) {
        return -1;
    }
    char args[768];
    char raw[1024];
    snprintf(args, sizeof args, "--lines-of \"%s\" \"%s\" %d", g_jmb, file, line);
    if (micc_query(args, raw, sizeof raw) != 0) {
        return -1;
    }
    const char *pc = strstr(raw, "代码 ");
    const char *po = strstr(raw, "偏移 ");
    if (!pc || !po) {
        return -1;
    }
    g_bp_code = atoi(pc + strlen("代码 "));
    g_bp_off = atoi(po + strlen("偏移 "));
    return g_bp_code >= 0 ? 0 : -1;
}
#endif

int main(int argc, char **argv) {
    const char *path = NULL;
    const char *brk = NULL;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--break") == 0 && i + 1 < argc) {
            brk = argv[++i];
        } else if (!path) {
            path = argv[i];
        } else {
            fprintf(stderr, "用法：jmrun [--break <文件>:<行>] <文件.jmb>\n");
            return 2;
        }
    }
    if (!path) {
        fprintf(stderr, "用法：jmrun [--break <文件>:<行>] <文件.jmb>\n");
        return 2;
    }
#ifdef JM_HOST_DEBUG
    g_jmb = path;
#endif

    if (brk) {
#ifdef JM_HOST_DEBUG
        if (resolve_break(brk) != 0) {
            fprintf(stderr, "断点无法落地：%s（文件:行无效，或没有调试信息）\n", u8arg(brk));
            return 2;
        }
#else
        fprintf(stderr, "这个 jmrun 没有编译调试接入（缺 JM_HOST_DEBUG）\n");
        return 2;
#endif
    }

    FILE *f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "打不开 %s\n", u8arg(path));
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

    if (jm_host_native_register() != 0) {
        fprintf(stderr, "原生表登记失败（任务模块）\n");
        free(buf);
        return 2;
    }

    int rc = jm_run(buf, (size_t)n);
#if JM_STACK_WATERMARK
    /* 观测构建专用：把本次运行的值栈峰值打到 stdout，便于按示例采集。 */
    {
        extern uint16_t jm_stack_hw;
        printf("[值栈峰值] %u 槽 / %d\n", (unsigned)jm_stack_hw, (int)JM_STACK_SLOTS);
    }
#endif
    if (rc != 0) {
        fprintf(stderr, "%s\n", jm_last_error());
#ifdef JM_HOST_DEBUG
        uint8_t code = 0;
        uint16_t ip = 0;
        char loc[512];
        if (jm_last_error_site(&code, &ip) == 0 &&
            lookup_line((int)code, (int)ip, loc, sizeof loc)) {
            fprintf(stderr, "于 %s\n", loc);
        }
#endif
        free(buf);
        return 1;
    }
    free(buf);
    return 0;
}

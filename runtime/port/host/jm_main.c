#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "jishim.h"
#include "jm_debug.h"

/* 交互调试的容量上限：断点数与预取行表条目数（只在宿主进程，纯静态数组）。 */
#define DBG_MAX_BP 8
#define DBG_LINE_MAX 2048

/* 主机端口登记的原生表（见 jm_host_native.c） */
int jm_host_native_register(void);

#ifdef _WIN32
#include <windows.h>
#include <shellapi.h>

/* UTF-8 → 宽字符；失败返回 NULL。 */
static wchar_t *jm_wide(const char *s) {
    int len = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    wchar_t *w;
    if (len <= 0) {
        return NULL;
    }
    w = (wchar_t *)malloc(sizeof(wchar_t) * (size_t)len);
    if (!w) {
        return NULL;
    }
    if (MultiByteToWideChar(CP_UTF8, 0, s, -1, w, len) <= 0) {
        free(w);
        return NULL;
    }
    return w;
}

/* 默认 main 的 argv 是系统 ANSI（CP_ACP）：en-US 的 runner 上中文路径会变成 '?'，
   .jmb 直接打不开。这里用系统 Unicode 命令行重建一份 UTF-8 的 argv。 */
static char **jm_utf8_argv(int *out_argc) {
    int n = 0;
    wchar_t **wargs = CommandLineToArgvW(GetCommandLineW(), &n);
    char **out;
    if (!wargs || n <= 0) {
        return NULL;
    }
    out = (char **)calloc((size_t)n + 1, sizeof(char *));
    if (!out) {
        LocalFree(wargs);
        return NULL;
    }
    for (int i = 0; i < n; i++) {
        int len = WideCharToMultiByte(CP_UTF8, 0, wargs[i], -1, NULL, 0, NULL, NULL);
        if (len > 0) {
            out[i] = (char *)malloc((size_t)len);
            if (out[i]) {
                WideCharToMultiByte(CP_UTF8, 0, wargs[i], -1, out[i], len, NULL, NULL);
            }
        }
        if (!out[i]) {
            out[i] = (char *)"";
        }
    }
    out[n] = NULL;
    LocalFree(wargs);
    *out_argc = n;
    return out;
}

/* Windows 的 fopen 走 ANSI；中文文件名在非中文 locale 上打不开，先转宽字符。 */
static FILE *jm_open_utf8(const char *path, const char *mode) {
    wchar_t *wpath = jm_wide(path);
    wchar_t *wmode = jm_wide(mode);
    FILE *f = NULL;
    if (wpath && wmode) {
        f = _wfopen(wpath, wmode);
    }
    free(wpath);
    free(wmode);
    return f ? f : fopen(path, mode);
}

/* 管道调 micc：命令行里带中文路径，同样转宽字符走 _wpopen。 */
static FILE *jm_popen_utf8(const char *cmd, const char *mode) {
    wchar_t *wcmd = jm_wide(cmd);
    wchar_t *wmode = jm_wide(mode);
    FILE *p = NULL;
    if (wcmd && wmode) {
        p = _wpopen(wcmd, wmode);
    }
    free(wcmd);
    free(wmode);
    return p ? p : popen(cmd, mode);
}

/* argv 已经是 UTF-8，打印时不用再转。 */
#define u8arg(s) (s)
#else
#define u8arg(s) (s)
#define jm_open_utf8(path, mode) fopen((path), (mode))
#define jm_popen_utf8(cmd, mode) popen((cmd), (mode))
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
    FILE *p = jm_popen_utf8(cmd, "r");
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
static int resolve_line(const char *spec, int *out_code, int *out_off) {
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
    *out_code = atoi(pc + strlen("代码 "));
    *out_off = atoi(po + strlen("偏移 "));
    return *out_code >= 0 ? 0 : -1;
}

/* 旧单断点：整程序运行到命中为止（行为保持不变，回归脚本依赖它）。 */
static int resolve_break(const char *spec) {
    if (resolve_line(spec, &g_bp_code, &g_bp_off) != 0) {
        return -1;
    }
    return 0;
}
#endif

#if defined(JM_HOST_DEBUG) && JM_DEBUG_HOOKS
/* ---- 交互式调试（M2：多断点 / 行级单步 / 步过 / task 过滤） ---- */

typedef struct {
    int code;
    int off;
    int hit;        /* 已命中次数：从 0 开始，命中时先 +1 */
    int hit_mode;   /* 0=每次停 1=#N 只在第 N 次停 2=%N 每 N 次停 */
    int hit_n;
    int cond_kind;  /* 0=无条件 1=当前帧局部 2=全局 */
    int cond_slot;
    char cond_name[96];
    int cond_op;    /* 0 ==  1 !=  2 <  3 <=  4 >  5 >= */
    int lit_kind;   /* 1=整数 2=文本 3=布尔 */
    long lit_int;
    char lit_text[96];
    int warned;     /* 条件求值失败只警告一次 */
} DbgBreak;

typedef struct {
    uint8_t code;
    uint8_t file;
    uint16_t off;
    uint16_t line;
} DbgLine;

static DbgBreak g_bps[DBG_MAX_BP];
static int g_nbps = 0;
static DbgLine g_lines[DBG_LINE_MAX];
static int g_nlines = 0;

static int g_dbg_on = 0;
static int g_mode = 0;      /* 0=继续到断点 1=单步指令 2=行级 3=步过 */
/* 1 = 每次暂停向 stderr 追加一行 NDJSON（编辑器读，人读文本照旧保留）。 */
static int g_stream = 0;
/* 本次暂停的原因：0=断点 1=单步指令 2=行级 3=步过。 */
static int g_pause_reason = 0;
static int g_step_task = -1;/* 被单步的任务（暂停时的当前任务） */
static int g_step_fp = 0;   /* 起始帧深（0=最外层） */
static int g_step_file = -1;
static int g_step_line = -1;
static int g_suppress = 0;  /* 恢复后放过暂停点本身一次 */
static int g_last_code = -1;
static int g_last_ip = -1;
static int g_task_filter = -1; /* -1=任意绿色任务 */
static int g_task_checked = 0;
static int g_filter_dead = 0;

/* 预取整份行表：一次 popen，行级判定不再逐指令调 micc。 */
static void load_line_table(void) {
    static char raw[32768];
    char args[512];
    snprintf(args, sizeof args, "--line-table \"%s\"", g_jmb);
    if (micc_query(args, raw, sizeof raw) != 0) {
        return;
    }
    char *p = raw;
    while (*p) {
        unsigned code = 0, off = 0, file = 0, line = 0;
        int used = 0;
        if (sscanf(p, "%u\t%u\t%u\t%u%n", &code, &off, &file, &line, &used) < 4) {
            break;
        }
        p += used;
        while (*p == '\r' || *p == '\n') {
            p++;
        }
        if (g_nlines >= DBG_LINE_MAX) {
            break;
        }
        DbgLine *e = &g_lines[g_nlines++];
        e->code = (uint8_t)code;
        e->file = (uint8_t)file;
        e->off = (uint16_t)off;
        e->line = (uint16_t)line;
    }
}

/* 代码 + 偏移 → 行身份；表按 (代码, 偏移) 升序，二分找最后一条 <= 目标。 */
static int line_lookup(int code, int off, int *file, int *line) {
    int lo = 0, hi = g_nlines - 1, best = -1;
    while (lo <= hi) {
        int mid = lo + (hi - lo) / 2;
        const DbgLine *e = &g_lines[mid];
        if ((int)e->code < code || ((int)e->code == code && (int)e->off <= off)) {
            best = mid;
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    if (best < 0 || (int)g_lines[best].code != code) {
        return 0;
    }
    *file = (int)g_lines[best].file;
    *line = (int)g_lines[best].line;
    return 1;
}

/* 当前代码对象里按名字找局部槽；找不到返回 0（调用方按全局名处理）。 */
static int find_local_slot(int code, const char *name, int *slot) {
    static char raw[4096];
    char args[512];
    snprintf(args, sizeof args, "--locals-of \"%s\" %d", g_jmb, code);
    if (micc_query(args, raw, sizeof raw) != 0) {
        return 0;
    }
    char *p = raw;
    while (*p) {
        unsigned s = 0;
        char nm[128];
        int used = 0;
        if (sscanf(p, "%u\t%127[^\n]%n", &s, nm, &used) < 2) {
            break;
        }
        p += used;
        while (*p == '\r' || *p == '\n') {
            p++;
        }
        if (strcmp(nm, name) == 0) {
            *slot = (int)s;
            return 1;
        }
    }
    return 0;
}

/* 规格：`文件:行 [if 名字 比较符 字面量] [#N|%N]`
 * 比较符 == != < <= > >=；字面量 = 十进制整数 / 真|假 / "文本"。
 * 解析失败时不改 g_bps。 */
static int parse_spec(const char *spec, DbgBreak *bp, char *msg, size_t cap) {
    char s[640];
    snprintf(s, sizeof s, "%s", spec);
    memset(bp, 0, sizeof *bp);

    /* 1) 尾部命中标记：#N（第 N 次停）或 %N（每 N 次停） */
    char *hash = strrchr(s, '#');
    char *pct = strrchr(s, '%');
    char *mark = hash && pct ? (hash > pct ? hash : pct) : (hash ? hash : pct);
    if (mark) {
        char *num = mark + 1;
        char *endp = NULL;
        long v = strtol(num, &endp, 10);
        if (!*num || (endp && *endp != '\0') || v <= 0) {
            snprintf(msg, cap, "命中计数写法无效：%s（应为 #N 或 %%N，N≥1）", spec);
            return -1;
        }
        bp->hit_mode = (*mark == '#') ? 1 : 2;
        bp->hit_n = (int)v;
        *mark = '\0';
        size_t L = strlen(s);
        while (L > 0 && s[L - 1] == ' ') {
            s[--L] = '\0';
        }
    }

    /* 2) 条件：` if 名字 比较符 字面量`；名字先按局部槽找，找不到按全局 */
    char *ifp = strstr(s, " if ");
    const char *loc = s;
    if (ifp) {
        *ifp = '\0';
    }
    /* 先解析位置：条件里的局部名要靠 code 去查符号表 */
    if (resolve_line(loc, &bp->code, &bp->off) != 0) {
        snprintf(msg, cap, "断点无法落地：%s（文件:行无效，或没有调试信息）", spec);
        return -1;
    }
    if (ifp) {
        const char *c = ifp + 4;
        while (*c == ' ') {
            c++;
        }
        char name[96];
        char op[4];
        int used1 = 0, used2 = 0;
        if (sscanf(c, "%95s%n %3s%n", name, &used1, op, &used2) != 2 || used1 == 0 ||
            used2 == 0) {
            snprintf(msg, cap, "条件写法无效：%s（应为 if 名字 比较符 字面量）", spec);
            return -1;
        }
        if (strcmp(op, "==") == 0) {
            bp->cond_op = 0;
        } else if (strcmp(op, "!=") == 0) {
            bp->cond_op = 1;
        } else if (strcmp(op, "<") == 0) {
            bp->cond_op = 2;
        } else if (strcmp(op, "<=") == 0) {
            bp->cond_op = 3;
        } else if (strcmp(op, ">") == 0) {
            bp->cond_op = 4;
        } else if (strcmp(op, ">=") == 0) {
            bp->cond_op = 5;
        } else {
            snprintf(msg, cap, "比较符只支持 == != < <= > >=：%s", op);
            return -1;
        }
        const char *litp = c + used2;
        while (*litp == ' ') {
            litp++;
        }
        char litbuf[200];
        snprintf(litbuf, sizeof litbuf, "%s", litp);
        size_t llen = strlen(litbuf);
        while (llen > 0 && (litbuf[llen - 1] == ' ' || litbuf[llen - 1] == '\t' ||
                            litbuf[llen - 1] == '\r')) {
            litbuf[--llen] = '\0';
        }
        if (litbuf[0] == '"') {
            if (llen < 2 || litbuf[llen - 1] != '"') {
                snprintf(msg, cap, "文本字面量缺少结尾引号：%s", spec);
                return -1;
            }
            size_t nch = llen - 2;
            if (nch >= sizeof bp->lit_text) {
                snprintf(msg, cap, "文本字面量太长（上限 %d 字节）", (int)sizeof bp->lit_text - 1);
                return -1;
            }
            memcpy(bp->lit_text, litbuf + 1, nch);
            bp->lit_text[nch] = '\0';
            bp->lit_kind = 2;
        } else if (strcmp(litbuf, "真") == 0) {
            bp->lit_kind = 3;
            bp->lit_int = 1;
        } else if (strcmp(litbuf, "假") == 0) {
            bp->lit_kind = 3;
            bp->lit_int = 0;
        } else {
            char *endp = NULL;
            long v = strtol(litbuf, &endp, 10);
            if (litbuf[0] && endp && *endp == '\0') {
                if (v > 268435455L || v < -268435456L) {
                    snprintf(msg, cap, "整数超出砾整数范围（-268435456..268435455）：%ld", v);
                    return -1;
                }
                bp->lit_kind = 1;
                bp->lit_int = v;
            } else if (litbuf[0] && strchr(litbuf, ' ') == NULL) {
                /* 裸文本：命令行里嵌双引号会被 shell 吃掉，留一条免转义写法。
                 * 文本含空格时仍要用 "文本"。 */
                if (llen >= sizeof bp->lit_text) {
                    snprintf(msg, cap, "文本字面量太长（上限 %d 字节）",
                             (int)sizeof bp->lit_text - 1);
                    return -1;
                }
                snprintf(bp->lit_text, sizeof bp->lit_text, "%s", litbuf);
                bp->lit_kind = 2;
            } else {
                snprintf(msg, cap,
                         "字面量只支持整数、真/假、文本（含空格时用 \"文本\" 包起来）：%s", litbuf);
                return -1;
            }
        }
        snprintf(bp->cond_name, sizeof bp->cond_name, "%s", name);
        bp->cond_kind = 2; /* 先按全局；下面若在局部符号表里找到就改回 1 */
        int slot = -1;
        if (find_local_slot(bp->code, bp->cond_name, &slot)) {
            bp->cond_kind = 1;
            bp->cond_slot = slot;
        }
    }

    return 0;
}

/* 加断点：重复设置视为成功（保留先设的条件）；超上限返回 -1 并给出说明。 */
static int bp_add(const char *spec, char *msg, size_t cap) {
    DbgBreak nb;
    if (parse_spec(spec, &nb, msg, cap) != 0) {
        return -1;
    }
    for (int i = 0; i < g_nbps; i++) {
        if (g_bps[i].code == nb.code && g_bps[i].off == nb.off) {
            return 0;
        }
    }
    if (g_nbps >= DBG_MAX_BP) {
        snprintf(msg, cap, "断点最多 %d 个，已拒绝：%s", DBG_MAX_BP, spec);
        return -1;
    }
    g_bps[g_nbps++] = nb;
    return 0;
}

static const char *cond_op_text(int op) {
    static const char *ops[6] = {"==", "!=", "<", "<=", ">", ">="};
    return op >= 0 && op < 6 ? ops[op] : "?";
}

static const char *cond_kind_text(int kind) {
    switch (kind) {
    case JM_DBG_KIND_INT:
        return "整数";
    case JM_DBG_KIND_TEXT:
        return "文本";
    case JM_DBG_KIND_BOOL:
        return "布尔";
    default:
        return "非整数/文本/布尔值";
    }
}

/* 条件求值（宿主侧，只在命中时做一次）。返回 1=成立 0=不成立 -1=求值失败。
 * 局部名读「当前执行帧」：tick 时 tasks[].fp 还没保存，必须走 jm_dbg_local_current。 */
static int cond_eval(const DbgBreak *bp, char *why, size_t cap) {
    JmValue v;
    if (bp->cond_kind == 1) {
        if (jm_dbg_local_current((uint8_t)bp->cond_slot, &v) != 0) {
            snprintf(why, cap, "局部变量 %s 尚未赋值", bp->cond_name);
            return -1;
        }
    } else {
        if (jm_dbg_global(bp->cond_name, &v) != 0) {
            snprintf(why, cap, "名字 %s 不存在或尚未赋值", bp->cond_name);
            return -1;
        }
    }
    int kind = jm_dbg_value_kind(v);
    if (bp->lit_kind == 1) {
        if (kind != JM_DBG_KIND_INT) {
            snprintf(why, cap, "类型不匹配：%s 是%s，字面量是整数", bp->cond_name,
                     cond_kind_text(kind));
            return -1;
        }
        long a = (long)jm_dbg_value_int(v);
        long b = bp->lit_int;
        switch (bp->cond_op) {
        case 0: return a == b;
        case 1: return a != b;
        case 2: return a < b;
        case 3: return a <= b;
        case 4: return a > b;
        default: return a >= b;
        }
    }
    if (bp->lit_kind == 2) {
        if (kind != JM_DBG_KIND_TEXT) {
            snprintf(why, cap, "类型不匹配：%s 是%s，字面量是文本", bp->cond_name,
                     cond_kind_text(kind));
            return -1;
        }
        if (bp->cond_op > 1) {
            snprintf(why, cap, "文本只支持 == 和 !=，不支持 %s", cond_op_text(bp->cond_op));
            return -1;
        }
        char txt[256];
        if (jm_dbg_value_text(v, txt, sizeof txt) != 0) {
            snprintf(why, cap, "文本太长，无法比较");
            return -1;
        }
        int eq = strcmp(txt, bp->lit_text) == 0;
        return bp->cond_op == 0 ? eq : !eq;
    }
    if (kind != JM_DBG_KIND_BOOL) {
        snprintf(why, cap, "类型不匹配：%s 是%s，字面量是布尔", bp->cond_name,
                 cond_kind_text(kind));
        return -1;
    }
    if (bp->cond_op > 1) {
        snprintf(why, cap, "布尔只支持 == 和 !=，不支持 %s", cond_op_text(bp->cond_op));
        return -1;
    }
    int bv = jm_dbg_value_bool(v);
    int want = (int)bp->lit_int;
    return bp->cond_op == 0 ? (bv == want) : (bv != want);
}

/* 每指令钩子：只在宿主做匹配，VM 里只有这一处调用。 */
int jm_host_dbg_tick(uint8_t code, uint16_t ip, uint8_t fp) {
    if (!g_dbg_on) {
        return 0;
    }
    int task = jm_dbg_current_task();

    /* task 过滤：只在第一次遇到断点地址时判定。不能放在首次 tick——
     * 子任务是程序跑起来后才创建的，那时总数还是 1，会误判「不存在」。
     * 判定后若该任务后续结束，也报一次，此后断点不再命中。 */
    if (g_task_filter >= 0 && g_task_checked && !g_filter_dead &&
        jm_dbg_task_state(g_task_filter) == 4) {
        fprintf(stderr, "任务 %d 已结束（断点不再命中，继续到结束）\n", g_task_filter);
        g_filter_dead = 1;
    }
    int task_ok = g_task_filter < 0 || (!g_filter_dead && task == g_task_filter);

    if (g_suppress && (int)code == g_last_code && (int)ip == g_last_ip) {
        g_suppress = 0;
        return 0; /* 刚从这条指令暂停恢复，不再重复停 */
    }

    /* 断点：任一命中即停；task 过滤不满足时本地址不算命中。 */
    for (int i = 0; i < g_nbps; i++) {
        if (g_bps[i].code == (int)code && g_bps[i].off == (int)ip) {
            if (g_task_filter >= 0 && !g_task_checked) {
                g_task_checked = 1;
                int total = jm_dbg_task_count();
                if (g_task_filter >= total) {
                    fprintf(stderr, "任务 %d 不存在（当前共 %d 个）\n", g_task_filter, total);
                    g_filter_dead = 1;
                } else if (jm_dbg_task_state(g_task_filter) == 4) {
                    fprintf(stderr, "任务 %d 已结束（断点不再命中，继续到结束）\n",
                            g_task_filter);
                    g_filter_dead = 1;
                }
                task_ok = g_task_filter < 0 || (!g_filter_dead && task == g_task_filter);
            }
            if (!task_ok) {
                break;
            }
            /* 命中计数：先 +1，再按 #N（第 N 次）或 %N（每 N 次）判断。 */
            DbgBreak *bp = &g_bps[i];
            bp->hit++;
            if (bp->hit_mode == 1 && bp->hit != bp->hit_n) {
                continue;
            }
            if (bp->hit_mode == 2 && (bp->hit % bp->hit_n) != 0) {
                continue;
            }
            if (bp->cond_kind) {
                char why[160];
                int r = cond_eval(bp, why, sizeof why);
                if (r < 0) {
                    if (!bp->warned) {
                        bp->warned = 1;
                        fprintf(stderr, "警告：断点 #%d 条件求值失败（%s），按不成立处理\n", i,
                                why);
                    }
                    continue; /* 不成立即继续，不中断程序 */
                }
                if (!r) {
                    continue;
                }
            }
            g_pause_reason = 0;
            g_last_code = (int)code;
            g_last_ip = (int)ip;
            return 1;
        }
    }

    if (g_mode == 1 && task_ok && task == g_step_task) {
        g_pause_reason = 1;
        g_mode = 0;
        g_last_code = (int)code;
        g_last_ip = (int)ip;
        return 1;
    }
    /* 行级 / 步过：行身份变化才停；步过另外要求帧深不超过起始帧深。 */
    if (g_mode >= 2 && task == g_step_task) {
        int file = 0, line = 0;
        if (line_lookup((int)code, (int)ip, &file, &line)) {
            int changed = file != g_step_file || line != g_step_line;
            int depth_ok = g_mode == 2 || (int)fp <= g_step_fp;
            if (changed && depth_ok) {
                g_pause_reason = g_mode;
                g_mode = 0;
                g_last_code = (int)code;
                g_last_ip = (int)ip;
                return 1;
            }
        }
    }
    return 0;
}

static int cur_frames(JmDbgFrame *fs, int max) {
    return jm_dbg_frames(jm_dbg_current_task(), fs, max);
}

/* ---- 事件流（NDJSON，写给编辑器；人读文本照旧留在 stderr） ---- */

/** JSON 字符串转义：只处理会破坏单行 JSON 的字符。 */
static void json_escape(const char *s, char *out, size_t cap) {
    size_t o = 0;
    for (; s && *s && o + 2 < cap; s++) {
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') {
            out[o++] = '\\';
            out[o++] = (char)c;
        } else if (c == '\n') {
            out[o++] = '\\';
            out[o++] = 'n';
        } else if (c == '\r') {
            out[o++] = '\\';
            out[o++] = 'r';
        } else if (c == '\t') {
            out[o++] = '\\';
            out[o++] = 't';
        } else if (c < 0x20) {
            out[o++] = ' ';
        } else {
            out[o++] = (char)c;
        }
    }
    out[o] = '\0';
}

static const char *pause_reason_text(void) {
    switch (g_pause_reason) {
    case 1:
        return "单步";
    case 2:
        return "行级";
    case 3:
        return "步过";
    default:
        return "断点";
    }
}

static void stream_pause(int task, const char *func, const char *file, int line, int has_line) {
    if (!g_stream) {
        return;
    }
    char fn[256];
    char fl[640];
    json_escape(func ? func : "?", fn, sizeof fn);
    json_escape(file ? file : "", fl, sizeof fl);
    if (has_line) {
        fprintf(stderr,
                "{\"事件\":\"暂停\",\"任务\":%d,\"函数\":\"%s\",\"文件\":\"%s\",\"行\":%d,"
                "\"原因\":\"%s\"}\n",
                task, fn, fl, line, pause_reason_text());
    } else {
        fprintf(stderr,
                "{\"事件\":\"暂停\",\"任务\":%d,\"函数\":\"%s\",\"文件\":\"\",\"行\":0,"
                "\"原因\":\"%s\"}\n",
                task, fn, pause_reason_text());
    }
    fflush(stderr);
}

/** 程序结束（正常 / 故障 / 用户退出）统一发一条结束事件。 */
static void stream_finish(int code, int quit) {
    if (!g_stream) {
        return;
    }
    /* REPL 的提示符 `(砾调试) ` 不换行，先补一个换行让事件独占一行。 */
    fputc('\n', stderr);
    if (quit) {
        fprintf(stderr, "{\"事件\":\"结束\",\"码\":%d,\"用户退出\":true}\n", code);
    } else {
        fprintf(stderr, "{\"事件\":\"结束\",\"码\":%d}\n", code);
    }
    fflush(stderr);
}

static void dbg_where(void) {
    JmDbgFrame fs[16];
    int n = cur_frames(fs, 16);
    if (n <= 0) {
        return;
    }
    const char *name = "?";
    jm_dbg_code_name(fs[0].code, &name);
    char loc[512];
    if (lookup_line((int)fs[0].code, (int)fs[0].ip, loc, sizeof loc)) {
        fprintf(stderr, "暂停：任务 %d 「%s」 %s\n", jm_dbg_current_task(), name, loc);
        char file[640] = "";
        int line = 0;
        const char *colon = strrchr(loc, ':');
        if (colon && colon != loc) {
            size_t len = (size_t)(colon - loc);
            if (len < sizeof file) {
                memcpy(file, loc, len);
                file[len] = '\0';
                line = atoi(colon + 1);
            }
        }
        stream_pause(jm_dbg_current_task(), name, file, line, line > 0);
    } else {
        fprintf(stderr, "暂停：任务 %d 「%s」 代码 %u 偏移 %u\n", jm_dbg_current_task(), name,
                (unsigned)fs[0].code, (unsigned)fs[0].ip);
        stream_pause(jm_dbg_current_task(), name, "", 0, 0);
    }
}

/* 当前帧的局部变量：槽号与名字来自 `micc --locals-of`（符号表在 .jmb 里）。 */
static void dbg_vars(void) {
    JmDbgFrame fs[16];
    int n = cur_frames(fs, 16);
    if (n <= 0) {
        return;
    }
    char args[512];
    char raw[1024];
    snprintf(args, sizeof args, "--locals-of \"%s\" %u", g_jmb, (unsigned)fs[0].code);
    if (micc_query(args, raw, sizeof raw) != 0) {
        fprintf(stderr, "（当前函数没有局部符号）\n");
        return;
    }
    int shown = 0;
    char *p = raw;
    while (*p) {
        unsigned slot = 0;
        char name[128];
        int used = 0;
        if (sscanf(p, "%u\t%127[^\n]%n", &slot, name, &used) < 2) {
            break;
        }
        p += used;
        while (*p == '\r' || *p == '\n') {
            p++;
        }
        JmValue v;
        if (jm_dbg_local(jm_dbg_current_task(), 0, (uint8_t)slot, &v) == 0) {
            char txt[256];
            if (jm_dbg_value_text(v, txt, sizeof txt) == 0) {
                fprintf(stderr, "  %s = %s\n", name, txt);
            } else {
                fprintf(stderr, "  %s = <太长>\n", name);
            }
        } else {
            fprintf(stderr, "  %s = <未赋值>\n", name);
        }
        shown = 1;
    }
    if (!shown) {
        fprintf(stderr, "（当前函数没有局部符号）\n");
    }
}

static void dbg_stack(void) {
    JmDbgFrame fs[16];
    int n = cur_frames(fs, 16);
    if (n <= 0) {
        fprintf(stderr, "（取不到调用栈）\n");
        return;
    }
    for (int i = 0; i < n; i++) {
        const char *name = "?";
        jm_dbg_code_name(fs[i].code, &name);
        char loc[512];
        if (lookup_line((int)fs[i].code, (int)fs[i].ip, loc, sizeof loc)) {
            fprintf(stderr, "  #%d 「%s」 %s\n", i, name, loc);
        } else {
            fprintf(stderr, "  #%d 「%s」 代码 %u 偏移 %u\n", i, name, (unsigned)fs[i].code,
                    (unsigned)fs[i].ip);
        }
    }
}

static void dbg_global(const char *name) {
    JmValue v;
    if (jm_dbg_global(name, &v) != 0) {
        fprintf(stderr, "  %s：没有这个全局变量或尚未赋值\n", name);
        return;
    }
    char txt[256];
    if (jm_dbg_value_text(v, txt, sizeof txt) == 0) {
        fprintf(stderr, "  %s = %s\n", name, txt);
    }
}

/* 以当前暂停点为基准启动行级/步过；后续 tick 只在行身份变化时停。 */
static void begin_step(int mode) {
    JmDbgFrame fs[16];
    int n = cur_frames(fs, 16);
    g_step_task = jm_dbg_current_task();
    g_step_fp = n > 0 ? n - 1 : 0;
    g_step_file = -1;
    g_step_line = -1;
    if (n > 0) {
        line_lookup((int)fs[0].code, (int)fs[0].ip, &g_step_file, &g_step_line);
    }
    g_mode = mode;
    g_suppress = 1;
}

static void dbg_break_list(void) {
    if (g_nbps == 0) {
        fprintf(stderr, "（没有断点）\n");
        return;
    }
    for (int i = 0; i < g_nbps; i++) {
        char loc[512];
        if (lookup_line(g_bps[i].code, g_bps[i].off, loc, sizeof loc)) {
            fprintf(stderr, "  #%d %s", i, loc);
        } else {
            fprintf(stderr, "  #%d 代码 %d 偏移 %d", i, g_bps[i].code, g_bps[i].off);
        }
        if (g_bps[i].cond_kind) {
            fprintf(stderr, "  if %s %s ", g_bps[i].cond_name, cond_op_text(g_bps[i].cond_op));
            if (g_bps[i].lit_kind == 1) {
                fprintf(stderr, "%ld", g_bps[i].lit_int);
            } else if (g_bps[i].lit_kind == 2) {
                fprintf(stderr, "\"%s\"", g_bps[i].lit_text);
            } else {
                fprintf(stderr, "%s", g_bps[i].lit_int ? "真" : "假");
            }
        }
        if (g_bps[i].hit_mode) {
            fprintf(stderr, "  %s%d", g_bps[i].hit_mode == 1 ? "#" : "%", g_bps[i].hit_n);
        }
        if (g_bps[i].hit) {
            fprintf(stderr, "  （已命中 %d）", g_bps[i].hit);
        }
        fprintf(stderr, "\n");
    }
}

/* 暂停后的交互循环。返回 0 = 程序已正常结束；1 = 用户退出不再继续；-1 = 故障。 */
static int dbg_repl(void) {
    for (;;) {
        dbg_where();
        fprintf(stderr, "(砾调试) ");
        fflush(stderr);
        char line[128];
        if (!fgets(line, sizeof line, stdin)) {
            return 1;
        }
        size_t len = strlen(line);
        while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r')) {
            line[--len] = '\0';
        }
        /* Windows 下用管道喂 UTF-8 输入时，第一行可能带 BOM；先剥掉再比命令。 */
        if (len >= 3 && (unsigned char)line[0] == 0xEF && (unsigned char)line[1] == 0xBB &&
            (unsigned char)line[2] == 0xBF) {
            memmove(line, line + 3, len - 2);
            len -= 3;
        }
        if (line[0] == '\0') {
            continue;
        }
        if (strcmp(line, "q") == 0 || strcmp(line, "退出") == 0) {
            return 1;
        }
        if (strcmp(line, "c") == 0 || strcmp(line, "继续") == 0) {
            g_mode = 0;
            g_suppress = 1;
        } else if (strcmp(line, "s") == 0 || strcmp(line, "单步") == 0) {
            g_mode = 1;
            g_suppress = 1;
        } else if (strcmp(line, "n") == 0 || strcmp(line, "下一行") == 0) {
            if (g_nlines == 0) {
                fprintf(stderr, "（没有行表：n/o 不可用）\n");
                continue;
            }
            begin_step(2);
        } else if (strcmp(line, "o") == 0 || strcmp(line, "步过") == 0) {
            if (g_nlines == 0) {
                fprintf(stderr, "（没有行表：n/o 不可用）\n");
                continue;
            }
            begin_step(3);
        } else if (strncmp(line, "b ", 2) == 0 || strncmp(line, "断点 ", 3) == 0) {
            const char *spec = line + (line[0] == 'b' ? 2 : 3);
            while (*spec == ' ') {
                spec++;
            }
            char msg[640];
            if (bp_add(spec, msg, sizeof msg) != 0) {
                fprintf(stderr, "%s\n", msg);
            } else {
                fprintf(stderr, "已设置断点 %d 个（上限 %d）\n", g_nbps, DBG_MAX_BP);
            }
            continue;
        } else if (strcmp(line, "bl") == 0 || strcmp(line, "断点列表") == 0) {
            dbg_break_list();
            continue;
        } else if (strcmp(line, "v") == 0 || strcmp(line, "变量") == 0) {
            dbg_vars();
            continue;
        } else if (strcmp(line, "bt") == 0 || strcmp(line, "栈") == 0 ||
                   strcmp(line, "调用栈") == 0) {
            dbg_stack();
            continue;
        } else if (strncmp(line, "g ", 2) == 0 || strncmp(line, "全局 ", 3) == 0) {
            const char *nm = line + (line[0] == 'g' ? 2 : 3);
            while (*nm == ' ') {
                nm++;
            }
            dbg_global(nm);
            continue;
        } else {
            fprintf(stderr,
                    "命令：c 继续 / s 单步指令 / n 下一行 / o 步过 / "
                    "b <文件>:<行> [if 名字 操作符 字面量] [#N|%%N] / bl 断点列表 / "
                    "v 变量 / bt 栈 / g <名字> 全局 / q 退出（未识别：「%s」）\n",
                    line);
            continue;
        }
        int rc = jm_dbg_resume();
        if (rc == 0) {
            return 0;
        }
        if (rc < 0) {
            return -1;
        }
    }
}
#endif

int main(int argc, char **argv) {
    const char *path = NULL;
    const char *brks[DBG_MAX_BP];
    static char brk_store[DBG_MAX_BP][640]; /* 断点规格（UTF-8） */
#ifdef _WIN32
    /* 默认 argv 是 ANSI；换成 Unicode 命令行重建的 UTF-8 版本。 */
    {
        int u8argc = 0;
        char **u8argv = jm_utf8_argv(&u8argc);
        if (u8argv) {
            argc = u8argc;
            argv = u8argv;
        }
    }
#endif
    int nbrks = 0;
    int task = -1;
    int dbg = 0;
    int stream = 0;
    const char *bpfile = NULL;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--break") == 0 && i + 1 < argc) {
            if (nbrks >= DBG_MAX_BP) {
                fprintf(stderr, "断点最多 %d 个\n", DBG_MAX_BP);
                return 2;
            }
            snprintf(brk_store[nbrks], sizeof brk_store[nbrks], "%s", u8arg(argv[++i]));
            brks[nbrks] = brk_store[nbrks];
            nbrks++;
        } else if (strcmp(argv[i], "--break-file") == 0 && i + 1 < argc) {
            bpfile = argv[++i];
        } else if (strcmp(argv[i], "--task") == 0 && i + 1 < argc) {
            char *end = NULL;
            task = (int)strtol(argv[++i], &end, 10);
            if (!end || *end != '\0' || task < 0) {
                fprintf(stderr, "--task 需要非负整数\n");
                return 2;
            }
        } else if (strcmp(argv[i], "--dbg") == 0) {
            dbg = 1;
        } else if (strcmp(u8arg(argv[i]), "--事件流") == 0 || strcmp(argv[i], "--ndjson") == 0) {
            stream = 1;
        } else if (!path) {
            path = argv[i];
        } else {
            fprintf(stderr,
                    "用法：jmrun [--break <规格>]... [--break-file bp.txt] [--task N] [--dbg] "
                    "<文件.jmb>\n");
            return 2;
        }
    }
    if (!path) {
        fprintf(stderr,
                "用法：jmrun [--break <规格>]... [--break-file bp.txt] [--task N] [--dbg] "
                "<文件.jmb>\n");
        return 2;
    }
    if (task >= 0 && !dbg) {
        fprintf(stderr, "--task 只能与 --dbg 一起用\n");
        return 2;
    }
    if (stream && !dbg) {
        fprintf(stderr, "--事件流 只能与 --dbg 一起用\n");
        return 2;
    }
#if !(defined(JM_HOST_DEBUG) && JM_DEBUG_HOOKS)
    if (dbg) {
        fprintf(stderr, "这个 jmrun 没有编译交互调试（缺 JM_DEBUG_HOOKS）\n");
        return 2;
    }
#endif
#ifdef JM_HOST_DEBUG
    g_jmb = path;
#endif

    if (nbrks > 0) {
#ifdef JM_HOST_DEBUG
#if JM_DEBUG_HOOKS
        if (dbg) {
            char msg[640];
            for (int i = 0; i < nbrks; i++) {
                if (bp_add(brks[i], msg, sizeof msg) != 0) {
                    fprintf(stderr, "%s\n", msg);
                    return 2;
                }
            }
        } else
#endif
        {
            if (nbrks > 1) {
                fprintf(stderr, "普通运行（不带 --dbg）只支持一个断点\n");
                return 2;
            }
            if (strstr(brks[0], " if ") || strchr(brks[0], '#') || strchr(brks[0], '%')) {
                fprintf(stderr, "条件断点与命中计数只能与 --dbg 一起用\n");
                return 2;
            }
            if (resolve_break(brks[0]) != 0) {
                fprintf(stderr, "断点无法落地：%s（文件:行无效，或没有调试信息）\n",
                        u8arg(brks[0]));
                return 2;
            }
        }
#else
        fprintf(stderr, "这个 jmrun 没有编译调试接入（缺 JM_HOST_DEBUG）\n");
        return 2;
#endif
    }

#if defined(JM_HOST_DEBUG) && JM_DEBUG_HOOKS
    if (bpfile) {
        if (!dbg) {
            fprintf(stderr, "--break-file 只能与 --dbg 一起用\n");
            return 2;
        }
        FILE *bf = jm_open_utf8(bpfile, "rb");
        if (!bf) {
            fprintf(stderr, "打不开断点脚本 %s\n", u8arg(bpfile));
            return 2;
        }
        char line[640];
        char msg[640];
        int ln = 0;
        while (fgets(line, sizeof line, bf)) {
            ln++;
            size_t L = strlen(line);
            while (L > 0 && (line[L - 1] == '\n' || line[L - 1] == '\r')) {
                line[--L] = '\0';
            }
            if (ln == 1 && L >= 3 && (unsigned char)line[0] == 0xEF &&
                (unsigned char)line[1] == 0xBB && (unsigned char)line[2] == 0xBF) {
                memmove(line, line + 3, L - 2);
            }
            char *p = line;
            while (*p == ' ' || *p == '\t') {
                p++;
            }
            L = strlen(p);
            while (L > 0 && (p[L - 1] == ' ' || p[L - 1] == '\t')) {
                p[--L] = '\0';
            }
            if (*p == '\0' || (p[0] == '/' && p[1] == '/')) {
                continue; /* 空行与 // 注释 */
            }
            if (bp_add(p, msg, sizeof msg) != 0) {
                fprintf(stderr, "断点脚本 %s 第 %d 行：%s\n", u8arg(bpfile), ln, msg);
                fclose(bf);
                return 2;
            }
        }
        fclose(bf);
    }
#else
    if (bpfile) {
        fprintf(stderr, "--break-file 需要带调试接入的 jmrun（缺 JM_HOST_DEBUG/JM_DEBUG_HOOKS）\n");
        return 2;
    }
#endif

    FILE *f = jm_open_utf8(path, "rb");
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

#if defined(JM_HOST_DEBUG) && JM_DEBUG_HOOKS
    if (dbg) {
        g_dbg_on = 1;
        g_stream = stream;
        g_task_filter = task;
        load_line_table();
        if (g_nbps == 0 && n >= 12) {
            g_bps[0].code = (int)buf[9]; /* 没给断点：停在入口代码第一条指令 */
            g_bps[0].off = 0;
            g_nbps = 1;
        }
    }
#endif

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
#if defined(JM_HOST_DEBUG) && JM_DEBUG_HOOKS
    if (rc == 1 && dbg) {
        int d = dbg_repl();
        if (d == 1) {
            /* 用户在暂停点退出：程序未跑完，按正常退出处理 */
            stream_finish(0, 1);
            free(buf);
            return 0;
        }
        rc = d;
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
#if defined(JM_HOST_DEBUG) && JM_DEBUG_HOOKS
        stream_finish(1, 0);
#endif
        return 1;
    }
#if defined(JM_HOST_DEBUG) && JM_DEBUG_HOOKS
    stream_finish(0, 0);
#endif
    free(buf);
    return 0;
}

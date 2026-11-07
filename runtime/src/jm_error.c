#include "jm_internal.h"
#include "jm_native.h"

static const char *msg_for(int code) {
    switch (code) {
    case E_STACK: return "值栈溢出";
    case E_UNDERFLOW: return "值栈下溢（内部错误）";
    case E_CONST: return "常量下标越界";
    case E_NAME: return "名字未定义";
    case E_NAMES: return "名字下标越界";
    case E_LOCAL: return "局部槽错误";
    case E_ZERO_DIV: return "除以零";
    case E_ZERO_MOD: return "取模零";
    case E_OVERFLOW: return "整数溢出";
    case E_TYPE: return "类型不支持该运算";
    case E_NOT_CALLABLE: return "这个值不能调用";
    case E_ARGC: return "实参个数不符";
    case E_INDEX_TYPE: return "下标类型错误";
    case E_INDEX: return "下标越界或键不存在";
    case E_NOT_ITER: return "这个值不能遍历";
    case E_ITER: return "迭代器错误";
    case E_OOM: return "堆内存耗尽";
    case E_OBJ: return "对象类型错误";
    case E_KEY: return "字典键不可用";
    case E_CODE: return "代码对象越界";
    case E_FRAMES: return "调用层数太深";
    case E_OPCODE: return "不支持的指令";
    case E_DIV_NONINT: return "真除法结果不是整数";
    case E_POW: return "幂指数非法";
    case E_IMPORT: return "模块导入尚未支持";
    case E_FILE: return "字节码文件有误";
    case JM_NE_PARAM: return "原生函数参数非法";
    case JM_NE_TYPE: return "原生函数参数类型不符";
    case JM_NE_HW: return "硬件操作失败";
    case JM_NE_STATE: return "原生函数状态错误";
    case JM_NE_NOMEM: return "原生函数内存不足";
    default: return "未知错误";
    }
}

static void app_str(char **p, char *end, const char *s) {
    while (*s && *p < end) {
        *(*p)++ = *s++;
    }
}

static void app_int(char **p, char *end, int v) {
    char tmp[12];
    int n = 0;
    if (v < 0) {
        app_str(p, end, "-");
        v = -v;
    }
    if (v == 0) {
        tmp[n++] = '0';
    }
    while (v > 0) {
        tmp[n++] = (char)('0' + (v % 10));
        v /= 10;
    }
    while (n > 0) {
        app_str(p, end, (char[]){tmp[--n], 0});
    }
}

void jm_error_format(char *buf, size_t cap) {
    if (cap == 0) {
        return;
    }
    char *p = buf;
    char *end = buf + cap - 1;
    JmState *st = &jm_state;
    app_str(&p, end, "错误[E");
    int code = st->has_err ? st->err.code : 0;
    if (code < 10) {
        app_str(&p, end, "00");
    } else if (code < 100) {
        app_str(&p, end, "0");
    }
    app_int(&p, end, code);
    app_str(&p, end, "] ");
    const char *msg = msg_for(code);
    app_str(&p, end, msg);
    if (st->has_err && st->err.detail && st->err.detail[0] &&
        strcmp(st->err.detail, msg) != 0) {
        app_str(&p, end, "：");
        app_str(&p, end, st->err.detail);
    }
    if (st->has_err) {
        app_str(&p, end, "（代码 ");
        app_int(&p, end, st->err.code_idx);
        app_str(&p, end, " 偏移 ");
        app_int(&p, end, st->err.ip);
        app_str(&p, end, "）");
    }
    *p = 0;
}

const char *jm_last_error(void) {
    static char buf[128];
    jm_error_format(buf, sizeof(buf));
    return buf;
}

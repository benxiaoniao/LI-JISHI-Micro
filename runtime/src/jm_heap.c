#include "jm_internal.h"

/* 静态堆：不用 malloc，块分配 + 相邻合并。 */

typedef struct {
    uint32_t size; /* 含头 */
    uint32_t free;
} JmBlk;

uint8_t jm_heap[JM_HEAP_SIZE] __attribute__((aligned(4)));

#define JM_BLK_HDR ((uint32_t)sizeof(JmBlk))
#define JM_HEAP_BYTES ((uint32_t)(JM_HEAP_SIZE & ~3u))
#define JM_HEAP_END ((uint8_t *)jm_heap + JM_HEAP_BYTES)

void jm_heap_init(void) {
    JmBlk *b = (JmBlk *)jm_heap;
    b->size = JM_HEAP_BYTES;
    b->free = 1;
}

uint32_t jm_heap_total(void) {
    return JM_HEAP_BYTES;
}

uint32_t jm_heap_used(void) {
    uint32_t used = 0;
    uint8_t *p = jm_heap;
    while (p < JM_HEAP_END) {
        JmBlk *b = (JmBlk *)p;
        if (b->size < JM_BLK_HDR || (uint8_t *)b + b->size > JM_HEAP_END) {
            break;
        }
        if (!b->free) {
            used += b->size;
        }
        p += b->size;
    }
    return used;
}

void *jm_raw_alloc(size_t n) {
    if (n == 0) {
        n = 4;
    }
    uint32_t need = ((uint32_t)n + 3u) & ~3u;
    uint32_t total = need + JM_BLK_HDR;

    uint8_t *p = jm_heap;
    while (p < JM_HEAP_END) {
        JmBlk *b = (JmBlk *)p;
        if (b->size < JM_BLK_HDR || (uint8_t *)b + b->size > JM_HEAP_END) {
            return NULL;
        }
        if (b->free && b->size >= total) {
            if (b->size - total >= JM_BLK_HDR + 16u) {
                JmBlk *rest = (JmBlk *)(p + total);
                rest->size = b->size - total;
                rest->free = 1;
                b->size = total;
            }
            b->free = 0;
            memset(p + JM_BLK_HDR, 0, b->size - JM_BLK_HDR);
            return p + JM_BLK_HDR;
        }
        p += b->size;
    }
    return NULL;
}

void jm_raw_free(void *ptr) {
    if (!ptr) {
        return;
    }
    uint8_t *p = (uint8_t *)ptr - JM_BLK_HDR;
    if (p < jm_heap || p >= JM_HEAP_END) {
        return;
    }
    JmBlk *b = (JmBlk *)p;
    b->free = 1;

    /* 向后合并 */
    while (1) {
        uint8_t *np = p + b->size;
        if (np >= JM_HEAP_END) {
            break;
        }
        JmBlk *nb = (JmBlk *)np;
        if (!nb->free) {
            break;
        }
        b->size += nb->size;
    }

    /* 向前合并：从头走一遍，堆很小，代价可接受 */
    uint8_t *q = jm_heap;
    JmBlk *prev = NULL;
    while (q < p) {
        JmBlk *qb = (JmBlk *)q;
        if (qb->size < JM_BLK_HDR) {
            break;
        }
        if (q + qb->size == p) {
            prev = qb;
            break;
        }
        q += qb->size;
    }
    if (prev && prev->free) {
        prev->size += b->size;
        /* 合并后可能还能吞掉下一块 */
        uint8_t *np = (uint8_t *)prev + prev->size;
        if (np < JM_HEAP_END) {
            JmBlk *nb = (JmBlk *)np;
            if (nb->free) {
                prev->size += nb->size;
            }
        }
    }
}

JmObj *jm_obj_alloc(uint16_t type, size_t payload) {
    JmObj *o = (JmObj *)jm_raw_alloc(sizeof(JmObj) + payload);
    if (!o) {
        return NULL;
    }
    o->type = type;
    o->rc = 1;
    return o;
}

JmStr *jm_str_new_obj(const char *s, uint32_t len) {
    JmStr *str = (JmStr *)jm_obj_alloc(
        JM_T_STR, (sizeof(JmStr) - sizeof(JmObj)) + len);
    if (!str) {
        return NULL;
    }
    str->len = len;
    if (len && s) {
        memcpy(str->data, s, len);
    }
    return str;
}

JmList *jm_list_new_obj(void) {
    JmList *l = (JmList *)jm_obj_alloc(JM_T_LIST, sizeof(JmList) - sizeof(JmObj));
    if (!l) {
        return NULL;
    }
    l->len = 0;
    l->cap = 0;
    l->items = NULL;
    return l;
}

JmDict *jm_dict_new_obj(void) {
    JmDict *d = (JmDict *)jm_obj_alloc(JM_T_DICT, sizeof(JmDict) - sizeof(JmObj));
    if (!d) {
        return NULL;
    }
    d->len = 0;
    d->cap = 0;
    d->entries = NULL;
    return d;
}

JmIter *jm_iter_new_obj(JmValue container, uint32_t kind) {
    JmIter *it = (JmIter *)jm_obj_alloc(JM_T_ITER, sizeof(JmIter) - sizeof(JmObj));
    if (!it) {
        return NULL;
    }
    it->container = container;
    it->idx = 0;
    it->kind = kind;
    jm_retain(container);
    return it;
}

JmRange *jm_range_new_obj(int32_t start, int32_t step, uint32_t len) {
    JmRange *r = (JmRange *)jm_obj_alloc(JM_T_RANGE, sizeof(JmRange) - sizeof(JmObj));
    if (!r) {
        return NULL;
    }
    r->start = start;
    r->step = step;
    r->len = len;
    return r;
}

JmDec *jm_dec_new_obj(int32_t raw) {
    JmDec *d = (JmDec *)jm_obj_alloc(JM_T_DEC, sizeof(JmDec) - sizeof(JmObj));
    if (!d) {
        return NULL;
    }
    d->raw = raw;
    return d;
}

int jm_list_reserve(JmState *st, JmList *l, uint32_t need) {
    if (l->cap >= need) {
        return 0;
    }
    uint32_t cap = l->cap ? l->cap : 4;
    while (cap < need) {
        cap *= 2;
        if (cap > 1000000u) {
            break;
        }
    }
    JmValue *items = (JmValue *)jm_raw_alloc((size_t)cap * sizeof(JmValue));
    if (!items) {
        return jm_fail(st, E_OOM, "列表扩容失败");
    }
    if (l->len) {
        memcpy(items, l->items, (size_t)l->len * sizeof(JmValue));
    }
    if (l->items) {
        jm_raw_free(l->items);
    }
    l->items = items;
    l->cap = cap;
    return 0;
}

int jm_list_push(JmState *st, JmList *l, JmValue v) {
    if (jm_list_reserve(st, l, l->len + 1)) {
        return -1;
    }
    l->items[l->len++] = v;
    return 0;
}

int jm_dict_set(JmState *st, JmDict *d, JmValue k, JmValue v) {
    for (uint32_t i = 0; i < d->len; i++) {
        if (jm_equal(d->entries[i].key, k)) {
            jm_release(d->entries[i].val);
            d->entries[i].val = v;
            jm_release(k);
            return 0;
        }
    }
    if (d->cap < d->len + 1) {
        uint32_t cap = d->cap ? d->cap * 2 : 4;
        JmEntry *e = (JmEntry *)jm_raw_alloc((size_t)cap * sizeof(JmEntry));
        if (!e) {
            return jm_fail(st, E_OOM, "字典扩容失败");
        }
        if (d->len) {
            memcpy(e, d->entries, (size_t)d->len * sizeof(JmEntry));
        }
        if (d->entries) {
            jm_raw_free(d->entries);
        }
        d->entries = e;
        d->cap = cap;
    }
    d->entries[d->len].key = k;
    d->entries[d->len].val = v;
    d->len++;
    return 0;
}

int jm_str_new(JmState *st, const char *s, uint32_t len, JmValue *out) {
    JmStr *str = jm_str_new_obj(s, len);
    if (!str) {
        return jm_fail(st, E_OOM, "创建文本失败");
    }
    *out = jm_heap_value(str);
    return 0;
}

int jm_list_new(JmState *st, JmValue *out) {
    JmList *l = jm_list_new_obj();
    if (!l) {
        return jm_fail(st, E_OOM, "创建列表失败");
    }
    *out = jm_heap_value(l);
    return 0;
}

int jm_dec_new(JmState *st, int32_t raw, JmValue *out) {
    JmDec *d = jm_dec_new_obj(raw);
    if (!d) {
        return jm_fail(st, E_OOM, "创建小数失败");
    }
    *out = jm_heap_value(d);
    return 0;
}

void jm_retain(JmValue v) {
    if (JM_TAG(v) == JM_TAG_HEAP) {
        JmObj *o = jm_obj(v);
        o->rc++;
    }
}

static void free_obj(JmValue v);

void jm_release(JmValue v) {
    if (JM_TAG(v) != JM_TAG_HEAP) {
        return;
    }
    JmObj *o = jm_obj(v);
    if (o->rc == 0) {
        return; /* 防御：重复释放不至于把 rc 弄成回绕 */
    }
    o->rc--;
    if (o->rc == 0) {
        free_obj(v);
    }
}

static void free_obj(JmValue v) {
    JmObj *o = jm_obj(v);
    switch (o->type) {
    case JM_T_STR:
        break;
    case JM_T_LIST: {
        JmList *l = (JmList *)o;
        for (uint32_t i = 0; i < l->len; i++) {
            jm_release(l->items[i]);
        }
        if (l->items) {
            jm_raw_free(l->items);
        }
        break;
    }
    case JM_T_DICT: {
        JmDict *d = (JmDict *)o;
        for (uint32_t i = 0; i < d->len; i++) {
            jm_release(d->entries[i].key);
            jm_release(d->entries[i].val);
        }
        if (d->entries) {
            jm_raw_free(d->entries);
        }
        break;
    }
    case JM_T_ITER: {
        JmIter *it = (JmIter *)o;
        jm_release(it->container);
        break;
    }
    case JM_T_RANGE:
        break;
    default:
        break;
    }
    jm_raw_free(o);
}

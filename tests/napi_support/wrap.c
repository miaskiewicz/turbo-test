/* An ObjectWrap-style addon: napi_wrap / napi_unwrap / napi_remove_wrap, the optional result
 * reference and the wrap finalizers (on collection and at env teardown). */
#include "napi.h"
#include <stdio.h>
#include <stdlib.h>

#define MAXID 64
static napi_ref g_ref[MAXID];
static bool g_del_ref[MAXID];
static double g_log[MAXID * 4];
static size_t g_log_n;
static double g_counters_freed;

static napi_value undef(napi_env env) {
    napi_value v;
    napi_get_undefined(env, &v);
    return v;
}
static napi_value num(napi_env env, double d) {
    napi_value v;
    napi_create_double(env, d, &v);
    return v;
}
static napi_value list(napi_env env, size_t n, const double *items) {
    napi_value arr;
    napi_create_array_with_length(env, n, &arr);
    for (size_t i = 0; i < n; i++) napi_set_element(env, arr, (uint32_t)i, num(env, items[i]));
    return arr;
}
/* Up to three arguments (missing ones are undefined) and `this`. */
static void args3(napi_env env, napi_callback_info info, napi_value *argv, napi_value *self) {
    size_t argc = 3;
    napi_get_cb_info(env, info, &argc, argv, self, NULL);
}
static double argn(napi_env env, napi_value v) {
    double d = -1;
    napi_get_value_double(env, v, &d);
    return d;
}
static void *ptr(double d) { return (void *)(intptr_t)d; }
static double back(void *p) { return (double)(intptr_t)p; }

/* Counter: the constructor wraps a malloc'd native counter on `this` (its finalizer frees it);
 * increment() unwraps `this` and mutates the native state. */
typedef struct {
    double count;
} Counter;
static void counter_free(napi_env env, void *data, void *hint) {
    (void)env;
    (void)hint;
    free(data);
    g_counters_freed++;
}
static napi_value counter_ctor(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    Counter *c = malloc(sizeof *c);
    c->count = argn(env, argv[0]);
    if (napi_wrap(env, self, c, counter_free, NULL, NULL) != 0) {
        free(c);
        napi_throw_error(env, NULL, "wrap failed");
        return NULL;
    }
    return self;
}
static napi_value counter_increment(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    Counter *c = NULL;
    if (napi_unwrap(env, self, (void **)&c) != 0) {
        napi_throw_error(env, NULL, "not a Counter");
        return NULL;
    }
    c->count += 1;
    return num(env, c->count);
}
static napi_value f_counters_freed(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, g_counters_freed);
}

/* wrap(obj, tag): wrap native pointer `tag`, no finalizer -> status. */
static napi_value f_wrap(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    return num(env, napi_wrap(env, argv[0], ptr(argn(env, argv[1])), NULL, NULL, NULL));
}
/* wrapNull(): a NULL js_object -> [wrap, unwrap, remove_wrap statuses]. */
static napi_value f_wrap_null(napi_env env, napi_callback_info info) {
    (void)info;
    void *r = NULL;
    double st[3] = {napi_wrap(env, NULL, ptr(1), NULL, NULL, NULL), napi_unwrap(env, NULL, &r),
                    napi_remove_wrap(env, NULL, &r)};
    return list(env, 3, st);
}
/* unwrap(obj) / removeWrap(obj) -> [status, native pointer (-1 when left untouched)]. */
static napi_value f_unwrap(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    void *r = ptr(-1);
    double it[2] = {napi_unwrap(env, argv[0], &r), back(r)};
    return list(env, 2, it);
}
static napi_value f_remove_wrap(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    void *r = ptr(-1);
    double it[2] = {napi_remove_wrap(env, argv[0], &r), back(r)};
    return list(env, 2, it);
}
/* unwrapNoResult(obj) / removeWrapNoResult(obj): a NULL result -> status. */
static napi_value f_unwrap_no_result(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    return num(env, napi_unwrap(env, argv[0], NULL));
}
static napi_value f_remove_wrap_no_result(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    return num(env, napi_remove_wrap(env, argv[0], NULL));
}

/* The finalizer of wrapFin: logs [id, hint, an undefined check, the delete status (-1: none)];
 * with mode 2 it deletes the wrap's reference (the ObjectWrap pattern). */
static void fin(napi_env env, void *data, void *hint) {
    int id = (int)(intptr_t)data;
    napi_value u = NULL;
    int ty = -1;
    napi_get_undefined(env, &u);
    napi_typeof(env, u, &ty);
    double del = -1;
    if (g_del_ref[id]) {
        del = napi_delete_reference(env, g_ref[id]);
        g_ref[id] = NULL;
    }
    g_log[g_log_n++] = id;
    g_log[g_log_n++] = back(hint);
    g_log[g_log_n++] = ty;
    g_log[g_log_n++] = del;
}
/* wrapFin(obj, id, mode): wrap native `id` with finalizer `fin` and hint id + 100; mode 1 also
 * takes the result reference, mode 2 takes it and has the finalizer delete it -> status. */
static napi_value f_wrap_fin(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    int id = (int)argn(env, argv[1]), mode = (int)argn(env, argv[2]);
    g_del_ref[id] = mode == 2;
    return num(env, napi_wrap(env, argv[0], ptr(id), fin, ptr(id + 100), mode ? &g_ref[id] : NULL));
}
/* wrapRefNoFin(obj): ask for the result reference without a finalizer -> status. */
static napi_value f_wrap_ref_no_fin(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    napi_ref r = NULL;
    return num(env, napi_wrap(env, argv[0], ptr(1), NULL, NULL, &r));
}
/* refValue(id): the value of wrap id's result reference (undefined once collected). */
static napi_value f_ref_value(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    napi_value v = NULL;
    napi_get_reference_value(env, g_ref[(int)argn(env, argv[0])], &v);
    return v ? v : undef(env);
}
/* finalized(): the finalizer log. */
static napi_value f_finalized(napi_env env, napi_callback_info info) {
    (void)info;
    return list(env, g_log_n, g_log);
}

/* A strong reference taken at module init (to an object holding 7 under "seven"). */
static napi_ref g_strong;

/* At env teardown: reads its own reference and the strong one, makes values, deletes its own
 * reference, tries to run JS and reports to stderr (a value is reported by its typeof). */
static void fin_teardown(napi_env env, void *data, void *hint) {
    int id = (int)(intptr_t)data;
    napi_value own = NULL, strong = NULL, obj = NULL, seven = NULL;
    int own_ty = -1, strong_ty = -1, obj_ty = -1;
    int own_st = napi_get_reference_value(env, g_ref[id], &own);
    if (own) napi_typeof(env, own, &own_ty);
    int strong_st = napi_get_reference_value(env, g_strong, &strong);
    if (strong) napi_typeof(env, strong, &strong_ty);
    int create = napi_create_object(env, &obj);
    if (obj) napi_typeof(env, obj, &obj_ty);
    int get = napi_get_named_property(env, strong, "seven", &seven);
    int del = napi_delete_reference(env, g_ref[id]);
    int call = napi_call_function(env, NULL, NULL, 0, NULL, NULL);
    fprintf(stderr,
            "wrap teardown finalizer: id=%d hint=%d own_ref=%d:%d strong_ref=%d:%d "
            "create_object=%d:%d get_named_property=%d delete_reference=%d call_function=%d\n",
            id, (int)(intptr_t)hint, own_st, own_ty, strong_st, strong_ty, create, obj_ty, get,
            del, call);
}
/* wrapTeardown(obj, id): wrap with fin_teardown, taking the result reference -> status. */
static napi_value f_wrap_teardown(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    int id = (int)argn(env, argv[1]);
    return num(env, napi_wrap(env, argv[0], ptr(id), fin_teardown, ptr(id + 100), &g_ref[id]));
}
/* finalizerTeardown(obj, id): napi_add_finalizer with fin_teardown, taking the result reference
 * -> status. */
static napi_value f_finalizer_teardown(napi_env env, napi_callback_info info) {
    napi_value argv[3], self;
    args3(env, info, argv, &self);
    int id = (int)argn(env, argv[1]);
    return num(env,
               napi_add_finalizer(env, argv[0], ptr(id), fin_teardown, ptr(id + 100), &g_ref[id]));
}

static const struct {
    const char *name;
    napi_callback cb;
} FNS[] = {
    {"countersFreed", f_counters_freed},
    {"wrap", f_wrap},
    {"wrapNull", f_wrap_null},
    {"unwrap", f_unwrap},
    {"removeWrap", f_remove_wrap},
    {"unwrapNoResult", f_unwrap_no_result},
    {"removeWrapNoResult", f_remove_wrap_no_result},
    {"wrapFin", f_wrap_fin},
    {"wrapRefNoFin", f_wrap_ref_no_fin},
    {"refValue", f_ref_value},
    {"finalized", f_finalized},
    {"wrapTeardown", f_wrap_teardown},
    {"finalizerTeardown", f_finalizer_teardown},
};

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    for (size_t i = 0; i < sizeof FNS / sizeof FNS[0]; i++) {
        napi_value f;
        napi_create_function(env, FNS[i].name, NAPI_AUTO_LENGTH, FNS[i].cb, NULL, &f);
        napi_set_named_property(env, exports, FNS[i].name, f);
    }
    /* the method goes on the prototype by hand (napi_create_function: no receiver signature, so
     * calling it on another object reaches napi_unwrap); objectwrap.c defines a class from
     * property descriptors */
    napi_value strong;
    napi_create_object(env, &strong);
    napi_set_named_property(env, strong, "seven", num(env, 7));
    napi_create_reference(env, strong, 1, &g_strong);
    napi_value cls, proto, inc;
    napi_define_class(env, "Counter", NAPI_AUTO_LENGTH, counter_ctor, NULL, 0, NULL, &cls);
    napi_get_named_property(env, cls, "prototype", &proto);
    napi_create_function(env, "increment", NAPI_AUTO_LENGTH, counter_increment, NULL, &inc);
    napi_set_named_property(env, proto, "increment", inc);
    napi_set_named_property(env, exports, "Counter", cls);
    return exports;
}

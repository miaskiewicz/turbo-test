/* napi_ref reference counts (refs.test.ts): napi_create_reference with an initial count,
 * napi_reference_ref / napi_reference_unref moving it up and down (0 is weak), and a wrap's
 * own reference made strong and weak again. A reference travels to JS as a number. */
#include "napi.h"

static int g_finalized;

static napi_value num(napi_env env, double d) {
    napi_value v;
    napi_create_double(env, d, &v);
    return v;
}
static napi_value pair(napi_env env, napi_status st, napi_value v) {
    napi_value arr;
    napi_create_array_with_length(env, 2, &arr);
    napi_set_element(env, arr, 0, num(env, st));
    napi_set_element(env, arr, 1, v);
    return arr;
}
static size_t argv2(napi_env env, napi_callback_info info, napi_value *argv) {
    size_t argc = 2;
    napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
    return argc;
}
static napi_ref ref_arg(napi_env env, napi_value v) {
    double d = 0;
    napi_get_value_double(env, v, &d);
    return (napi_ref)(uintptr_t)d;
}

/* create(value, count) -> [status, ref] */
static napi_value f_create(napi_env env, napi_callback_info info) {
    napi_value argv[2];
    argv2(env, info, argv);
    uint32_t count = 0;
    napi_get_value_uint32(env, argv[1], &count);
    napi_ref r = NULL;
    napi_status st = napi_create_reference(env, argv[0], count, &r);
    return pair(env, st, num(env, (double)(uintptr_t)r));
}

/* ref(r) / unref(r) -> [status, count] */
static napi_value f_ref(napi_env env, napi_callback_info info) {
    napi_value argv[2];
    argv2(env, info, argv);
    uint32_t count = 77;
    napi_status st = napi_reference_ref(env, ref_arg(env, argv[0]), &count);
    return pair(env, st, num(env, count));
}
static napi_value f_unref(napi_env env, napi_callback_info info) {
    napi_value argv[2];
    argv2(env, info, argv);
    uint32_t count = 77;
    napi_status st = napi_reference_unref(env, ref_arg(env, argv[0]), &count);
    return pair(env, st, num(env, count));
}

/* get(r) -> [status, value] (the string "NULL" when the reference has no value) */
static napi_value f_get(napi_env env, napi_callback_info info) {
    napi_value argv[2];
    argv2(env, info, argv);
    napi_value out = NULL;
    napi_status st = napi_get_reference_value(env, ref_arg(env, argv[0]), &out);
    if (out == NULL) napi_create_string_utf8(env, "NULL", NAPI_AUTO_LENGTH, &out);
    return pair(env, st, out);
}

/* del(r) -> status */
static napi_value f_del(napi_env env, napi_callback_info info) {
    napi_value argv[2];
    argv2(env, info, argv);
    return num(env, napi_delete_reference(env, ref_arg(env, argv[0])));
}

static void on_finalize(napi_env env, void *data, void *hint) {
    (void)env;
    (void)data;
    (void)hint;
    g_finalized++;
}

/* wrap(obj) -> [status, the wrap's own reference] */
static napi_value f_wrap(napi_env env, napi_callback_info info) {
    napi_value argv[2];
    argv2(env, info, argv);
    napi_ref r = NULL;
    napi_status st = napi_wrap(env, argv[0], &g_finalized, on_finalize, NULL, &r);
    return pair(env, st, num(env, (double)(uintptr_t)r));
}

static napi_value f_finalized(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, g_finalized);
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    static const struct {
        const char *name;
        napi_callback cb;
    } fns[] = {{"create", f_create}, {"ref", f_ref},   {"unref", f_unref},
               {"get", f_get},       {"del", f_del},   {"wrap", f_wrap},
               {"finalized", f_finalized}};
    for (size_t i = 0; i < sizeof fns / sizeof fns[0]; i++) {
        napi_value fn;
        napi_create_function(env, fns[i].name, NAPI_AUTO_LENGTH, fns[i].cb, NULL, &fn);
        napi_set_named_property(env, exports, fns[i].name, fn);
    }
    return exports;
}

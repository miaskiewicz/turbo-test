/* Type tags (napi_type_tag_object / napi_check_object_type_tag) and napi_add_finalizer: tagging
 * objects to recognise them later, and finalizers attached to any object (on collection, with
 * an optional result reference). */
#include "napi.h"
#include <stdint.h>

#define MAXID 64
static const napi_type_tag TAGS[] = {
    {0x1234, 0x5678}, /* 0 */
    {0x1234, 0x9abc}, /* 1: same lower half as 0 */
    {7, 0},           /* 2: upper half 0 (stored as a one-word BigInt) */
    {0, 0},           /* 3: all zero (stored as 0n) */
};
static napi_ref g_ref[MAXID];
static bool g_del_ref[MAXID];
static double g_log[MAXID * 3];
static size_t g_log_n;

static napi_value num(napi_env env, double d) {
    napi_value v;
    napi_create_double(env, d, &v);
    return v;
}
static napi_value undef(napi_env env) {
    napi_value v;
    napi_get_undefined(env, &v);
    return v;
}
static napi_value list(napi_env env, size_t n, const double *items) {
    napi_value arr;
    napi_create_array_with_length(env, n, &arr);
    for (size_t i = 0; i < n; i++) napi_set_element(env, arr, (uint32_t)i, num(env, items[i]));
    return arr;
}
static void args3(napi_env env, napi_callback_info info, napi_value *argv) {
    size_t argc = 3;
    napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
}
static double argn(napi_env env, napi_value v) {
    double d = -1;
    napi_get_value_double(env, v, &d);
    return d;
}
/* tag index -> the tag (-1: NULL) */
static const napi_type_tag *tag_of(double i) { return i < 0 ? NULL : &TAGS[(int)i]; }

/* tag(obj, i) -> status of napi_type_tag_object */
static napi_value f_tag(napi_env env, napi_callback_info info) {
    napi_value argv[3];
    args3(env, info, argv);
    return num(env, napi_type_tag_object(env, argv[0], tag_of(argn(env, argv[1]))));
}
/* check(obj, i) -> [status, result (-1 when left untouched)] */
static napi_value f_check(napi_env env, napi_callback_info info) {
    napi_value argv[3];
    args3(env, info, argv);
    bool r = false;
    double st = napi_check_object_type_tag(env, argv[0], tag_of(argn(env, argv[1])), &r);
    double res[2] = {st, st == napi_ok ? (double)r : -1};
    return list(env, 2, res);
}
/* nullChecks(obj) -> statuses: tag and check with a NULL object, check with a NULL result, and
 * both with an exception pending. */
static napi_value f_null_checks(napi_env env, napi_callback_info info) {
    napi_value argv[3], exc;
    args3(env, info, argv);
    bool r;
    double st[5];
    st[0] = napi_type_tag_object(env, NULL, &TAGS[0]);
    st[1] = napi_check_object_type_tag(env, NULL, &TAGS[0], &r);
    st[2] = napi_check_object_type_tag(env, argv[0], &TAGS[0], NULL);
    napi_throw_error(env, NULL, "pending");
    st[3] = napi_type_tag_object(env, argv[0], &TAGS[0]);
    st[4] = napi_check_object_type_tag(env, argv[0], &TAGS[0], &r);
    napi_get_and_clear_last_exception(env, &exc);
    return list(env, 5, st);
}

/* The finalizer of addFinalizer: logs [id, hint, typeof undefined]; with mode 2 it deletes its
 * result reference. */
static void fin(napi_env env, void *data, void *hint) {
    int id = (int)(intptr_t)data;
    napi_value u = NULL;
    int ty = -1;
    napi_get_undefined(env, &u);
    napi_typeof(env, u, &ty);
    if (g_del_ref[id]) {
        napi_delete_reference(env, g_ref[id]);
        g_ref[id] = NULL;
    }
    g_log[g_log_n++] = id;
    g_log[g_log_n++] = (double)(intptr_t)hint;
    g_log[g_log_n++] = ty;
}
/* addFinalizer(obj, id, mode): attach `fin` with data id and hint id + 100; mode 1 takes the
 * result reference, mode 2 takes it and has the finalizer delete it -> status. */
static napi_value f_add_finalizer(napi_env env, napi_callback_info info) {
    napi_value argv[3];
    args3(env, info, argv);
    int id = (int)argn(env, argv[1]), mode = (int)argn(env, argv[2]);
    g_del_ref[id] = mode == 2;
    return num(env, napi_add_finalizer(env, argv[0], (void *)(intptr_t)id, fin,
                                       (void *)(intptr_t)(id + 100), mode ? &g_ref[id] : NULL));
}
/* addFinalizerChecks(v) -> statuses: a NULL object, a NULL finalizer, the value `v`, and a
 * valid call with an exception pending (no NAPI_PREAMBLE: it succeeds). */
static napi_value f_add_finalizer_checks(napi_env env, napi_callback_info info) {
    napi_value argv[3], obj, exc;
    args3(env, info, argv);
    napi_create_object(env, &obj);
    double st[4];
    st[0] = napi_add_finalizer(env, NULL, NULL, fin, NULL, NULL);
    st[1] = napi_add_finalizer(env, obj, NULL, NULL, NULL, NULL);
    st[2] = napi_add_finalizer(env, argv[0], NULL, fin, NULL, NULL);
    napi_throw_error(env, NULL, "pending");
    st[3] = napi_add_finalizer(env, obj, (void *)(intptr_t)63, fin, NULL, NULL);
    napi_get_and_clear_last_exception(env, &exc);
    return list(env, 4, st);
}
/* refValue(id): the value of finalizer id's result reference (undefined once collected). */
static napi_value f_ref_value(napi_env env, napi_callback_info info) {
    napi_value argv[3];
    args3(env, info, argv);
    napi_value v = NULL;
    napi_get_reference_value(env, g_ref[(int)argn(env, argv[0])], &v);
    return v ? v : undef(env);
}
/* finalized(): the finalizer log. */
static napi_value f_finalized(napi_env env, napi_callback_info info) {
    (void)info;
    return list(env, g_log_n, g_log);
}

static const struct {
    const char *name;
    napi_callback cb;
} FNS[] = {
    {"tag", f_tag},
    {"check", f_check},
    {"nullChecks", f_null_checks},
    {"addFinalizer", f_add_finalizer},
    {"addFinalizerChecks", f_add_finalizer_checks},
    {"refValue", f_ref_value},
    {"finalized", f_finalized},
};

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    for (size_t i = 0; i < sizeof FNS / sizeof FNS[0]; i++) {
        napi_value f;
        napi_create_function(env, FNS[i].name, NAPI_AUTO_LENGTH, FNS[i].cb, NULL, &f);
        napi_set_named_property(env, exports, FNS[i].name, f);
    }
    return exports;
}

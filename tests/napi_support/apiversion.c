/* An addon declaring its Node-API version (node_api_module_get_api_version_v1, what
 * NAPI_MODULE_INIT emits for NAPI_VERSION), built several ways by tests/napi_host.rs:
 * -DTT_VERSION=<n>. It reports napi_get_version / napi_get_node_version, references values
 * (primitives can be referenced from version 10), and a wrap finalizer at env teardown reports
 * what a call that would run JS returns (napi_cannot_run_js from version 10). */
#include "napi.h"
#include <stdio.h>

#ifndef TT_VERSION
#define TT_VERSION 10
#endif

int32_t node_api_module_get_api_version_v1(void) { return TT_VERSION; }

static napi_value num(napi_env env, double d) {
    napi_value v;
    napi_create_double(env, d, &v);
    return v;
}

/* versions() -> [status, napi version, status, major, minor, patch, release, NULL checks] */
static napi_value f_versions(napi_env env, napi_callback_info info) {
    (void)info;
    uint32_t v = 0;
    const napi_node_version *nv = NULL;
    napi_status s1 = napi_get_version(env, &v), s2 = napi_get_node_version(env, &nv);
    napi_value it[8], release;
    napi_create_string_utf8(env, nv ? nv->release : "none", NAPI_AUTO_LENGTH, &release);
    it[0] = num(env, s1);
    it[1] = num(env, v);
    it[2] = num(env, s2);
    it[3] = num(env, nv ? nv->major : -1);
    it[4] = num(env, nv ? nv->minor : -1);
    it[5] = num(env, nv ? nv->patch : -1);
    it[6] = release;
    it[7] = num(env, napi_get_version(env, NULL) * 100 + napi_get_node_version(env, NULL));
    napi_value arr;
    napi_create_array_with_length(env, 8, &arr);
    for (uint32_t i = 0; i < 8; i++) napi_set_element(env, arr, i, it[i]);
    return arr;
}

/* refValue(v, count) -> [status, the referenced value ("NULL" when it has none)] */
static napi_value f_ref_value(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2], out = NULL;
    napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
    uint32_t count = 0;
    napi_get_value_uint32(env, argv[1], &count);
    napi_ref r = NULL;
    napi_status st = napi_create_reference(env, argv[0], count, &r);
    if (st == napi_ok) napi_get_reference_value(env, r, &out);
    if (out == NULL) napi_create_string_utf8(env, "NULL", NAPI_AUTO_LENGTH, &out);
    napi_value arr;
    napi_create_array_with_length(env, 2, &arr);
    napi_set_element(env, arr, 0, num(env, st));
    napi_set_element(env, arr, 1, out);
    return arr;
}

static napi_value g_fn_holder;
static void at_teardown(napi_env env, void *data, void *hint) {
    (void)data;
    (void)hint;
    napi_value global, undef, out;
    napi_get_global(env, &global);
    napi_get_undefined(env, &undef);
    fprintf(stderr, "apiversion %d teardown: napi_call_function=%d\n", TT_VERSION,
            napi_call_function(env, undef, global, 0, NULL, &out));
    fflush(stderr);
}

/* keepUntilTeardown(obj): wrap obj with a finalizer that reports at env teardown */
static napi_value f_keep(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value obj;
    napi_get_cb_info(env, info, &argc, &obj, NULL, NULL);
    napi_wrap(env, obj, NULL, at_teardown, NULL, NULL);
    (void)g_fn_holder;
    return NULL;
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value f;
    napi_create_function(env, "versions", NAPI_AUTO_LENGTH, f_versions, NULL, &f);
    napi_set_named_property(env, exports, "versions", f);
    napi_create_function(env, "refValue", NAPI_AUTO_LENGTH, f_ref_value, NULL, &f);
    napi_set_named_property(env, exports, "refValue", f);
    napi_create_function(env, "keepUntilTeardown", NAPI_AUTO_LENGTH, f_keep, NULL, &f);
    napi_set_named_property(env, exports, "keepUntilTeardown", f);
    return exports;
}

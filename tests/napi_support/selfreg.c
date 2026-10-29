/* An addon that registers itself with napi_module_register from a static constructor (the
 * NAPI_MODULE macro), exporting no napi_register_module_v1. */
#include "napi.h"

static int g_inits;

static napi_value init(napi_env env, napi_value exports) {
    napi_value v;
    napi_create_string_utf8(env, "self-registered", NAPI_AUTO_LENGTH, &v);
    napi_set_named_property(env, exports, "how", v);
    napi_create_double(env, ++g_inits, &v);
    napi_set_named_property(env, exports, "inits", v);
    return exports;
}

static napi_module mod = {1, 0, __FILE__, init, "selfreg", NULL, {0}};

__attribute__((constructor)) static void register_module(void) { napi_module_register(&mod); }

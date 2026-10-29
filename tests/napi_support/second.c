/* A second addon, loaded next to full.node: its own function table (who, held) and its own
 * references must not be confused with the first addon's. */
#include "napi.h"

static napi_ref g_held;

static napi_value f_who(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value v;
    napi_create_string_utf8(env, "second", NAPI_AUTO_LENGTH, &v);
    return v;
}

/* The `what` of the object this addon referenced during init (reference id 1 of its own env). */
static napi_value f_held(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value o, v;
    napi_get_reference_value(env, g_held, &o);
    napi_get_named_property(env, o, "what", &v);
    return v;
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value f, held, what;
    napi_create_function(env, "who", NAPI_AUTO_LENGTH, f_who, NULL, &f);
    napi_set_named_property(env, exports, "who", f);
    napi_create_function(env, "held", NAPI_AUTO_LENGTH, f_held, NULL, &f);
    napi_set_named_property(env, exports, "held", f);
    napi_create_object(env, &held);
    napi_create_string_utf8(env, "held by second", NAPI_AUTO_LENGTH, &what);
    napi_set_named_property(env, held, "what", what);
    napi_create_reference(env, held, 1, &g_held);
    return exports;
}

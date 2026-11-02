/* Module init that returns its own object instead of the exports it was handed. */
#include "napi.h"
napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value t, custom, kind;
    napi_get_boolean(env, 1, &t);
    napi_set_named_property(env, exports, "onExports", t);
    napi_create_object(env, &custom);
    napi_create_string_utf8(env, "custom", NAPI_AUTO_LENGTH, &kind);
    napi_set_named_property(env, custom, "kind", kind);
    return custom;
}

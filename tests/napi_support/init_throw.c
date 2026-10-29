/* Module init that throws a JS error. */
#include "napi.h"
napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_throw_error(env, NULL, "init exploded");
    return exports;
}

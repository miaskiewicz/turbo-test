/* napi_fatal_error from a native call: the process prints the error and aborts. */
#include "napi.h"

static napi_value f_die(napi_env env, napi_callback_info info) {
    (void)env;
    (void)info;
    napi_fatal_error("tt_fatal_location", NAPI_AUTO_LENGTH, "the message and more", 11);
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value f;
    napi_create_function(env, "die", NAPI_AUTO_LENGTH, f_die, NULL, &f);
    napi_set_named_property(env, exports, "die", f);
    return exports;
}

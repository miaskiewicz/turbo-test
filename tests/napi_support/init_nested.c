/* Module init that runs JS which require()s another addon (globalThis.loadNested, set by the
 * spec) and exports what it got: module init re-entered on the same thread. */
#include "napi.h"

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value global, load, undefined, nested = NULL;
    napi_get_global(env, &global);
    napi_get_named_property(env, global, "loadNested", &load);
    napi_get_undefined(env, &undefined);
    napi_call_function(env, undefined, load, 0, NULL, &nested);
    napi_set_named_property(env, exports, "nested", nested);
    return exports;
}

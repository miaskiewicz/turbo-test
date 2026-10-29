/* An addon importing a Node-API function no host provides: loading it must fail require()
 * with an error naming the import, not crash the run on the first call. */
#include "napi.h"

extern napi_status napi_turbo_test_unknown_entrypoint(napi_env, napi_value *);

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value v;
    napi_turbo_test_unknown_entrypoint(env, &v);
    return v;
}

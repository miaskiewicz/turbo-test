/* Synthetic native (.node) addon for the segfault-hardening test.
 * Its module init calls napi_wrap — an N-API entrypoint turbo-test routes to a
 * catchable JS throw ("not implemented") rather than a null-pointer call. Used
 * to prove a misbehaving native addon can no longer SIGSEGV the whole run.
 * Built at test time (see test/compat-napi.test.mjs); the .node is gitignored. */
#include <stddef.h>
typedef int napi_status;
typedef void* napi_value;
typedef void* napi_env;
typedef void* napi_ref;
extern napi_status napi_create_object(napi_env, napi_value*);
extern napi_status napi_wrap(napi_env, napi_value, void*, void*, void*, napi_ref*);
napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value obj;
    napi_create_object(env, &obj);
    napi_wrap(env, obj, (void*)1, 0, 0, 0);
    return exports;
}

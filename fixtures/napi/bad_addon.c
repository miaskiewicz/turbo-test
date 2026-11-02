/* Synthetic native (.node) addon for the segfault-hardening test.
 * It imports a Node-API function no host provides (napi_turbo_test_unknown_entrypoint) and
 * calls it from its module init. turbo-test binds every import when the addon loads, so the
 * require() fails with a catchable error naming the import instead of calling a null pointer.
 * Used to prove a misbehaving native addon can no longer SIGSEGV the whole run.
 * Built at test time (see test/compat-napi.test.mjs); the .node is gitignored. */
#include <stddef.h>
typedef int napi_status;
typedef void* napi_value;
typedef void* napi_env;
extern napi_status napi_create_object(napi_env, napi_value*);
extern napi_status napi_turbo_test_unknown_entrypoint(napi_env, napi_value*);
napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value out;
    napi_create_object(env, &out);
    napi_turbo_test_unknown_entrypoint(env, &out);
    return exports;
}

/* A NAN / node-gyp addon in miniature: built against Node's own V8 headers, it imports V8's C++
 * API by mangled name (here the symbol libxmljs2's NAN build fails on, turbo-test#391), which
 * turbo-test, embedding its own V8 and exporting only Node-API, does not provide. Loading it must
 * fail require() with an error saying so. Declared in C under its Itanium-mangled name so the
 * fixture needs no C++ compiler or V8 headers. */
#include "napi.h"

extern void _ZN2v811HandleScope16DeleteExtensionsEPNS_7IsolateE(void *);

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    (void)env;
    _ZN2v811HandleScope16DeleteExtensionsEPNS_7IsolateE(0);
    return exports;
}

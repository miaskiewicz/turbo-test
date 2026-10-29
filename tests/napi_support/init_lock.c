/* Module init that records how many threads are inside it at once. Each entry waits up to
 * 500 ms for a second thread to arrive, so two workers that require() it at about the same
 * time overlap reliably unless the host serializes module init. */
#include "napi.h"
#include <stdatomic.h>
#include <unistd.h>

static atomic_int g_inside, g_max;

static napi_value f_max_inside(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value v;
    napi_create_uint32(env, (uint32_t)atomic_load(&g_max), &v);
    return v;
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    atomic_fetch_add(&g_inside, 1);
    for (int i = 0; i < 50 && atomic_load(&g_inside) < 2; i++) usleep(10000);
    int now = atomic_load(&g_inside), seen = atomic_load(&g_max);
    while (now > seen && !atomic_compare_exchange_weak(&g_max, &seen, now)) {
    }
    atomic_fetch_sub(&g_inside, 1);
    napi_value f;
    napi_create_function(env, "maxInside", NAPI_AUTO_LENGTH, f_max_inside, NULL, &f);
    napi_set_named_property(env, exports, "maxInside", f);
    return exports;
}

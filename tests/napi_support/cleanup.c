/* What runs when the addon's env is torn down (cleanup.test.ts; tests/napi_host.rs reads the
 * lines on stderr): env cleanup hooks (one removed again), an async cleanup hook that removes
 * itself, the instance data's finalizer (the data replaced once: the old finalizer never runs),
 * a wrap finalizer, and an external string's finalizer, which runs with a NULL env once the env
 * is gone. The expected order is Node 26's. */
#include "napi.h"
#include <stdio.h>

static void say(const char *what, long n) {
    fprintf(stderr, "cleanup: %s %ld\n", what, n);
    fflush(stderr);
}
static void hook(void *arg) { say("hook", (long)arg); }
static void async_hook(napi_async_cleanup_hook_handle handle, void *arg) {
    say("async hook", (long)arg);
    say("async hook removed", napi_remove_async_cleanup_hook(handle));
}
static void data_finalize(napi_env env, void *data, void *hint) {
    (void)env;
    (void)hint;
    say("instance data finalizer", (long)data);
}
static void wrap_finalize(napi_env env, void *data, void *hint) {
    (void)env;
    (void)hint;
    say("wrap finalizer", (long)data);
}
static void string_finalize(napi_env env, void *data, void *hint) {
    (void)data;
    say("external string finalizer, env is NULL:", env == NULL);
    say("external string finalizer hint", (long)hint);
}

static void tsfn_finalize(napi_env env, void *data, void *context) {
    (void)env;
    (void)context;
    say("threadsafe function finalizer", (long)data);
}

static napi_value num(napi_env env, double d) {
    napi_value v;
    napi_create_double(env, d, &v);
    return v;
}

static napi_threadsafe_function g_tsfn;
static void tsfn_call_js(napi_env env, napi_value js_cb, void *context, void *data) {
    (void)js_cb;
    (void)context;
    say(env ? "threadsafe call" : "threadsafe call with a NULL env", (long)data);
}

/* setUp(obj) -> the statuses of the registrations (the external string is returned through
 * obj.str, the wrap is on obj) */
static napi_value f_set_up(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value obj, s;
    napi_get_cb_info(env, info, &argc, &obj, NULL, NULL);
    static char text[] = "kept until teardown";
    napi_async_cleanup_hook_handle h = NULL;
    double st[] = {
        napi_add_env_cleanup_hook(env, hook, (void *)1),
        napi_add_env_cleanup_hook(env, hook, (void *)2),
        napi_add_async_cleanup_hook(env, async_hook, (void *)3, &h),
        napi_add_env_cleanup_hook(env, hook, (void *)4),
        napi_remove_env_cleanup_hook(env, hook, (void *)2),
        /* removing one that is not registered does nothing */
        napi_remove_env_cleanup_hook(env, hook, (void *)99),
        napi_set_instance_data(env, (void *)5, data_finalize, NULL),
        napi_set_instance_data(env, (void *)6, data_finalize, NULL),
        napi_wrap(env, obj, (void *)7, wrap_finalize, NULL, NULL),
        node_api_create_external_string_latin1(env, text, NAPI_AUTO_LENGTH, string_finalize,
                                               (void *)8, &s, NULL),
        napi_set_named_property(env, obj, "str", s),
        h != NULL,
        /* an open threadsafe function (unref'd, or the process would never exit): closed and
         * finalized at teardown */
        napi_create_threadsafe_function(env, NULL, NULL, s, 0, 1, (void *)9, tsfn_finalize, NULL,
                                        tsfn_call_js, &g_tsfn),
        napi_unref_threadsafe_function(env, g_tsfn),
    };
    napi_value arr;
    napi_create_array_with_length(env, sizeof st / sizeof st[0], &arr);
    for (size_t i = 0; i < sizeof st / sizeof st[0]; i++)
        napi_set_element(env, arr, (uint32_t)i, num(env, st[i]));
    return arr;
}

/* checks() -> statuses: NULL hook functions, a NULL async hook, removing a NULL handle, and a
 * hook added and removed before teardown (it never runs) */
static napi_value f_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_async_cleanup_hook_handle h = NULL;
    double st[] = {
        napi_add_env_cleanup_hook(env, NULL, NULL),
        napi_remove_env_cleanup_hook(env, NULL, NULL),
        napi_add_async_cleanup_hook(env, NULL, NULL, &h),
        napi_remove_async_cleanup_hook(NULL),
        napi_add_async_cleanup_hook(env, async_hook, (void *)100, &h),
        napi_remove_async_cleanup_hook(h),
        napi_add_async_cleanup_hook(env, async_hook, (void *)101, NULL),
    };
    napi_value arr;
    napi_create_array_with_length(env, sizeof st / sizeof st[0], &arr);
    for (size_t i = 0; i < sizeof st / sizeof st[0]; i++)
        napi_set_element(env, arr, (uint32_t)i, num(env, st[i]));
    return arr;
}

/* instanceData() -> the current instance data (as a number) */
static napi_value f_instance_data(napi_env env, napi_callback_info info) {
    (void)info;
    void *data = (void *)-1;
    napi_get_instance_data(env, &data);
    return num(env, (double)(long)data);
}

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    napi_value f;
    void *data = (void *)-1;
    /* no instance data yet: NULL */
    napi_get_instance_data(env, &data);
    say("instance data at init", (long)data);
    napi_create_function(env, "setUp", NAPI_AUTO_LENGTH, f_set_up, NULL, &f);
    napi_set_named_property(env, exports, "setUp", f);
    napi_create_function(env, "checks", NAPI_AUTO_LENGTH, f_checks, NULL, &f);
    napi_set_named_property(env, exports, "checks", f);
    napi_create_function(env, "instanceData", NAPI_AUTO_LENGTH, f_instance_data, NULL, &f);
    napi_set_named_property(env, exports, "instanceData", f);
    return exports;
}

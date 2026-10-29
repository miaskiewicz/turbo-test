/* Async Node-API (async.test.ts): async work on the worker pool (complete on the JS thread,
 * cancelling queued work), threadsafe functions called from other threads (blocking calls on a
 * full queue, release / abort, the default call_js, finalizers), napi_async_init /
 * napi_make_callback, callback scopes and napi_get_uv_event_loop. Each probe hands its
 * statuses back to JS; asynchronous results come through promises. */
#include "napi.h"
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

static napi_value num(napi_env env, double d) {
    napi_value v;
    napi_create_double(env, d, &v);
    return v;
}
static napi_value str(napi_env env, const char *s) {
    napi_value v;
    napi_create_string_utf8(env, s, NAPI_AUTO_LENGTH, &v);
    return v;
}
static napi_value nums(napi_env env, size_t n, const double *items) {
    napi_value arr;
    napi_create_array_with_length(env, n, &arr);
    for (size_t i = 0; i < n; i++) napi_set_element(env, arr, (uint32_t)i, num(env, items[i]));
    return arr;
}
static size_t args(napi_env env, napi_callback_info info, napi_value *argv, size_t n) {
    size_t argc = n;
    napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
    return argc;
}
static double argd(napi_env env, napi_value v) {
    double d = 0;
    napi_get_value_double(env, v, &d);
    return d;
}
static napi_value take_exception(napi_env env) {
    napi_value exc = NULL;
    napi_get_and_clear_last_exception(env, &exc);
    return exc;
}

static pthread_t g_main;

/* ---- async work ------------------------------------------------------------------------ */

typedef struct {
    napi_async_work work;
    napi_deferred deferred;
    double n, result;
    int other_thread;
} Job;

static void job_execute(napi_env env, void *data) {
    (void)env;
    Job *j = data;
    usleep(20000);
    j->result = j->n * 2;
    j->other_thread = !pthread_equal(pthread_self(), g_main);
}
static void job_complete(napi_env env, napi_status status, void *data) {
    Job *j = data;
    double r[3] = {status, j->result, j->other_thread};
    napi_resolve_deferred(env, j->deferred, nums(env, 3, r));
    napi_delete_async_work(env, j->work);
    free(j);
}

/* work(n) -> a promise of [the complete's status, n * 2, whether execute ran on another
 * thread] */
static napi_value f_work(napi_env env, napi_callback_info info) {
    napi_value argv[1], promise, resource;
    args(env, info, argv, 1);
    Job *j = calloc(1, sizeof *j);
    j->n = argd(env, argv[0]);
    napi_create_promise(env, &j->deferred, &promise);
    napi_create_object(env, &resource);
    napi_create_async_work(env, resource, str(env, "tt-work"), job_execute, job_complete, j,
                           &j->work);
    napi_queue_async_work(env, j->work);
    return promise;
}

/* The pool blocked: four works that wait for release(), then a victim to cancel. */
static atomic_int g_started, g_released;
static napi_async_work g_blockers[4], g_victim;
static napi_deferred g_block_deferred;
static double g_block_status[5];
static int g_block_done;

static void blocker_execute(napi_env env, void *data) {
    (void)env;
    (void)data;
    atomic_fetch_add(&g_started, 1);
    while (!atomic_load(&g_released)) usleep(1000);
}
static void blocker_complete(napi_env env, napi_status status, void *data) {
    long i = (long)data;
    g_block_status[i] = status;
    napi_delete_async_work(env, i == 4 ? g_victim : g_blockers[i]);
    if (++g_block_done == 5) napi_resolve_deferred(env, g_block_deferred, nums(env, 5, g_block_status));
}

/* blockPool() -> a promise of the five complete statuses (four blockers, the victim) */
static napi_value f_block_pool(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value promise;
    napi_create_promise(env, &g_block_deferred, &promise);
    for (long i = 0; i < 4; i++) {
        napi_create_async_work(env, NULL, str(env, "blocker"), blocker_execute, blocker_complete,
                               (void *)i, &g_blockers[i]);
        napi_queue_async_work(env, g_blockers[i]);
    }
    return promise;
}
static napi_value f_started(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, atomic_load(&g_started));
}

/* queueAndCancel() -> [cancel the queued victim, cancel a running blocker, that call's last
 * error code and engine error code], then lets the blockers finish */
static napi_value f_queue_and_cancel(napi_env env, napi_callback_info info) {
    (void)info;
    napi_create_async_work(env, NULL, str(env, "victim"), blocker_execute, blocker_complete,
                           (void *)4, &g_victim);
    napi_queue_async_work(env, g_victim);
    double r[4];
    r[0] = napi_cancel_async_work(env, g_victim);
    r[1] = napi_cancel_async_work(env, g_blockers[0]);
    const napi_extended_error_info *ei = NULL;
    napi_get_last_error_info(env, &ei);
    r[2] = ei->error_code;
    r[3] = (int32_t)ei->engine_error_code;
    atomic_store(&g_released, 1);
    return nums(env, 4, r);
}

static void throwing_execute(napi_env env, void *data) {
    (void)env;
    (void)data;
}
static void throwing_complete(napi_env env, napi_status status, void *data) {
    (void)status;
    napi_deferred d = data;
    napi_resolve_deferred(env, d, str(env, "resolved before the throw"));
    napi_throw_error(env, NULL, "thrown by a complete callback");
}
/* workThrows() -> a promise; the complete callback then throws (an uncaught exception) */
static napi_value f_work_throws(napi_env env, napi_callback_info info) {
    (void)info;
    napi_deferred d;
    napi_value promise;
    napi_async_work w;
    napi_create_promise(env, &d, &promise);
    napi_create_async_work(env, NULL, str(env, "throws"), throwing_execute, throwing_complete, d,
                           &w);
    napi_queue_async_work(env, w);
    return promise;
}

/* workChecks() -> statuses: NULL execute / result / name, a non-object resource, NULL work for
 * delete / queue / cancel, and a work with no complete callback */
static napi_value f_work_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_async_work w = NULL;
    napi_value name = str(env, "n");
    double st[] = {
        napi_create_async_work(env, NULL, name, NULL, NULL, NULL, &w),
        napi_create_async_work(env, NULL, name, throwing_execute, NULL, NULL, NULL),
        napi_create_async_work(env, NULL, NULL, throwing_execute, NULL, NULL, &w),
        napi_delete_async_work(env, NULL),
        napi_queue_async_work(env, NULL),
        napi_cancel_async_work(env, NULL),
        napi_create_async_work(env, num(env, 5), name, throwing_execute, NULL, NULL, &w),
        napi_queue_async_work(env, w),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* ---- threadsafe functions ------------------------------------------------------------- */

typedef struct {
    napi_threadsafe_function tsfn;
    napi_deferred done;
    long n;
    double statuses[16];
    int null_env_calls;
} Tsfn;

static void tsfn_call_js(napi_env env, napi_value js_cb, void *context, void *data) {
    Tsfn *t = context;
    if (env == NULL) {
        t->null_env_calls++;
        return;
    }
    napi_value undef, arg = num(env, (double)(long)data);
    napi_get_undefined(env, &undef);
    /* the first call takes a while: the calling thread fills the queue and blocks meanwhile */
    if (data == NULL && t->n > 2) usleep(50000);
    napi_call_function(env, undef, js_cb, 1, &arg, NULL);
}
static void tsfn_finalize(napi_env env, void *finalize_data, void *context) {
    Tsfn *t = context;
    double r[18];
    for (int i = 0; i < 16; i++) r[i] = t->statuses[i];
    r[16] = t->null_env_calls;
    r[17] = finalize_data == (void *)&g_main;
    napi_resolve_deferred(env, t->done, nums(env, 18, r));
    free(t);
}

static void *caller_thread(void *arg) {
    Tsfn *t = arg;
    for (long i = 0; i < t->n; i++)
        t->statuses[i] = napi_call_threadsafe_function(t->tsfn, (void *)i, napi_tsfn_blocking);
    t->statuses[15] = napi_release_threadsafe_function(t->tsfn, napi_tsfn_release);
    return NULL;
}

/* tsfnThreads(fn, n) -> a promise of [the n call statuses (from another thread, blocking on a
 * queue of 2)..., the release status, calls with a NULL env, the finalize data]; fn(i) is
 * called for each i in order */
static napi_value f_tsfn_threads(napi_env env, napi_callback_info info) {
    napi_value argv[2], promise;
    args(env, info, argv, 2);
    Tsfn *t = calloc(1, sizeof *t);
    t->n = (long)argd(env, argv[1]);
    for (int i = 0; i < 16; i++) t->statuses[i] = -1;
    napi_create_promise(env, &t->done, &promise);
    napi_create_threadsafe_function(env, argv[0], NULL, str(env, "tt-tsfn"), 2, 1, &g_main,
                                    tsfn_finalize, t, tsfn_call_js, &t->tsfn);
    pthread_t th;
    pthread_create(&th, NULL, caller_thread, t);
    pthread_detach(th);
    return promise;
}

/* tsfnDefault(fn) -> a promise as tsfnThreads': one call through the default call_js (fn is
 * called with no arguments), then the release */
static napi_value f_tsfn_default(napi_env env, napi_callback_info info) {
    napi_value argv[1], promise;
    args(env, info, argv, 1);
    Tsfn *t = calloc(1, sizeof *t);
    for (int i = 0; i < 16; i++) t->statuses[i] = -1;
    napi_create_promise(env, &t->done, &promise);
    t->statuses[0] = napi_create_threadsafe_function(env, argv[0], NULL, str(env, "default"), 0,
                                                     1, NULL, tsfn_finalize, t, NULL, &t->tsfn);
    t->statuses[1] = napi_call_threadsafe_function(t->tsfn, NULL, napi_tsfn_nonblocking);
    t->statuses[2] = napi_release_threadsafe_function(t->tsfn, napi_tsfn_release);
    return promise;
}

/* tsfnLifecycle() -> a promise as tsfnThreads': a queue of 1 filled from the JS thread (the
 * second nonblocking call is napi_queue_full), acquire, the context, ref / unref, releases down
 * to 0 then one more, an abort; afterwards calls and acquires are napi_closing, and the call
 * left queued reaches call_js with a NULL env */
static napi_value f_tsfn_lifecycle(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value promise;
    Tsfn *t = calloc(1, sizeof *t);
    for (int i = 0; i < 16; i++) t->statuses[i] = -1;
    napi_create_promise(env, &t->done, &promise);
    double *st = t->statuses;
    st[0] = napi_create_threadsafe_function(env, NULL, NULL, str(env, "lifecycle"), 1, 1, NULL,
                                            tsfn_finalize, t, tsfn_call_js, &t->tsfn);
    st[1] = napi_call_threadsafe_function(t->tsfn, (void *)1, napi_tsfn_nonblocking);
    st[2] = napi_call_threadsafe_function(t->tsfn, (void *)2, napi_tsfn_nonblocking);
    st[3] = napi_acquire_threadsafe_function(t->tsfn);
    void *ctx = NULL;
    st[4] = napi_get_threadsafe_function_context(t->tsfn, &ctx);
    st[5] = ctx == t;
    st[6] = napi_unref_threadsafe_function(env, t->tsfn);
    st[7] = napi_ref_threadsafe_function(env, t->tsfn);
    st[8] = napi_release_threadsafe_function(t->tsfn, napi_tsfn_abort);
    st[9] = napi_call_threadsafe_function(t->tsfn, (void *)3, napi_tsfn_nonblocking);
    st[10] = napi_acquire_threadsafe_function(t->tsfn);
    /* a call on a closing threadsafe function lets go of the caller's hold: none is left */
    st[11] = napi_release_threadsafe_function(t->tsfn, napi_tsfn_release);
    st[12] = napi_call_threadsafe_function(t->tsfn, (void *)4, napi_tsfn_nonblocking);
    return promise;
}

/* tsfnRelease() -> statuses: two holders released, then a release with none left (the JS
 * thread has not dispatched in between) */
static napi_value f_tsfn_release(napi_env env, napi_callback_info info) {
    (void)info;
    napi_threadsafe_function tsfn;
    napi_value promise;
    Tsfn *t = calloc(1, sizeof *t);
    napi_create_promise(env, &t->done, &promise);
    double st[4];
    st[0] = napi_create_threadsafe_function(env, NULL, NULL, str(env, "release"), 0, 2, NULL,
                                            tsfn_finalize, t, tsfn_call_js, &tsfn);
    st[1] = napi_release_threadsafe_function(tsfn, napi_tsfn_release);
    st[2] = napi_release_threadsafe_function(tsfn, napi_tsfn_release);
    st[3] = napi_release_threadsafe_function(tsfn, napi_tsfn_release);
    napi_value it[2] = {nums(env, 4, st), promise};
    napi_value arr;
    napi_create_array_with_length(env, 2, &arr);
    napi_set_element(env, arr, 0, it[0]);
    napi_set_element(env, arr, 1, it[1]);
    return arr;
}

/* tsfnHeld() -> [a promise as tsfnThreads', resolved when the threadsafe function is finalized
 * while two threads still hold it]; tsfnHeldLetGo() -> [a call (napi_closing: one holder left),
 * a release (the last: napi_ok, and it is freed)] */
static napi_threadsafe_function g_held;
static napi_value f_tsfn_held(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value promise;
    Tsfn *t = calloc(1, sizeof *t);
    for (int i = 0; i < 16; i++) t->statuses[i] = -1;
    napi_create_promise(env, &t->done, &promise);
    t->statuses[0] = napi_create_threadsafe_function(env, NULL, NULL, str(env, "held"), 0, 3, NULL,
                                                     tsfn_finalize, t, tsfn_call_js, &g_held);
    t->statuses[1] = napi_release_threadsafe_function(g_held, napi_tsfn_abort);
    return promise;
}
static napi_value f_tsfn_held_let_go(napi_env env, napi_callback_info info) {
    (void)info;
    double st[2] = {napi_call_threadsafe_function(g_held, NULL, napi_tsfn_nonblocking),
                    napi_release_threadsafe_function(g_held, napi_tsfn_release)};
    return nums(env, 2, st);
}

/* tsfnDefaultAborted(fn) -> a promise as tsfnThreads': a call through the default call_js left
 * queued by an abort; at finalization it reaches the default call_js with a NULL env, which
 * does not call fn */
static napi_value f_tsfn_default_aborted(napi_env env, napi_callback_info info) {
    napi_value argv[1], promise;
    args(env, info, argv, 1);
    Tsfn *t = calloc(1, sizeof *t);
    for (int i = 0; i < 16; i++) t->statuses[i] = -1;
    napi_create_promise(env, &t->done, &promise);
    t->statuses[0] = napi_create_threadsafe_function(env, argv[0], NULL, str(env, "aborted"), 0,
                                                     1, NULL, tsfn_finalize, t, NULL, &t->tsfn);
    t->statuses[1] = napi_call_threadsafe_function(t->tsfn, NULL, napi_tsfn_nonblocking);
    t->statuses[2] = napi_release_threadsafe_function(t->tsfn, napi_tsfn_abort);
    return promise;
}

/* host-only: tsfnHeldLate() -> as tsfnHeld, the holders let go only after the finalization
 * has completed (in Node the threadsafe function is freed then, and these calls are a use after
 * free); tsfnLateLetGo(release) -> [a call (napi_closing: one holder left), then the last
 * holder's call (napi_closing) or release (napi_ok); the last one frees it] */
static napi_threadsafe_function g_late;
static napi_value f_tsfn_held_late(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value promise;
    Tsfn *t = calloc(1, sizeof *t);
    for (int i = 0; i < 16; i++) t->statuses[i] = -1;
    napi_create_promise(env, &t->done, &promise);
    t->statuses[0] = napi_create_threadsafe_function(env, NULL, NULL, str(env, "late"), 0, 3, NULL,
                                                     tsfn_finalize, t, tsfn_call_js, &g_late);
    t->statuses[1] = napi_release_threadsafe_function(g_late, napi_tsfn_abort);
    return promise;
}
static napi_value f_tsfn_late_let_go(napi_env env, napi_callback_info info) {
    napi_value argv[1];
    args(env, info, argv, 1);
    bool release = false;
    napi_get_value_bool(env, argv[0], &release);
    double st[2];
    st[0] = napi_call_threadsafe_function(g_late, NULL, napi_tsfn_nonblocking);
    st[1] = release ? napi_release_threadsafe_function(g_late, napi_tsfn_release)
                    : napi_call_threadsafe_function(g_late, NULL, napi_tsfn_nonblocking);
    return nums(env, 2, st);
}

static void throwing_call_js(napi_env env, napi_value js_cb, void *context, void *data) {
    (void)js_cb;
    (void)data;
    Tsfn *t = context;
    if (env == NULL) return;
    napi_resolve_deferred(env, t->done, str(env, "called"));
    napi_throw_error(env, NULL, "thrown by call_js");
}
/* tsfnThrows() -> a promise resolved by call_js, which then throws (module API version 8: a
 * deprecation warning, the exception dropped) */
static napi_value f_tsfn_throws(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value promise;
    Tsfn *t = calloc(1, sizeof *t);
    napi_create_promise(env, &t->done, &promise);
    napi_create_threadsafe_function(env, NULL, NULL, str(env, "throws"), 0, 1, NULL, NULL, t,
                                    throwing_call_js, &t->tsfn);
    napi_call_threadsafe_function(t->tsfn, NULL, napi_tsfn_nonblocking);
    napi_release_threadsafe_function(t->tsfn, napi_tsfn_release);
    return promise;
}

/* tsfnChecks(fn) -> statuses: no initial thread, a NULL name / result, a function that is not
 * one, neither a function nor a call_js */
static napi_value f_tsfn_checks(napi_env env, napi_callback_info info) {
    napi_value argv[1], name = str(env, "n");
    args(env, info, argv, 1);
    napi_threadsafe_function t;
    double st[] = {
        napi_create_threadsafe_function(env, argv[0], NULL, name, 0, 0, NULL, NULL, NULL, NULL,
                                        &t),
        napi_create_threadsafe_function(env, argv[0], NULL, NULL, 0, 1, NULL, NULL, NULL, NULL,
                                        &t),
        napi_create_threadsafe_function(env, argv[0], NULL, name, 0, 1, NULL, NULL, NULL, NULL,
                                        NULL),
        napi_create_threadsafe_function(env, name, NULL, name, 0, 1, NULL, NULL, NULL, NULL, &t),
        napi_create_threadsafe_function(env, NULL, NULL, name, 0, 1, NULL, NULL, NULL, NULL, &t),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* ---- async contexts, napi_make_callback, callback scopes, the uv loop ----------------- */

/* makeCallback(recv, fn, arg) -> [status, the result, exception] */
static napi_value f_make_callback(napi_env env, napi_callback_info info) {
    napi_value argv[3], out = NULL;
    args(env, info, argv, 3);
    napi_async_context ctx = NULL;
    napi_async_init(env, NULL, str(env, "tt-callback"), &ctx);
    napi_status st = napi_make_callback(env, ctx, argv[0], argv[1], 1, &argv[2], &out);
    napi_value exc = take_exception(env);
    napi_async_destroy(env, ctx);
    napi_value arr, r[3] = {num(env, st), out, exc};
    napi_create_array_with_length(env, 3, &arr);
    for (uint32_t i = 0; i < 3; i++) {
        napi_value v = r[i];
        if (v == NULL) napi_get_undefined(env, &v);
        napi_set_element(env, arr, i, v);
    }
    return arr;
}

/* asyncChecks(fn) -> statuses: napi_async_init NULL name / result, a resource object, a
 * NULL-context napi_make_callback, NULL recv, argc without argv, a NULL result (allowed),
 * napi_async_destroy NULL; callback scopes: open, close, a close too many, NULL result / scope */
static napi_value f_async_checks(napi_env env, napi_callback_info info) {
    napi_value argv[1], name = str(env, "n"), obj, out;
    args(env, info, argv, 1);
    napi_create_object(env, &obj);
    napi_async_context ctx = NULL, ctx2 = NULL;
    napi_callback_scope cs = NULL;
    double st[] = {
        napi_async_init(env, NULL, NULL, &ctx),
        napi_async_init(env, NULL, name, NULL),
        napi_async_init(env, obj, name, &ctx2),
        napi_make_callback(env, NULL, obj, argv[0], 0, NULL, &out),
        napi_make_callback(env, ctx2, NULL, argv[0], 0, NULL, &out),
        napi_make_callback(env, ctx2, obj, argv[0], 1, NULL, &out),
        napi_make_callback(env, ctx2, obj, argv[0], 0, NULL, NULL),
        napi_async_destroy(env, NULL),
        napi_open_callback_scope(env, obj, ctx2, &cs),
        napi_close_callback_scope(env, cs),
        napi_close_callback_scope(env, cs),
        napi_open_callback_scope(env, obj, ctx2, NULL),
        napi_close_callback_scope(env, NULL),
        napi_async_destroy(env, ctx2),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* uvLoop() -> [status, the loop is not NULL, status with a NULL result] */
static napi_value f_uv_loop(napi_env env, napi_callback_info info) {
    (void)info;
    struct uv_loop_s *loop = NULL;
    double r[3];
    r[0] = napi_get_uv_event_loop(env, &loop);
    r[1] = loop != NULL;
    r[2] = napi_get_uv_event_loop(env, NULL);
    return nums(env, 3, r);
}

/* nullHandles() -> statuses of the threadsafe-function calls with a NULL function
 * (host-only: Node aborts) */
static napi_value f_null_handles(napi_env env, napi_callback_info info) {
    (void)info;
    void *ctx;
    double st[] = {
        napi_get_threadsafe_function_context(NULL, &ctx),
        napi_call_threadsafe_function(NULL, NULL, napi_tsfn_nonblocking),
        napi_acquire_threadsafe_function(NULL),
        napi_release_threadsafe_function(NULL, napi_tsfn_release),
        napi_ref_threadsafe_function(env, NULL),
        napi_unref_threadsafe_function(env, NULL),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

static const struct {
    const char *name;
    napi_callback cb;
} FNS[] = {
    {"work", f_work},
    {"blockPool", f_block_pool},
    {"started", f_started},
    {"queueAndCancel", f_queue_and_cancel},
    {"workThrows", f_work_throws},
    {"workChecks", f_work_checks},
    {"tsfnThreads", f_tsfn_threads},
    {"tsfnDefault", f_tsfn_default},
    {"tsfnLifecycle", f_tsfn_lifecycle},
    {"tsfnRelease", f_tsfn_release},
    {"tsfnHeld", f_tsfn_held},
    {"tsfnHeldLetGo", f_tsfn_held_let_go},
    {"tsfnDefaultAborted", f_tsfn_default_aborted},
    {"tsfnHeldLate", f_tsfn_held_late},
    {"tsfnLateLetGo", f_tsfn_late_let_go},
    {"tsfnThrows", f_tsfn_throws},
    {"tsfnChecks", f_tsfn_checks},
    {"makeCallback", f_make_callback},
    {"asyncChecks", f_async_checks},
    {"uvLoop", f_uv_loop},
    {"nullHandles", f_null_handles},
};

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    g_main = pthread_self();
    for (size_t i = 0; i < sizeof FNS / sizeof FNS[0]; i++) {
        napi_value f;
        napi_create_function(env, FNS[i].name, NAPI_AUTO_LENGTH, FNS[i].cb, NULL, &f);
        napi_set_named_property(env, exports, FNS[i].name, f);
    }
    return exports;
}

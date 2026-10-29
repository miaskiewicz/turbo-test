/* Exercises the core Node-API entrypoints (api.c, async.c and the others cover the rest). Each export is a
 * thin wrapper around one (or a few) napi_* calls; it hands the results back to JS so
 * the spec (full.test.ts) can assert them, and records the napi_status of its last
 * call in g_status (read back via `status()`). */
#include "napi.h"
#include <stdlib.h>
#include <string.h>

static uint32_t g_status;
#define ST(call) (g_status = (uint32_t)(call))
#define MAXA 8

typedef struct {
    size_t argc;
    napi_value argv[MAXA];
    napi_value self;
    void *data;
} Args;

static napi_value undef(napi_env env) {
    napi_value v;
    napi_get_undefined(env, &v);
    return v;
}
static napi_value dbl(napi_env env, double d) {
    napi_value v;
    napi_create_double(env, d, &v);
    return v;
}
static napi_value boolv(napi_env env, bool b) {
    napi_value v;
    napi_get_boolean(env, b, &v);
    return v;
}
static napi_value strn(napi_env env, const char *s, size_t n) {
    napi_value v;
    napi_create_string_utf8(env, s, n, &v);
    return v;
}
static napi_value or_undef(napi_env env, napi_value v) { return v ? v : undef(env); }

static Args args(napi_env env, napi_callback_info info) {
    Args a;
    a.argc = MAXA;
    napi_get_cb_info(env, info, &a.argc, a.argv, &a.self, &a.data);
    return a;
}
static napi_value arg(napi_env env, Args *a, size_t i) {
    return (i < a->argc && i < MAXA) ? a->argv[i] : undef(env);
}
static double argd(napi_env env, Args *a, size_t i) {
    double d = 0;
    napi_get_value_double(env, arg(env, a, i), &d);
    return d;
}
static napi_value list(napi_env env, size_t n, napi_value *items) {
    napi_value arr;
    napi_create_array_with_length(env, n, &arr);
    for (size_t i = 0; i < n; i++) napi_set_element(env, arr, (uint32_t)i, items[i]);
    return arr;
}
/* malloc'd UTF-8 copy of a JS string (length query, then copy). */
static char *cstr(napi_env env, napi_value v) {
    size_t len = 0;
    napi_get_value_string_utf8(env, v, NULL, 0, &len);
    char *b = malloc(len + 1);
    napi_get_value_string_utf8(env, v, b, len + 1, &len);
    return b;
}

static napi_value f_status(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value v;
    napi_create_uint32(env, g_status, &v);
    return v;
}

static napi_value f_basics(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value it[13];
    int acc = 0;
    acc |= napi_get_undefined(env, &it[0]);
    acc |= napi_get_null(env, &it[1]);
    acc |= napi_get_global(env, &it[2]);
    acc |= napi_get_boolean(env, true, &it[3]);
    acc |= napi_get_boolean(env, false, &it[4]);
    acc |= napi_create_uint32(env, 4294967295u, &it[5]);
    acc |= napi_create_double(env, -1.5, &it[6]);
    acc |= napi_create_string_utf8(env, "h\xc3\xa9llo w\xc3\xb6rld \xe2\x82\xac", NAPI_AUTO_LENGTH,
                                   &it[7]);
    acc |= napi_create_string_utf8(env, "abcdef", 3, &it[8]);
    acc |= napi_create_object(env, &it[9]);
    acc |= napi_create_array_with_length(env, 3, &it[10]);
    acc |= napi_create_string_utf8(env, "", 0, &it[11]);
    it[12] = dbl(env, acc);
    return list(env, 13, it);
}

/* Strings made from invalid UTF-8 (each sequence takes the replacement path), then the OR of
 * the creation statuses. */
static napi_value f_bad_utf8(napi_env env, napi_callback_info info) {
    (void)info;
    static const char *const SRC[] = {"a\xff" "b", "\xc3", "x\xe2\x82", "\xed\xa0\x80" "z",
                                      "\xf0\x9f\x98\x80\xc0\xaf"};
    napi_value it[6];
    int acc = 0;
    for (int i = 0; i < 5; i++)
        acc |= napi_create_string_utf8(env, SRC[i], strlen(SRC[i]), &it[i]);
    it[5] = dbl(env, acc);
    return list(env, 6, it);
}

static napi_value f_typeof(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    int t = -1;
    ST(napi_typeof(env, arg(env, &a, 0), &t));
    return dbl(env, t);
}

static napi_value f_strinfo(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value v = arg(env, &a, 0);
    int acc = 0;
    size_t len = 999;
    acc |= napi_get_value_string_utf8(env, v, NULL, 0, &len);
    char *full = malloc(len + 1);
    size_t copied = 999;
    acc |= napi_get_value_string_utf8(env, v, full, len + 1, &copied);
    char t[4];
    size_t tl = 999;
    acc |= napi_get_value_string_utf8(env, v, t, sizeof t, &tl);
    char u[8];
    acc |= napi_get_value_string_utf8(env, v, u, sizeof u, NULL);
    napi_value it[6] = {dbl(env, (double)len), strn(env, full, copied), strn(env, t, tl),
                        dbl(env, (double)tl), strn(env, u, NAPI_AUTO_LENGTH), dbl(env, acc)};
    free(full);
    return list(env, 6, it);
}

/* strStatus(v): a length query -> [status, length (999 when left untouched)]. */
static napi_value f_str_status(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    size_t len = 999;
    ST(napi_get_value_string_utf8(env, arg(env, &a, 0), NULL, 0, &len));
    napi_value it[2] = {dbl(env, g_status), dbl(env, (double)len)};
    return list(env, 2, it);
}

/* strNoOut(v): NULL buf and NULL result -> the status. */
static napi_value f_str_no_out(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    ST(napi_get_value_string_utf8(env, arg(env, &a, 0), NULL, 0, NULL));
    return dbl(env, g_status);
}

/* strCut(v, n): copy into a 0xaa-filled buffer with bufsize n (n < 16) ->
 * [status, copied, the first n + 1 buffer bytes]: what was written, plus one byte past bufsize. */
static napi_value f_str_cut(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    size_t n = (size_t)argd(env, &a, 1);
    char b[16];
    memset(b, 0xaa, sizeof b);
    size_t r = 99;
    ST(napi_get_value_string_utf8(env, arg(env, &a, 0), b, n, &r));
    napi_value it[18];
    it[0] = dbl(env, g_status);
    it[1] = dbl(env, (double)r);
    for (size_t i = 0; i <= n && i < 16; i++) it[2 + i] = dbl(env, (uint8_t)b[i]);
    return list(env, 2 + (n < 16 ? n + 1 : 16), it);
}

static napi_value f_coerce(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    ST(napi_coerce_to_string(env, arg(env, &a, 0), &out));
    return or_undef(env, out);
}

/* num(v) -> [uint32, double, int64, bool, then the four getters' statuses]; outputs a getter
 * leaves untouched stay 99 (false for the bool). */
static napi_value f_num(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value v = arg(env, &a, 0);
    uint32_t u = 99;
    double d = 99;
    int64_t i = 99;
    bool b = false;
    int su = napi_get_value_uint32(env, v, &u);
    int sd = napi_get_value_double(env, v, &d);
    int si = napi_get_value_int64(env, v, &i);
    int sb = napi_get_value_bool(env, v, &b);
    napi_value it[8] = {dbl(env, u),  dbl(env, d),  dbl(env, (double)i), boolv(env, b),
                        dbl(env, su), dbl(env, sd), dbl(env, si),        dbl(env, sb)};
    return list(env, 8, it);
}

static napi_value f_set_named(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    char *k = cstr(env, arg(env, &a, 1));
    ST(napi_set_named_property(env, arg(env, &a, 0), k, arg(env, &a, 2)));
    free(k);
    return dbl(env, g_status);
}

static napi_value f_get_named(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    char *k = cstr(env, arg(env, &a, 1));
    napi_value out = NULL;
    ST(napi_get_named_property(env, arg(env, &a, 0), k, &out));
    free(k);
    return or_undef(env, out);
}

static napi_value f_set_el(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    uint32_t i = 0;
    napi_get_value_uint32(env, arg(env, &a, 1), &i);
    ST(napi_set_element(env, arg(env, &a, 0), i, arg(env, &a, 2)));
    return dbl(env, g_status);
}

static napi_value f_get_el(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    uint32_t i = 0;
    napi_get_value_uint32(env, arg(env, &a, 1), &i);
    napi_value out = NULL;
    ST(napi_get_element(env, arg(env, &a, 0), i, &out));
    return or_undef(env, out);
}

static napi_value f_is_array(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    bool b = false;
    ST(napi_is_array(env, arg(env, &a, 0), &b));
    return boolv(env, b);
}

static napi_value f_arr_len(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    uint32_t n = 0;
    ST(napi_get_array_length(env, arg(env, &a, 0), &n));
    return g_status == 0 ? dbl(env, n) : undef(env);
}

static napi_value f_keys(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    ST(napi_get_property_names(env, arg(env, &a, 0), &out));
    return or_undef(env, out);
}

/* The callback behind functions made by makeFn: reports [argc, this, data, ...args<=4]. */
static napi_value f_echo(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value it[7];
    size_t n = 0;
    it[n++] = dbl(env, (double)a.argc);
    it[n++] = a.self;
    it[n++] = dbl(env, (double)(intptr_t)a.data);
    for (size_t i = 0; i < a.argc && i < 4; i++) it[n++] = a.argv[i];
    return list(env, n, it);
}

static napi_value f_make_fn(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value f = NULL;
    ST(napi_create_function(env, "echo", NAPI_AUTO_LENGTH, f_echo,
                            (void *)(intptr_t)argd(env, &a, 0), &f));
    return or_undef(env, f);
}

/* names() -> functions created with a NUL-terminated name, an explicit length ("echoXYZ", 4)
 * and a NULL name, then the OR of the statuses. */
static napi_value f_names(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value it[4];
    int acc = napi_create_function(env, "echo", NAPI_AUTO_LENGTH, f_echo, NULL, &it[0]) |
              napi_create_function(env, "echoXYZ", 4, f_echo, NULL, &it[1]) |
              napi_create_function(env, NULL, 0, f_echo, NULL, &it[2]);
    it[3] = dbl(env, acc);
    return list(env, 4, it);
}

static napi_value f_argc_only(napi_env env, napi_callback_info info) {
    size_t argc = 0;
    ST(napi_get_cb_info(env, info, &argc, NULL, NULL, NULL));
    return dbl(env, (double)argc);
}

/* padded(...): napi_get_cb_info into a 4-slot argv -> [argc, slot0..slot3]; slots past the
 * real argc must be filled with undefined. */
static napi_value f_padded(napi_env env, napi_callback_info info) {
    size_t argc = 4;
    napi_value argv[4] = {NULL, NULL, NULL, NULL};
    ST(napi_get_cb_info(env, info, &argc, argv, NULL, NULL));
    napi_value it[5] = {dbl(env, (double)argc), argv[0], argv[1], argv[2], argv[3]};
    return list(env, 5, it);
}

static napi_value f_this_only(napi_env env, napi_callback_info info) {
    napi_value self = NULL;
    void *data = (void *)1;
    ST(napi_get_cb_info(env, info, NULL, NULL, &self, &data));
    return data == NULL ? self : undef(env);
}

static napi_value f_ret_null(napi_env env, napi_callback_info info) {
    (void)env;
    (void)info;
    return NULL;
}

static napi_value f_call(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    size_t n = a.argc > MAXA ? MAXA : a.argc;
    napi_value out = NULL;
    ST(napi_call_function(env, arg(env, &a, 1), arg(env, &a, 0), n > 2 ? n - 2 : 0, a.argv + 2,
                          &out));
    return or_undef(env, out);
}

/* callNullArgv(fn, argc): call fn with argc arguments but a NULL argv -> the status. */
static napi_value f_call_null_argv(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    ST(napi_call_function(env, undef(env), arg(env, &a, 0), (size_t)argd(env, &a, 1), NULL, &out));
    return dbl(env, g_status);
}

static napi_value f_call_null_recv(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    size_t n = a.argc > MAXA ? MAXA : a.argc;
    napi_value out = NULL;
    ST(napi_call_function(env, NULL, arg(env, &a, 0), n > 1 ? n - 1 : 0, a.argv + 1, &out));
    return or_undef(env, out);
}

static napi_value f_ab(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    size_t n = (size_t)argd(env, &a, 0);
    void *data = NULL;
    napi_value ab;
    ST(napi_create_arraybuffer(env, n, &data, &ab));
    for (size_t i = 0; i < n; i++) ((uint8_t *)data)[i] = (uint8_t)(i + 1);
    return ab;
}

static napi_value f_ab_no_data(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value ab;
    ST(napi_create_arraybuffer(env, (size_t)argd(env, &a, 0), NULL, &ab));
    return ab;
}

static napi_value f_ta(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    ST(napi_create_typedarray(env, (int)argd(env, &a, 0), (size_t)argd(env, &a, 1),
                              arg(env, &a, 2), (size_t)argd(env, &a, 3), &out));
    return or_undef(env, out);
}

static char EXT_AB[] = "xyz-external";
static napi_value f_ext_ab(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out;
    ST(napi_create_external_arraybuffer(env, EXT_AB, (size_t)argd(env, &a, 0), NULL, NULL, &out));
    return out;
}

static napi_value f_buf(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    size_t n = (size_t)argd(env, &a, 0);
    void *data = NULL;
    napi_value out;
    ST(napi_create_buffer(env, n, &data, &out));
    for (size_t i = 0; i < n; i++) ((uint8_t *)data)[i] = (uint8_t)(i * 2);
    return out;
}

static napi_value f_buf_no_data(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out;
    ST(napi_create_buffer(env, (size_t)argd(env, &a, 0), NULL, &out));
    return out;
}

/* bufCopy(str, len): copy `len` bytes of str, then poke 'Z' through result_data. */
static napi_value f_buf_copy(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    char *src = cstr(env, arg(env, &a, 0));
    size_t n = (size_t)argd(env, &a, 1);
    void *rd = (void *)1;
    napi_value out;
    ST(napi_create_buffer_copy(env, n, src, &rd, &out));
    free(src);
    if (rd && n > 0) ((uint8_t *)rd)[0] = 'Z';
    napi_value it[2] = {out, boolv(env, rd != NULL)};
    return list(env, 2, it);
}

static napi_value f_buf_copy_null(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out;
    ST(napi_create_buffer_copy(env, (size_t)argd(env, &a, 0), NULL, NULL, &out));
    return out;
}

static napi_value f_buf_copy_no_res(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    char *src = cstr(env, arg(env, &a, 0));
    napi_value out;
    ST(napi_create_buffer_copy(env, strlen(src), src, NULL, &out));
    free(src);
    return out;
}

static char EXT_BUF[] = "external-buf";
static napi_value f_ext_buf(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out;
    ST(napi_create_external_buffer(env, (size_t)argd(env, &a, 0), EXT_BUF, NULL, NULL, &out));
    return out;
}

static napi_value f_ext_buf_null(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out;
    ST(napi_create_external_buffer(env, (size_t)argd(env, &a, 0), NULL, NULL, NULL, &out));
    return out;
}

/* bufInfo(v) -> [byteLength, firstByte | -1]; also increments the first byte in place. */
static napi_value f_buf_info(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    void *d = (void *)1;
    size_t len = 999;
    ST(napi_get_buffer_info(env, arg(env, &a, 0), &d, &len));
    if (g_status != 0) return undef(env);
    double first = -1;
    if (d && len > 0) {
        first = ((uint8_t *)d)[0];
        ((uint8_t *)d)[0] += 1;
    }
    napi_value it[2] = {dbl(env, (double)len), dbl(env, first)};
    return list(env, 2, it);
}

static napi_value f_buf_info_nulls(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    ST(napi_get_buffer_info(env, arg(env, &a, 0), NULL, NULL));
    return dbl(env, g_status);
}

static napi_value f_new_instance(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    ST(napi_new_instance(env, arg(env, &a, 0), 0, NULL, &out));
    return or_undef(env, out);
}

static napi_value f_throw_val(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    ST(napi_throw(env, arg(env, &a, 0)));
    return NULL;
}

static napi_value f_throw_err(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    char *m = cstr(env, arg(env, &a, 0));
    ST(napi_throw_error(env, NULL, m));
    free(m);
    return NULL;
}

/* throwErrCode(code, msg): napi_throw_error with a code. */
static napi_value f_throw_err_code(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    char *c = cstr(env, arg(env, &a, 0));
    char *m = cstr(env, arg(env, &a, 1));
    ST(napi_throw_error(env, c, m));
    free(c);
    free(m);
    return NULL;
}

/* createErrCode(code, msg): napi_create_error with a code value (any JS value). */
static napi_value f_create_err_code(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    ST(napi_create_error(env, arg(env, &a, 0), arg(env, &a, 1), &out));
    return or_undef(env, out);
}

static napi_value f_create_err(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    ST(napi_create_error(env, NULL, arg(env, &a, 0), &out));
    return or_undef(env, out);
}

static napi_value f_is_error(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    bool b = false;
    ST(napi_is_error(env, arg(env, &a, 0), &b));
    return boolv(env, b);
}

static napi_value f_pending(napi_env env, napi_callback_info info) {
    (void)info;
    bool p = true;
    napi_value ex = NULL;
    int acc = napi_is_exception_pending(env, &p) | napi_get_and_clear_last_exception(env, &ex);
    napi_value it[3] = {boolv(env, p), or_undef(env, ex), dbl(env, acc)};
    return list(env, 3, it);
}

static napi_ref ref_of(napi_env env, Args *a) {
    return (napi_ref)(uintptr_t)argd(env, a, 0);
}

static napi_value f_ref(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_ref r = NULL;
    ST(napi_create_reference(env, arg(env, &a, 0), 1, &r));
    return dbl(env, (double)(uintptr_t)r);
}

static napi_value f_deref(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    ST(napi_get_reference_value(env, ref_of(env, &a), &out));
    return or_undef(env, out);
}

static napi_value f_del_ref(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    ST(napi_delete_reference(env, ref_of(env, &a)));
    return dbl(env, g_status);
}

static napi_value f_ref_unref(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    uint32_t count = 0;
    ST(napi_reference_unref(env, ref_of(env, &a), &count));
    return dbl(env, g_status);
}

static napi_value f_stubs(napi_env env, napi_callback_info info) {
    (void)info;
    void *h = NULL;
    napi_value it[6];
    it[0] = dbl(env, napi_add_env_cleanup_hook(env, NULL, NULL));
    it[1] = dbl(env, napi_stub());
    it[2] = dbl(env, napi_create_threadsafe_function(env, NULL, NULL, NULL, 0, 1, NULL, NULL, NULL,
                                                     NULL, &h));
    it[3] = boolv(env, h != NULL);
    it[4] = dbl(env, napi_call_threadsafe_function(h, NULL, 0));
    it[5] = dbl(env, napi_unref_threadsafe_function(env, h));
    return list(env, 6, it);
}

/* Constructor behind exports.Cls (napi_define_class): this.value = arg0, this.tag = data. */
static napi_value f_cls_ctor(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_set_named_property(env, a.self, "value", arg(env, &a, 0));
    napi_set_named_property(env, a.self, "tag", dbl(env, (double)(intptr_t)a.data));
    return a.self;
}

/* Strings / property keys / function names / error messages and codes longer than V8's
 * String::kMaxLength cannot be created. */
static napi_value f_huge(napi_env env, napi_callback_info info) {
    (void)info;
    const size_t n = (size_t)1 << 29;
    char *b = malloc(n + 1);
    if (!b) return undef(env);
    memset(b, 'a', n);
    b[n] = 0;
    napi_value s = NULL, out = NULL, f = NULL, o;
    napi_create_object(env, &o);
    napi_value it[10];
    it[0] = dbl(env, napi_create_string_utf8(env, b, n, &s));
    it[1] = dbl(env, napi_set_named_property(env, o, b, o));
    it[2] = dbl(env, napi_get_named_property(env, o, b, &out));
    it[5] = dbl(env, napi_create_function(env, b, n, f_status, NULL, &f));
    /* an error message / code V8 cannot hold: nothing is thrown */
    it[7] = dbl(env, napi_throw_error(env, NULL, b));
    it[8] = dbl(env, napi_throw_error(env, b, "m"));
    bool pending = true;
    napi_is_exception_pending(env, &pending);
    it[9] = boolv(env, pending);
    free(b);
    it[3] = boolv(env, s == NULL);
    it[4] = boolv(env, out == NULL);
    it[6] = boolv(env, f == NULL);
    return list(env, 10, it);
}

/* throwPending(msg): napi_throw_error, then read and clear the pending exception ->
 * [throw status, pending before, the exception, pending after, OR of the query statuses].
 * The exception is cleared, so the call returns normally. */
static napi_value f_throw_pending(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    char *m = cstr(env, arg(env, &a, 0));
    int st = napi_throw_error(env, NULL, m);
    free(m);
    bool before = false, after = true;
    napi_value ex = NULL;
    int acc = napi_is_exception_pending(env, &before) |
              napi_get_and_clear_last_exception(env, &ex) | napi_is_exception_pending(env, &after);
    napi_value it[5] = {dbl(env, st), boolv(env, before), or_undef(env, ex), boolv(env, after),
                        dbl(env, acc)};
    return list(env, 5, it);
}

/* callPending(fn): call fn, then read and clear any pending exception ->
 * [call status, pending, the exception, result left NULL, OR of the query statuses]. */
static napi_value f_call_pending(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    int st = napi_call_function(env, undef(env), arg(env, &a, 0), 0, NULL, &out);
    bool p = false;
    napi_value ex = NULL;
    int acc = napi_is_exception_pending(env, &p) | napi_get_and_clear_last_exception(env, &ex);
    napi_value it[5] = {dbl(env, st), boolv(env, p), or_undef(env, ex), boolv(env, out == NULL),
                        dbl(env, acc)};
    return list(env, 5, it);
}

/* afterThrow(fn, Ctor): with an exception pending, every napi call that can run JS returns
 * napi_pending_exception without running (Node's NAPI_PREAMBLE); the others still work.
 * Returns the statuses in call order, then the (first) exception, cleared at the end. */
static napi_value f_after_throw(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value fn = arg(env, &a, 0), ctor = arg(env, &a, 1), obj, s, n, t, ab, buf, out;
    napi_create_object(env, &obj);
    napi_create_string_utf8(env, "str", NAPI_AUTO_LENGTH, &s);
    napi_create_double(env, 2, &n);
    napi_get_boolean(env, true, &t);
    napi_create_arraybuffer(env, 8, NULL, &ab);
    napi_create_buffer(env, 2, NULL, &buf);
    napi_ref r = NULL;
    napi_create_reference(env, obj, 1, &r);
    napi_throw_error(env, NULL, "first");
    int st[48];
    int k = 0;
    double d;
    bool b;
    uint32_t u;
    int64_t i64;
    size_t len;
    char sb[8];
    int ty;
    napi_ref r2;
    void *data;
    size_t argc = 0;
    /* run JS (or may): napi_pending_exception */
    st[k++] = napi_throw_error(env, NULL, "second");
    st[k++] = napi_throw(env, obj);
    st[k++] = napi_call_function(env, obj, fn, 0, NULL, &out);
    st[k++] = napi_get_named_property(env, obj, "k", &out);
    st[k++] = napi_set_named_property(env, obj, "k", s);
    st[k++] = napi_get_element(env, obj, 0, &out);
    st[k++] = napi_set_element(env, obj, 0, s);
    st[k++] = napi_get_property_names(env, obj, &out);
    st[k++] = napi_coerce_to_string(env, obj, &out);
    st[k++] = napi_create_function(env, "f", NAPI_AUTO_LENGTH, f_status, NULL, &out);
    st[k++] = napi_define_class(env, "C", NAPI_AUTO_LENGTH, f_status, NULL, 0, NULL, &out);
    st[k++] = napi_get_array_length(env, obj, &u);
    st[k++] = napi_create_arraybuffer(env, 1, NULL, &out);
    st[k++] = napi_create_external_arraybuffer(env, sb, 1, NULL, NULL, &out);
    st[k++] = napi_create_typedarray(env, 1, 1, ab, 0, &out);
    st[k++] = napi_create_buffer(env, 1, &data, &out);
    st[k++] = napi_create_buffer_copy(env, 1, "x", &data, &out);
    st[k++] = napi_create_external_buffer(env, 1, sb, NULL, NULL, &out);
    st[k++] = napi_new_instance(env, ctor, 0, NULL, &out);
    st[k++] = napi_wrap(env, obj, (void *)1, NULL, NULL, NULL);
    st[k++] = napi_unwrap(env, obj, &data);
    st[k++] = napi_remove_wrap(env, obj, &data);
    /* no JS: napi_ok */
    st[k++] = napi_get_undefined(env, &out);
    st[k++] = napi_get_null(env, &out);
    st[k++] = napi_get_global(env, &out);
    st[k++] = napi_get_boolean(env, true, &out);
    st[k++] = napi_create_object(env, &out);
    st[k++] = napi_create_uint32(env, 1, &out);
    st[k++] = napi_create_double(env, 1, &out);
    st[k++] = napi_create_string_utf8(env, "x", 1, &out);
    st[k++] = napi_create_error(env, NULL, s, &out);
    st[k++] = napi_typeof(env, obj, &ty);
    st[k++] = napi_is_array(env, obj, &b);
    st[k++] = napi_is_error(env, obj, &b);
    st[k++] = napi_get_value_double(env, n, &d);
    st[k++] = napi_get_value_int64(env, n, &i64);
    st[k++] = napi_get_value_uint32(env, n, &u);
    st[k++] = napi_get_value_bool(env, t, &b);
    st[k++] = napi_get_value_string_utf8(env, s, sb, sizeof sb, &len);
    st[k++] = napi_get_buffer_info(env, buf, &data, &len);
    st[k++] = napi_create_reference(env, obj, 1, &r2);
    st[k++] = napi_get_reference_value(env, r, &out);
    st[k++] = napi_reference_unref(env, r, &u);
    st[k++] = napi_delete_reference(env, r);
    st[k++] = napi_get_cb_info(env, info, &argc, NULL, NULL, NULL);
    napi_value ex = NULL;
    napi_get_and_clear_last_exception(env, &ex);
    napi_value it[49];
    for (int j = 0; j < k; j++) it[j] = dbl(env, st[j]);
    it[k] = or_undef(env, ex);
    return list(env, (size_t)k + 1, it);
}

static const struct {
    const char *name;
    napi_callback cb;
} FNS[] = {
    {"status", f_status},
    {"basics", f_basics},
    {"badUtf8", f_bad_utf8},
    {"typeOf", f_typeof},
    {"strInfo", f_strinfo},
    {"strStatus", f_str_status},
    {"strCut", f_str_cut},
    {"strNoOut", f_str_no_out},
    {"coerce", f_coerce},
    {"num", f_num},
    {"setNamed", f_set_named},
    {"getNamed", f_get_named},
    {"setEl", f_set_el},
    {"getEl", f_get_el},
    {"isArray", f_is_array},
    {"arrLen", f_arr_len},
    {"keys", f_keys},
    {"makeFn", f_make_fn},
    {"names", f_names},
    {"argcOnly", f_argc_only},
    {"padded", f_padded},
    {"thisOnly", f_this_only},
    {"retNull", f_ret_null},
    {"call", f_call},
    {"callNullRecv", f_call_null_recv},
    {"callNullArgv", f_call_null_argv},
    {"ab", f_ab},
    {"abNoData", f_ab_no_data},
    {"ta", f_ta},
    {"extAb", f_ext_ab},
    {"buf", f_buf},
    {"bufNoData", f_buf_no_data},
    {"bufCopy", f_buf_copy},
    {"bufCopyNull", f_buf_copy_null},
    {"bufCopyNoRes", f_buf_copy_no_res},
    {"extBuf", f_ext_buf},
    {"extBufNull", f_ext_buf_null},
    {"bufInfo", f_buf_info},
    {"bufInfoNulls", f_buf_info_nulls},
    {"newInstance", f_new_instance},
    {"throwVal", f_throw_val},
    {"throwErr", f_throw_err},
    {"throwErrCode", f_throw_err_code},
    {"createErr", f_create_err},
    {"createErrCode", f_create_err_code},
    {"isError", f_is_error},
    {"pending", f_pending},
    {"throwPending", f_throw_pending},
    {"callPending", f_call_pending},
    {"afterThrow", f_after_throw},
    {"ref", f_ref},
    {"deref", f_deref},
    {"delRef", f_del_ref},
    {"refUnref", f_ref_unref},
    {"stubs", f_stubs},
    {"huge", f_huge},
};

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    for (size_t i = 0; i < sizeof FNS / sizeof FNS[0]; i++) {
        napi_value f;
        napi_create_function(env, FNS[i].name, NAPI_AUTO_LENGTH, FNS[i].cb, NULL, &f);
        napi_set_named_property(env, exports, FNS[i].name, f);
    }
    napi_value cls;
    napi_define_class(env, "Cls", NAPI_AUTO_LENGTH, f_cls_ctor, (void *)7, 0, NULL, &cls);
    napi_set_named_property(env, exports, "Cls", cls);
    /* NULL: the host must hand back the exports object it passed in. */
    return NULL;
}

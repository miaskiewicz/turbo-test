/* The Node-API surface beyond full.c, one section per family (api.test.ts asserts each against
 * the values Node 26 produces with the same fixture). Every probe hands its napi statuses and
 * results back to JS in an array. */
#include "napi.h"
#include <stdlib.h>
#include <string.h>

#define MAXA 8

typedef struct {
    size_t argc;
    napi_value argv[MAXA];
    napi_value self;
} Args;

static napi_value undef(napi_env env) {
    napi_value v;
    napi_get_undefined(env, &v);
    return v;
}
static napi_value num(napi_env env, double d) {
    napi_value v;
    napi_create_double(env, d, &v);
    return v;
}
static napi_value str(napi_env env, const char *s) {
    napi_value v;
    if (s == NULL) return undef(env);
    napi_create_string_utf8(env, s, NAPI_AUTO_LENGTH, &v);
    return v;
}
static napi_value or_undef(napi_env env, napi_value v) { return v ? v : undef(env); }
static Args args(napi_env env, napi_callback_info info) {
    Args a;
    a.argc = MAXA;
    napi_get_cb_info(env, info, &a.argc, a.argv, &a.self, NULL);
    return a;
}
static napi_value arg(napi_env env, Args *a, size_t i) {
    return i < a->argc ? a->argv[i] : undef(env);
}
static double argd(napi_env env, Args *a, size_t i) {
    double d = 0;
    napi_get_value_double(env, arg(env, a, i), &d);
    return d;
}
static bool is_undefined(napi_env env, napi_value v) {
    int t = -1;
    napi_typeof(env, v, &t);
    return t == 0;
}
/* malloc'd UTF-8 copy of a JS string argument, NULL for undefined */
static char *cstr(napi_env env, napi_value v) {
    if (is_undefined(env, v)) return NULL;
    size_t len = 0;
    napi_get_value_string_utf8(env, v, NULL, 0, &len);
    char *b = malloc(len + 1);
    napi_get_value_string_utf8(env, v, b, len + 1, &len);
    return b;
}
static napi_value list(napi_env env, size_t n, const napi_value *items) {
    napi_value arr;
    napi_create_array_with_length(env, n, &arr);
    for (size_t i = 0; i < n; i++) napi_set_element(env, arr, (uint32_t)i, items[i]);
    return arr;
}
static napi_value nums(napi_env env, size_t n, const double *items) {
    napi_value arr;
    napi_create_array_with_length(env, n, &arr);
    for (size_t i = 0; i < n; i++) napi_set_element(env, arr, (uint32_t)i, num(env, items[i]));
    return arr;
}
/* The last call's error info, copied before any other napi call (each one resets it). */
typedef struct {
    napi_status status, code;
    const char *message;
    uint32_t engine;
} LastError;
static LastError snap(napi_env env) {
    const napi_extended_error_info *info = NULL;
    LastError e = {napi_get_last_error_info(env, &info), 0, NULL, 0};
    if (e.status == napi_ok) {
        e.code = info->error_code;
        e.message = info->error_message;
        e.engine = info->engine_error_code;
    }
    return e;
}
/* [error_code, error_message (undefined for NULL), engine_error_code] */
static napi_value err_value(napi_env env, LastError e) {
    if (e.status != napi_ok) return num(env, -e.status);
    napi_value it[3] = {num(env, e.code), str(env, e.message), num(env, e.engine)};
    return list(env, 3, it);
}
/* ---- errors --------------------------------------------------------------------------- */

typedef napi_status (*ThrowFn)(napi_env, const char *, const char *);
typedef napi_status (*CreateFn)(napi_env, napi_value, napi_value, napi_value *);
static const ThrowFn THROWS[] = {napi_throw_error, napi_throw_type_error, napi_throw_range_error,
                                 node_api_throw_syntax_error};
static const CreateFn CREATES[] = {napi_create_error, napi_create_type_error,
                                   napi_create_range_error, node_api_create_syntax_error};

/* throwKind(kind, code | undefined, msg): napi_throw_*error; the error reaches the caller */
static napi_value f_throw_kind(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    char *code = cstr(env, arg(env, &a, 1)), *msg = cstr(env, arg(env, &a, 2));
    THROWS[(int)argd(env, &a, 0)](env, code, msg);
    free(code);
    free(msg);
    return NULL;
}

/* createKind(kind, code, msg) -> [status, error]: napi_create_*error (code undefined: NULL) */
static napi_value f_create_kind(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value code = arg(env, &a, 1), out = NULL;
    napi_status st = CREATES[(int)argd(env, &a, 0)](env, is_undefined(env, code) ? NULL : code,
                                                    arg(env, &a, 2), &out);
    napi_value it[2] = {num(env, st), or_undef(env, out)};
    return list(env, 2, it);
}

/* errorChecks() -> statuses: each throw with a NULL message, each create with a NULL message
 * and with a NULL result, then the last error after the last of them */
static napi_value f_error_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value msg = str(env, "m"), out;
    double st[12];
    for (int k = 0; k < 4; k++) {
        st[k] = THROWS[k](env, "CODE", NULL);
        st[4 + k] = CREATES[k](env, NULL, NULL, &out);
        st[8 + k] = CREATES[k](env, NULL, msg, NULL);
    }
    LastError last = snap(env);
    napi_value it[2] = {nums(env, 12, st), err_value(env, last)};
    return list(env, 2, it);
}

/* lastErrorInfo(notANumber) -> the last error after: a failing call, then again (reading it
 * changes nothing), a successful call, a call of napi_get_last_error_info with a NULL result
 * (its status), and the last error after that call */
static napi_value f_last_error_info(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value bad = arg(env, &a, 0), one = num(env, 1);
    double d;
    napi_get_value_double(env, bad, &d);
    LastError e0 = snap(env), e1 = snap(env);
    napi_get_value_double(env, one, &d);
    LastError e2 = snap(env);
    napi_status null_result = napi_get_last_error_info(env, NULL);
    LastError e3 = snap(env);
    napi_value it[5] = {err_value(env, e0), err_value(env, e1), err_value(env, e2),
                        num(env, null_result), err_value(env, e3)};
    return list(env, 5, it);
}

/* fatalException(err) -> status of napi_fatal_exception (err undefined: NULL) */
static napi_value f_fatal_exception(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value err = arg(env, &a, 0);
    return num(env, napi_fatal_exception(env, is_undefined(env, err) ? NULL : err));
}

/* ---- numbers, bigints, dates ----------------------------------------------------------- */

/* numbers() -> [int32s, int64s, statuses of the NULL-result creations] */
static napi_value f_numbers(napi_env env, napi_callback_info info) {
    (void)info;
    static const int32_t I32[] = {0, -1, INT32_MIN, INT32_MAX};
    static const int64_t I64[] = {0, -7, 9007199254740993LL, INT64_MIN, INT64_MAX};
    napi_value a32[4], a64[5];
    for (int i = 0; i < 4; i++) napi_create_int32(env, I32[i], &a32[i]);
    for (int i = 0; i < 5; i++) napi_create_int64(env, I64[i], &a64[i]);
    double st[2] = {napi_create_int32(env, 1, NULL), napi_create_int64(env, 1, NULL)};
    napi_value it[3] = {list(env, 4, a32), list(env, 5, a64), nums(env, 2, st)};
    return list(env, 3, it);
}

/* getInt32(v) -> [status, value (-99 when left alone)] */
static napi_value f_get_int32(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    int32_t n = -99;
    napi_status st = napi_get_value_int32(env, arg(env, &a, 0), &n);
    double r[2] = {st, n};
    return nums(env, 2, r);
}

/* bigints() -> [from int64s, from uint64s, from words, NULL / oversize statuses] */
static napi_value f_bigints(napi_env env, napi_callback_info info) {
    (void)info;
    static const int64_t I[] = {0, -1, INT64_MIN, INT64_MAX};
    static const uint64_t U[] = {0, 1, UINT64_MAX};
    static const uint64_t W[] = {1, 2};
    napi_value ai[4], au[3], aw[4], out;
    for (int i = 0; i < 4; i++) napi_create_bigint_int64(env, I[i], &ai[i]);
    for (int i = 0; i < 3; i++) napi_create_bigint_uint64(env, U[i], &au[i]);
    napi_create_bigint_words(env, 0, 2, W, &aw[0]);
    napi_create_bigint_words(env, 1, 2, W, &aw[1]);
    napi_create_bigint_words(env, 1, 0, W, &aw[2]);
    napi_create_bigint_words(env, 0, 1, W, &aw[3]);
    /* one word past V8's maximum BigInt size: a RangeError, left pending */
    size_t huge = ((size_t)1 << 24) + 1;
    uint64_t *big = calloc(huge, sizeof *big);
    double st[8];
    st[0] = napi_create_bigint_int64(env, 1, NULL);
    st[1] = napi_create_bigint_uint64(env, 1, NULL);
    st[2] = napi_create_bigint_words(env, 0, 1, NULL, &out);
    st[3] = napi_create_bigint_words(env, 0, 1, W, NULL);
    st[4] = napi_create_bigint_words(env, 0, (size_t)INT32_MAX + 1, W, &out);
    st[5] = napi_create_bigint_words(env, 0, huge, big, &out);
    free(big);
    napi_value exc = NULL;
    napi_get_and_clear_last_exception(env, &exc);
    /* with an exception pending, napi_create_bigint_words runs no JS */
    napi_throw_error(env, NULL, "pending");
    st[6] = napi_create_bigint_words(env, 0, 1, W, &out);
    napi_get_and_clear_last_exception(env, &out);
    st[7] = napi_create_bigint_int64(env, 1, &out);
    napi_value it[5] = {list(env, 4, ai), list(env, 3, au), list(env, 4, aw), nums(env, 8, st),
                        or_undef(env, exc)};
    return list(env, 5, it);
}

/* getBigInt64(v) / getBigUint64(v) -> [status, value as a BigInt, lossless] */
static napi_value f_get_bigint64(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    int64_t n = 0;
    bool lossless = false;
    napi_status st = napi_get_value_bigint_int64(env, arg(env, &a, 0), &n, &lossless);
    napi_value v, b;
    napi_create_bigint_int64(env, n, &v);
    napi_get_boolean(env, lossless, &b);
    napi_value it[3] = {num(env, st), v, b};
    return list(env, 3, it);
}
static napi_value f_get_biguint64(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    uint64_t n = 0;
    bool lossless = false;
    napi_status st = napi_get_value_bigint_uint64(env, arg(env, &a, 0), &n, &lossless);
    napi_value v, b;
    napi_create_bigint_uint64(env, n, &v);
    napi_get_boolean(env, lossless, &b);
    napi_value it[3] = {num(env, st), v, b};
    return list(env, 3, it);
}

/* getWords(v, capacity) -> [status, sign, word count, words written (as BigInts)...];
 * capacity -1: a word-count query (NULL sign and words) */
static napi_value f_get_words(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    int cap = (int)argd(env, &a, 1);
    int sign = -1;
    uint64_t words[4] = {0, 0, 0, 0};
    size_t count = cap < 0 ? 0 : (size_t)cap;
    napi_status st = cap < 0 ? napi_get_value_bigint_words(env, arg(env, &a, 0), NULL, &count, NULL)
                             : napi_get_value_bigint_words(env, arg(env, &a, 0), &sign, &count,
                                                           words);
    napi_value it[7];
    it[0] = num(env, st);
    it[1] = num(env, sign);
    it[2] = num(env, (double)count);
    for (int i = 0; i < 4; i++) napi_create_bigint_uint64(env, words[i], &it[3 + i]);
    return list(env, 7, it);
}

/* bigintChecks(big) -> statuses: NULL value / result / lossless / word count, a sign without
 * words, words without a sign, a non-BigInt for each getter */
static napi_value f_bigint_checks(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value big = arg(env, &a, 0), n = num(env, 1);
    int64_t i;
    uint64_t u, w[1];
    bool l;
    int sign;
    size_t count = 1;
    double st[] = {
        napi_get_value_bigint_int64(env, NULL, &i, &l),
        napi_get_value_bigint_int64(env, big, NULL, &l),
        napi_get_value_bigint_int64(env, big, &i, NULL),
        napi_get_value_bigint_uint64(env, NULL, &u, &l),
        napi_get_value_bigint_uint64(env, big, &u, NULL),
        napi_get_value_bigint_words(env, NULL, &sign, &count, w),
        napi_get_value_bigint_words(env, big, &sign, NULL, w),
        napi_get_value_bigint_words(env, big, &sign, &count, NULL),
        napi_get_value_bigint_words(env, big, NULL, &count, w),
        napi_get_value_bigint_int64(env, n, &i, &l),
        napi_get_value_bigint_uint64(env, n, &u, &l),
        napi_get_value_bigint_words(env, n, &sign, &count, w),
        napi_get_value_bigint_words(env, n, NULL, NULL, NULL),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* dates(time, notADate) -> [the date, isDate(date), isDate(notADate), value, statuses] */
static napi_value f_dates(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value date = NULL, other = arg(env, &a, 1);
    napi_status created = napi_create_date(env, argd(env, &a, 0), &date);
    bool is_date = false, other_is = true, b;
    double value = -1, d;
    napi_is_date(env, date, &is_date);
    napi_is_date(env, other, &other_is);
    napi_status got = napi_get_date_value(env, date, &value);
    double st[] = {
        created,
        got,
        napi_get_date_value(env, other, &d),
        napi_get_date_value(env, NULL, &d),
        napi_get_date_value(env, date, NULL),
        napi_is_date(env, NULL, &b),
        napi_is_date(env, date, NULL),
        napi_create_date(env, 0, NULL),
    };
    napi_value bt, bf;
    napi_get_boolean(env, is_date, &bt);
    napi_get_boolean(env, other_is, &bf);
    napi_value it[5] = {or_undef(env, date), bt, bf, num(env, value),
                        nums(env, sizeof st / sizeof st[0], st)};
    return list(env, 5, it);
}

/* ---- strings, symbols, coercions, arrays --------------------------------------------- */

/* strings() -> [latin1 (auto, cut), utf16 (auto, cut), property keys (latin1, utf8, utf16),
 * statuses: a NULL string of length 0 / 1, a NULL result, a length over INT_MAX] */
static napi_value f_strings(napi_env env, napi_callback_info info) {
    (void)info;
    static const uint16_t U16[] = {0x68, 0xe9, 0xd83d, 0xde00, 0};
    napi_value it[8], out;
    napi_create_string_latin1(env, "caf\xe9", NAPI_AUTO_LENGTH, &it[0]);
    napi_create_string_latin1(env, "caf\xe9", 3, &it[1]);
    napi_create_string_utf16(env, U16, NAPI_AUTO_LENGTH, &it[2]);
    napi_create_string_utf16(env, U16, 2, &it[3]);
    node_api_create_property_key_latin1(env, "k\xe9y", NAPI_AUTO_LENGTH, &it[4]);
    node_api_create_property_key_utf8(env, "k\xc3\xa9y", NAPI_AUTO_LENGTH, &it[5]);
    node_api_create_property_key_utf16(env, U16, 4, &it[6]);
    double st[] = {
        napi_create_string_latin1(env, NULL, 0, &out),
        napi_create_string_latin1(env, NULL, 1, &out),
        napi_create_string_latin1(env, "x", 1, NULL),
        napi_create_string_latin1(env, "x", (size_t)INT32_MAX + 1, &out),
        napi_create_string_utf16(env, NULL, NAPI_AUTO_LENGTH, &out),
        napi_create_string_utf16(env, NULL, 0, &out),
        napi_create_string_utf16(env, U16, 1, NULL),
        node_api_create_property_key_latin1(env, NULL, 2, &out),
        node_api_create_property_key_utf8(env, "x", 1, NULL),
        node_api_create_property_key_utf16(env, U16, (size_t)INT32_MAX + 1, &out),
        napi_create_string_utf8(env, NULL, 3, &out),
        napi_create_string_utf8(env, "x", 1, NULL),
    };
    it[7] = nums(env, sizeof st / sizeof st[0], st);
    return list(env, 8, it);
}

/* getLatin1(v, bufsize) / getUtf16(v, bufsize) -> [status, the copy, *result];
 * bufsize -1: a length query (NULL buffer) */
static napi_value f_get_latin1(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    int bufsize = (int)argd(env, &a, 1);
    char buf[16];
    memset(buf, 'Z', sizeof buf);
    size_t res = 999;
    napi_status st = napi_get_value_string_latin1(env, arg(env, &a, 0), bufsize < 0 ? NULL : buf,
                                                  bufsize < 0 ? 0 : (size_t)bufsize, &res);
    napi_value it[3], copy;
    napi_create_string_latin1(env, buf, bufsize > 0 ? res + 1 : 0, &copy);
    it[0] = num(env, st);
    it[1] = copy;
    it[2] = num(env, (double)res);
    return list(env, 3, it);
}
static napi_value f_get_utf16(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    int bufsize = (int)argd(env, &a, 1);
    uint16_t buf[16];
    for (int i = 0; i < 16; i++) buf[i] = 'Z';
    size_t res = 999;
    napi_status st = napi_get_value_string_utf16(env, arg(env, &a, 0), bufsize < 0 ? NULL : buf,
                                                 bufsize < 0 ? 0 : (size_t)bufsize, &res);
    napi_value it[3], copy;
    napi_create_string_utf16(env, buf, bufsize > 0 ? res + 1 : 0, &copy);
    it[0] = num(env, st);
    it[1] = copy;
    it[2] = num(env, (double)res);
    return list(env, 3, it);
}

/* getStringChecks(notAString) -> statuses: NULL value, a non-string, a NULL buffer and result,
 * and a NULL result with a buffer (allowed), for latin1 then utf16 */
static napi_value f_get_string_checks(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value other = arg(env, &a, 0), s = str(env, "abc");
    char b[4];
    uint16_t w[4];
    size_t r;
    double st[] = {
        napi_get_value_string_latin1(env, NULL, b, 4, &r),
        napi_get_value_string_latin1(env, other, b, 4, &r),
        napi_get_value_string_latin1(env, s, NULL, 0, NULL),
        napi_get_value_string_latin1(env, s, b, 4, NULL),
        napi_get_value_string_utf16(env, NULL, w, 4, &r),
        napi_get_value_string_utf16(env, other, w, 4, &r),
        napi_get_value_string_utf16(env, s, NULL, 0, NULL),
        napi_get_value_string_utf16(env, s, w, 4, NULL),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

static int g_ext_finalized;
static void ext_finalize(napi_env env, void *data, void *hint) {
    (void)env;
    /* the data is the buffer, the hint what was passed */
    if (hint == (void *)&g_ext_finalized) g_ext_finalized++;
    free(data);
}

/* externalStrings() -> [latin1 [status, string, copied], utf16 [...], an external string with
 * no finalizer, statuses of a NULL string / result] */
static napi_value f_external_strings(napi_env env, napi_callback_info info) {
    (void)info;
    char *l1 = malloc(13);
    memcpy(l1, "external \xe9\xe9\xe9", 13);
    uint16_t *u16 = malloc(4 * sizeof *u16);
    u16[0] = 0x20ac;
    u16[1] = 0xd83d;
    u16[2] = 0xde00;
    u16[3] = 0;
    static char fixed[] = "fixed";
    bool c1 = true, c2 = true;
    napi_value s1 = NULL, s2 = NULL, s3 = NULL, out;
    napi_status st1 = node_api_create_external_string_latin1(env, l1, 12, ext_finalize,
                                                             &g_ext_finalized, &s1, &c1);
    napi_status st2 = node_api_create_external_string_utf16(env, u16, NAPI_AUTO_LENGTH,
                                                            ext_finalize, &g_ext_finalized, &s2,
                                                            &c2);
    node_api_create_external_string_latin1(env, fixed, NAPI_AUTO_LENGTH, NULL, NULL, &s3, NULL);
    double st[] = {
        node_api_create_external_string_latin1(env, NULL, 1, NULL, NULL, &out, NULL),
        node_api_create_external_string_utf16(env, u16, 1, NULL, NULL, NULL, NULL),
    };
    napi_value b1, b2, it[4];
    napi_get_boolean(env, c1, &b1);
    napi_get_boolean(env, c2, &b2);
    napi_value r1[3] = {num(env, st1), or_undef(env, s1), b1};
    napi_value r2[3] = {num(env, st2), or_undef(env, s2), b2};
    it[0] = list(env, 3, r1);
    it[1] = list(env, 3, r2);
    it[2] = or_undef(env, s3);
    it[3] = nums(env, 2, st);
    return list(env, 4, it);
}
static napi_value f_ext_finalized(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, g_ext_finalized);
}

/* symbols(desc | undefined) -> [status, symbol, Symbol.for("tt.key"), statuses] */
static napi_value f_symbols(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value d = arg(env, &a, 0), sym = NULL, sym_for = NULL, out;
    napi_status st = napi_create_symbol(env, is_undefined(env, d) ? NULL : d, &sym);
    node_api_symbol_for(env, "tt.key and more", 6, &sym_for);
    double checks[] = {
        napi_create_symbol(env, NULL, NULL),
        node_api_symbol_for(env, NULL, 2, &out),
        node_api_symbol_for(env, "k", 1, NULL),
    };
    napi_value it[4] = {num(env, st), or_undef(env, sym), or_undef(env, sym_for),
                        nums(env, 3, checks)};
    return list(env, 4, it);
}

/* coerce(v) -> [[status, ToBoolean], [status, ToNumber], what ToNumber left pending,
 * [status, ToObject], what ToObject left pending] (each exception cleared) */
static napi_value f_coerce(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value v = arg(env, &a, 0), b = NULL, n = NULL, o = NULL, e1 = NULL, e2 = NULL;
    napi_status sb = napi_coerce_to_bool(env, v, &b);
    napi_status sn = napi_coerce_to_number(env, v, &n);
    napi_get_and_clear_last_exception(env, &e1);
    napi_status so = napi_coerce_to_object(env, v, &o);
    napi_get_and_clear_last_exception(env, &e2);
    napi_value rb[2] = {num(env, sb), or_undef(env, b)};
    napi_value rn[2] = {num(env, sn), or_undef(env, n)};
    napi_value ro[2] = {num(env, so), or_undef(env, o)};
    napi_value it[5] = {list(env, 2, rb), list(env, 2, rn), e1, list(env, 2, ro), e2};
    return list(env, 5, it);
}

/* coerceChecks() -> statuses: NULL value / result for each coercion */
static napi_value f_coerce_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value v = num(env, 1), out;
    double st[] = {
        napi_coerce_to_bool(env, NULL, &out),  napi_coerce_to_bool(env, v, NULL),
        napi_coerce_to_number(env, NULL, &out), napi_coerce_to_number(env, v, NULL),
        napi_coerce_to_object(env, NULL, &out), napi_coerce_to_object(env, v, NULL),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* createArray() -> [the array, status with a NULL result] */
static napi_value f_create_array(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value arr = NULL;
    napi_create_array(env, &arr);
    napi_value it[2] = {or_undef(env, arr), num(env, napi_create_array(env, NULL))};
    return list(env, 2, it);
}

/* ---- properties ----------------------------------------------------------------------- */

static napi_value boolean(napi_env env, bool b) {
    napi_value v;
    napi_get_boolean(env, b, &v);
    return v;
}
/* the exception left pending (cleared: building the results needs calls that run no JS while
 * one is pending), undefined when none */
static napi_value take_exception(napi_env env) {
    napi_value exc = NULL;
    napi_get_and_clear_last_exception(env, &exc);
    return exc;
}

/* props(obj, key, value) -> [set, get, has, hasOwn, hasNamed("named"), delete, has after,
 * exception] (each [status, result]) */
static napi_value f_props(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value o = arg(env, &a, 0), k = arg(env, &a, 1), v = arg(env, &a, 2), got = NULL;
    bool has = false, own = false, named = false, del = false, after = true;
    napi_status st[7] = {
        napi_set_property(env, o, k, v),         napi_get_property(env, o, k, &got),
        napi_has_property(env, o, k, &has),      napi_has_own_property(env, o, k, &own),
        napi_has_named_property(env, o, "named", &named),
        napi_delete_property(env, o, k, &del),   napi_has_property(env, o, k, &after),
    };
    napi_value exc = take_exception(env);
    napi_value results[7] = {undef(env),       or_undef(env, got),   boolean(env, has),
                             boolean(env, own), boolean(env, named), boolean(env, del),
                             boolean(env, after)};
    napi_value it[8];
    for (int i = 0; i < 7; i++) {
        napi_value pair[2] = {num(env, st[i]), results[i]};
        it[i] = list(env, 2, pair);
    }
    it[7] = exc;
    return list(env, 8, it);
}

/* propChecks(obj) -> statuses: NULL key / value / result / object for each property call, a
 * non-name key for napi_has_own_property, a NULL name for napi_has_named_property, a NULL
 * result for napi_delete_property (allowed) */
static napi_value f_prop_checks(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value o = arg(env, &a, 0), k = str(env, "k"), v = num(env, 1), out;
    bool b;
    double st[] = {
        napi_set_property(env, o, NULL, v),      napi_set_property(env, o, k, NULL),
        napi_set_property(env, NULL, k, v),      napi_get_property(env, o, NULL, &out),
        napi_get_property(env, o, k, NULL),      napi_get_property(env, NULL, k, &out),
        napi_has_property(env, o, k, NULL),      napi_has_property(env, o, NULL, &b),
        napi_has_property(env, NULL, k, &b),     napi_delete_property(env, o, NULL, &b),
        napi_delete_property(env, NULL, k, &b),  napi_delete_property(env, o, k, NULL),
        napi_has_own_property(env, o, NULL, &b), napi_has_own_property(env, o, k, NULL),
        napi_has_own_property(env, NULL, k, &b), napi_has_own_property(env, o, v, &b),
        napi_has_named_property(env, o, "k", NULL), napi_has_named_property(env, NULL, "k", &b),
        napi_has_named_property(env, o, NULL, &b),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* elements(obj, index) -> [hasElement, deleteElement, hasElement after, exception] */
static napi_value f_elements(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value o = arg(env, &a, 0);
    uint32_t i = (uint32_t)argd(env, &a, 1);
    bool has = false, del = false, after = true;
    napi_status s1 = napi_has_element(env, o, i, &has);
    napi_status s2 = napi_delete_element(env, o, i, &del);
    napi_status s3 = napi_has_element(env, o, i, &after);
    napi_value exc = take_exception(env);
    napi_value p1[2] = {num(env, s1), boolean(env, has)};
    napi_value p2[2] = {num(env, s2), boolean(env, del)};
    napi_value p3[2] = {num(env, s3), boolean(env, after)};
    napi_value it[4] = {list(env, 2, p1), list(env, 2, p2), list(env, 2, p3), exc};
    return list(env, 4, it);
}

/* elementChecks(obj) -> statuses: NULL result / object, and a NULL result for
 * napi_delete_element (allowed) */
static napi_value f_element_checks(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value o = arg(env, &a, 0);
    bool b;
    double st[] = {napi_has_element(env, o, 0, NULL), napi_has_element(env, NULL, 0, &b),
                   napi_delete_element(env, NULL, 0, &b), napi_delete_element(env, o, 0, NULL)};
    return nums(env, 4, st);
}

/* allNames(obj, mode, filter, conversion) -> [status, names, exception] */
static napi_value f_all_names(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value names = NULL;
    napi_status st = napi_get_all_property_names(env, arg(env, &a, 0), (int)argd(env, &a, 1),
                                                 (int)argd(env, &a, 2), (int)argd(env, &a, 3),
                                                 &names);
    napi_value exc = take_exception(env);
    napi_value it[3] = {num(env, st), or_undef(env, names), exc};
    return list(env, 3, it);
}

/* proto(obj) -> [status, prototype, exception]; integrity(obj, seal) -> [status, exception] */
static napi_value f_proto(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value p = NULL;
    napi_status st = napi_get_prototype(env, arg(env, &a, 0), &p);
    napi_value exc = take_exception(env);
    napi_value it[3] = {num(env, st), or_undef(env, p), exc};
    return list(env, 3, it);
}
static napi_value f_integrity(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    bool seal = false;
    napi_get_value_bool(env, arg(env, &a, 1), &seal);
    napi_status st = seal ? napi_object_seal(env, arg(env, &a, 0))
                          : napi_object_freeze(env, arg(env, &a, 0));
    napi_value exc = take_exception(env);
    napi_value it[2] = {num(env, st), exc};
    return list(env, 2, it);
}

/* objectChecks() -> statuses: napi_get_all_property_names with a NULL result / object, an
 * unknown mode / conversion; napi_get_prototype NULL result / object; freeze / seal NULL */
static napi_value f_object_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value o, out;
    napi_create_object(env, &o);
    double st[] = {
        napi_get_all_property_names(env, o, 0, 0, 0, NULL),
        napi_get_all_property_names(env, NULL, 0, 0, 0, &out),
        napi_get_all_property_names(env, o, 2, 0, 0, &out),
        napi_get_all_property_names(env, o, 0, 0, 2, &out),
        napi_get_prototype(env, o, NULL),
        napi_get_prototype(env, NULL, &out),
        napi_object_freeze(env, NULL),
        napi_object_seal(env, NULL),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* ---- comparisons and type checks ------------------------------------------------------ */

/* compare(a, b) -> [status, a === b]; instanceOf(v, ctor) -> [status, result, exception] */
static napi_value f_compare(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    bool eq = false;
    napi_status st = napi_strict_equals(env, arg(env, &a, 0), arg(env, &a, 1), &eq);
    napi_value it[2] = {num(env, st), boolean(env, eq)};
    return list(env, 2, it);
}
static napi_value f_instance_of(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    bool r = true;
    napi_status st = napi_instanceof(env, arg(env, &a, 0), arg(env, &a, 1), &r);
    napi_value exc = take_exception(env);
    napi_value it[3] = {num(env, st), boolean(env, r), exc};
    return list(env, 3, it);
}

/* kinds(v) -> [isArrayBuffer, isTypedArray, isDataView, isBuffer, isDetachedArrayBuffer,
 * the OR of the statuses] */
static napi_value f_kinds(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value v = arg(env, &a, 0);
    bool r[5] = {true, true, true, true, true};
    int acc = napi_is_arraybuffer(env, v, &r[0]) | napi_is_typedarray(env, v, &r[1]) |
              napi_is_dataview(env, v, &r[2]) | napi_is_buffer(env, v, &r[3]) |
              napi_is_detached_arraybuffer(env, v, &r[4]);
    napi_value it[6];
    for (int i = 0; i < 5; i++) it[i] = boolean(env, r[i]);
    it[5] = num(env, acc);
    return list(env, 6, it);
}

/* compareChecks() -> statuses: NULL arguments of napi_strict_equals / napi_instanceof and of
 * each type check */
static napi_value f_compare_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value v = num(env, 1), out;
    napi_get_global(env, &out);
    bool b;
    double st[] = {
        napi_strict_equals(env, NULL, v, &b),  napi_strict_equals(env, v, NULL, &b),
        napi_strict_equals(env, v, v, NULL),   napi_instanceof(env, NULL, out, &b),
        napi_instanceof(env, v, out, NULL),    napi_instanceof(env, v, NULL, &b),
        napi_is_arraybuffer(env, NULL, &b),    napi_is_arraybuffer(env, v, NULL),
        napi_is_typedarray(env, NULL, &b),     napi_is_dataview(env, v, NULL),
        napi_is_buffer(env, NULL, &b),         napi_is_detached_arraybuffer(env, v, NULL),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* ---- arraybuffers, typed arrays, dataviews, buffers ------------------------------------- */

/* abInfo(ab) -> [status, byteLength, first byte (bumped in place)] */
static napi_value f_ab_info(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    void *data = NULL;
    size_t len = 999;
    napi_status st = napi_get_arraybuffer_info(env, arg(env, &a, 0), &data, &len);
    double first = -1;
    if (st == napi_ok && len > 0) first = ((uint8_t *)data)[0]++;
    double r[3] = {st, (double)len, first};
    return nums(env, 3, r);
}

/* taInfo(view) -> [status, type, length, byte offset, the buffer, first byte (bumped)] */
static napi_value f_ta_info(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    int type = -1;
    size_t len = 999, off = 999;
    void *data = NULL;
    napi_value ab = NULL;
    napi_status st = napi_get_typedarray_info(env, arg(env, &a, 0), &type, &len, &data, &ab, &off);
    double first = -1;
    if (st == napi_ok && len > 0) first = ((uint8_t *)data)[0]++;
    napi_value it[6] = {num(env, st),        num(env, type), num(env, (double)len),
                        num(env, (double)off), or_undef(env, ab), num(env, first)};
    return list(env, 6, it);
}

/* dataView(ab, byteLength, byteOffset) -> [status, the view, exception];
 * dvInfo(view) -> [status, byteLength, byte offset, the buffer, first byte (bumped)] */
static napi_value f_data_view(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value dv = NULL;
    napi_status st = napi_create_dataview(env, (size_t)argd(env, &a, 1), arg(env, &a, 0),
                                          (size_t)argd(env, &a, 2), &dv);
    napi_value exc = take_exception(env);
    napi_value it[3] = {num(env, st), or_undef(env, dv), exc};
    return list(env, 3, it);
}
static napi_value f_dv_info(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    size_t len = 999, off = 999;
    void *data = NULL;
    napi_value ab = NULL;
    napi_status st = napi_get_dataview_info(env, arg(env, &a, 0), &len, &data, &ab, &off);
    double first = -1;
    if (st == napi_ok && len > 0) first = ((uint8_t *)data)[0]++;
    napi_value it[5] = {num(env, st), num(env, (double)len), num(env, (double)off),
                        or_undef(env, ab), num(env, first)};
    return list(env, 5, it);
}

/* detach(v) -> status */
static napi_value f_detach(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    return num(env, napi_detach_arraybuffer(env, arg(env, &a, 0)));
}

/* bufferFromAb(ab, offset, length) -> [status, the buffer, exception] */
static napi_value f_buffer_from_ab(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value b = NULL;
    napi_status st = node_api_create_buffer_from_arraybuffer(
        env, arg(env, &a, 0), (size_t)argd(env, &a, 1), (size_t)argd(env, &a, 2), &b);
    napi_value exc = take_exception(env);
    napi_value it[3] = {num(env, st), or_undef(env, b), exc};
    return list(env, 3, it);
}

/* viewChecks() -> statuses: NULL / wrong arguments of the info and creation calls */
static napi_value f_view_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value ab, obj, out;
    napi_create_arraybuffer(env, 8, NULL, &ab);
    napi_create_object(env, &obj);
    double st[] = {
        napi_get_arraybuffer_info(env, NULL, NULL, NULL),
        napi_get_arraybuffer_info(env, obj, NULL, NULL),
        napi_get_arraybuffer_info(env, ab, NULL, NULL),
        napi_get_typedarray_info(env, NULL, NULL, NULL, NULL, NULL, NULL),
        napi_get_typedarray_info(env, ab, NULL, NULL, NULL, NULL, NULL),
        napi_create_dataview(env, 1, NULL, 0, &out),
        napi_create_dataview(env, 1, ab, 0, NULL),
        napi_create_dataview(env, 1, obj, 0, &out),
        napi_get_dataview_info(env, NULL, NULL, NULL, NULL, NULL),
        napi_get_dataview_info(env, ab, NULL, NULL, NULL, NULL),
        napi_detach_arraybuffer(env, NULL),
        node_api_create_buffer_from_arraybuffer(env, NULL, 0, 0, &out),
        node_api_create_buffer_from_arraybuffer(env, ab, 0, 0, NULL),
        node_api_create_buffer_from_arraybuffer(env, obj, 0, 0, &out),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

static int g_ext_ab_finalized;
static uint8_t *g_ext_ab_mem;
static void ext_ab_finalize(napi_env env, void *data, void *hint) {
    (void)env;
    if (data == g_ext_ab_mem && hint == (void *)&g_ext_ab_finalized) g_ext_ab_finalized++;
    free(data);
}

/* externalAb(buffer) -> an external ArrayBuffer (buffer false) or Buffer (buffer true) over 4
 * bytes of the addon's memory; extAbPeek() -> [the memory's first byte (0 once finalized),
 * finalized count]; extAbPoke(v) writes the first byte from native code */
static napi_value f_external_ab(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    bool buffer = false;
    napi_get_value_bool(env, arg(env, &a, 0), &buffer);
    g_ext_ab_mem = calloc(4, 1);
    g_ext_ab_mem[0] = 7;
    napi_value out = NULL;
    if (buffer)
        napi_create_external_buffer(env, 4, g_ext_ab_mem, ext_ab_finalize, &g_ext_ab_finalized,
                                    &out);
    else
        napi_create_external_arraybuffer(env, g_ext_ab_mem, 4, ext_ab_finalize,
                                         &g_ext_ab_finalized, &out);
    return out;
}
static napi_value f_ext_ab_poke(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    g_ext_ab_mem[0] = (uint8_t)argd(env, &a, 0);
    return NULL;
}
static napi_value f_ext_ab_finalized(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, g_ext_ab_finalized);
}
/* an external ArrayBuffer with a NULL result: created, napi_ok */
static napi_value f_external_ab_no_result(napi_env env, napi_callback_info info) {
    (void)info;
    static char mem[2];
    return num(env, napi_create_external_arraybuffer(env, mem, 2, NULL, NULL, NULL));
}

/* ---- promises and externals --------------------------------------------------------- */

/* promise(resolve, value) -> [status, the promise, isPromise(promise), conclude status] */
static napi_value f_promise(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    bool resolve = false, is = false;
    napi_get_value_bool(env, arg(env, &a, 0), &resolve);
    napi_deferred d = NULL;
    napi_value p = NULL;
    napi_status st = napi_create_promise(env, &d, &p);
    napi_is_promise(env, p, &is);
    napi_status done = resolve ? napi_resolve_deferred(env, d, arg(env, &a, 1))
                               : napi_reject_deferred(env, d, arg(env, &a, 1));
    napi_value it[4] = {num(env, st), or_undef(env, p), boolean(env, is), num(env, done)};
    return list(env, 4, it);
}

/* a deferred kept for later: deferLater() -> the promise; settleLater(resolve, v) -> status */
static napi_deferred g_later;
static napi_value f_defer_later(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value p;
    napi_create_promise(env, &g_later, &p);
    return p;
}
static napi_value f_settle_later(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    bool resolve = false;
    napi_get_value_bool(env, arg(env, &a, 0), &resolve);
    return num(env, resolve ? napi_resolve_deferred(env, g_later, arg(env, &a, 1))
                            : napi_reject_deferred(env, g_later, arg(env, &a, 1)));
}

/* promiseChecks(notAPromise) -> [isPromise(notAPromise), statuses: NULL deferred / promise
 * out-pointers, NULL resolution value, NULL value / result of napi_is_promise, creation and
 * resolution with an exception pending, then that deferred resolved afterwards] */
static napi_value f_promise_checks(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_deferred d = NULL, d2 = NULL;
    napi_value p, ignored, v = num(env, 1);
    bool is = true, b;
    napi_is_promise(env, arg(env, &a, 0), &is);
    napi_create_promise(env, &d, &p);
    double st[9];
    st[0] = napi_create_promise(env, NULL, &p);
    st[1] = napi_create_promise(env, &d2, NULL);
    st[2] = napi_resolve_deferred(env, d, NULL);
    st[3] = napi_is_promise(env, NULL, &b);
    st[4] = napi_is_promise(env, p, NULL);
    napi_throw_error(env, NULL, "pending");
    st[5] = napi_create_promise(env, &d2, &p);
    st[6] = napi_reject_deferred(env, d, v);
    napi_get_and_clear_last_exception(env, &ignored);
    /* the deferred survived the refused call */
    st[7] = napi_resolve_deferred(env, d, v);
    napi_value it[2] = {boolean(env, is), nums(env, 8, st)};
    return list(env, 2, it);
}

static int g_external_finalized;
static void external_finalize(napi_env env, void *data, void *hint) {
    (void)env;
    if (data == (void *)&g_external_finalized && hint == (void *)7) g_external_finalized++;
}

/* externals() -> [status, the external, typeof, the data matches, a second external (no
 * finalizer), statuses: get the value of a non-external / NULL, NULL result of create / get] */
static napi_value f_externals(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value e = NULL, e2 = NULL, obj, out;
    napi_create_object(env, &obj);
    napi_status st = napi_create_external(env, &g_external_finalized, external_finalize,
                                          (void *)7, &e);
    int t = -1;
    napi_typeof(env, e, &t);
    void *data = NULL, *d;
    napi_get_value_external(env, e, &data);
    napi_create_external(env, (void *)42, NULL, NULL, &e2);
    double checks[] = {
        napi_get_value_external(env, obj, &d),
        napi_get_value_external(env, NULL, &d),
        napi_get_value_external(env, e, NULL),
        napi_create_external(env, NULL, NULL, NULL, NULL),
    };
    (void)out;
    napi_value it[6] = {num(env, st),  or_undef(env, e), num(env, t),
                        boolean(env, data == (void *)&g_external_finalized), or_undef(env, e2),
                        nums(env, 4, checks)};
    return list(env, 6, it);
}
static napi_value f_external_finalized(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, g_external_finalized);
}

/* externalTags(external) -> statuses and results: check before tagging, tag it, tag it again,
 * check the tag, check another tag */
static napi_value f_external_tags(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value e = arg(env, &a, 0);
    static const napi_type_tag T1 = {1, 2}, T2 = {1, 3};
    bool r1 = false, r2 = true, r3 = true;
    double st[] = {
        napi_check_object_type_tag(env, e, &T1, &r3),
        napi_type_tag_object(env, e, &T1),
        napi_type_tag_object(env, e, &T2),
        napi_check_object_type_tag(env, e, &T1, &r1),
        napi_check_object_type_tag(env, e, &T2, &r2),
        r1,
        r2,
        r3,
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

/* externalTagNulls(external) -> statuses of a NULL tag (tagging, checking) and a NULL result
 * for an External (host-only: Node dereferences them) */
static napi_value f_external_tag_nulls(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value e = arg(env, &a, 0);
    static const napi_type_tag T1 = {1, 2};
    bool r;
    double st[] = {
        napi_type_tag_object(env, e, NULL),
        napi_check_object_type_tag(env, e, NULL, &r),
        napi_check_object_type_tag(env, e, &T1, NULL),
    };
    return nums(env, 3, st);
}

/* ---- instances, scripts, handle scopes, memory, module file name ---------------------------- */

/* newInstance(ctor, a, b) -> [status, the instance, exception] */
static napi_value f_new_instance(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value inst = NULL, argv[2] = {arg(env, &a, 1), arg(env, &a, 2)};
    napi_status st = napi_new_instance(env, arg(env, &a, 0), 2, argv, &inst);
    napi_value exc = take_exception(env);
    napi_value it[3] = {num(env, st), or_undef(env, inst), exc};
    return list(env, 3, it);
}

/* newInstanceChecks(ctor) -> statuses: NULL constructor / argv with argc > 0 / result, no
 * arguments (argc 0, NULL argv) */
static napi_value f_new_instance_checks(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value ctor = arg(env, &a, 0), out;
    double st[] = {
        napi_new_instance(env, NULL, 0, NULL, &out),
        napi_new_instance(env, ctor, 1, NULL, &out),
        napi_new_instance(env, ctor, 0, NULL, NULL),
        napi_new_instance(env, ctor, 0, NULL, &out),
    };
    return nums(env, 4, st);
}

/* runScript(source) -> [status, the result, exception] */
static napi_value f_run_script(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value out = NULL;
    napi_status st = napi_run_script(env, arg(env, &a, 0), &out);
    napi_value exc = take_exception(env);
    napi_value it[3] = {num(env, st), or_undef(env, out), exc};
    return list(env, 3, it);
}

/* handleScopes(v) -> [statuses..., the escaped value, last error after the mismatch] */
static napi_value f_handle_scopes(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value v = arg(env, &a, 0), escaped = NULL, again = NULL, inner;
    napi_handle_scope hs = NULL;
    napi_escapable_handle_scope es = NULL;
    double st[12];
    st[0] = napi_open_handle_scope(env, &hs);
    napi_create_object(env, &inner);
    st[1] = napi_close_handle_scope(env, hs);
    st[2] = napi_open_escapable_handle_scope(env, &es);
    st[3] = napi_escape_handle(env, es, v, &escaped);
    st[4] = napi_escape_handle(env, es, v, &again);
    st[5] = napi_close_escapable_handle_scope(env, es);
    /* a failing call, then a close with no scope open: napi_handle_scope_mismatch, which is
     * not recorded (the last error stays the failing call's) */
    double d;
    napi_get_value_double(env, str(env, "x"), &d);
    st[6] = napi_close_handle_scope(env, (napi_handle_scope)&st);
    LastError after = snap(env);
    st[7] = napi_close_escapable_handle_scope(env, (napi_escapable_handle_scope)&st);
    st[8] = napi_open_handle_scope(env, NULL);
    st[9] = napi_close_handle_scope(env, NULL);
    st[10] = napi_open_escapable_handle_scope(env, NULL);
    st[11] = napi_escape_handle(env, NULL, v, &escaped);
    napi_value it[4] = {nums(env, 12, st), or_undef(env, escaped), or_undef(env, again),
                        err_value(env, after)};
    return list(env, 4, it);
}

/* escapeChecks(v) -> statuses of napi_escape_handle with a NULL escapee and result */
static napi_value f_escape_checks(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_escapable_handle_scope es;
    napi_value out;
    napi_open_escapable_handle_scope(env, &es);
    double st[2] = {napi_escape_handle(env, es, NULL, &out),
                    napi_escape_handle(env, es, arg(env, &a, 0), NULL)};
    napi_close_escapable_handle_scope(env, es);
    return nums(env, 2, st);
}

/* externalMemory(change) -> [status, adjusted value >= change, status with a NULL result] */
static napi_value f_external_memory(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    int64_t change = (int64_t)argd(env, &a, 0), adjusted = -1;
    napi_status st = napi_adjust_external_memory(env, change, &adjusted);
    napi_status st0 = napi_adjust_external_memory(env, 0, NULL);
    napi_adjust_external_memory(env, -change, &adjusted);
    double r[3] = {st, adjusted >= 0, st0};
    return nums(env, 3, r);
}

/* moduleFileName() -> [status, the file name, status with a NULL result] */
static napi_value f_module_file_name(napi_env env, napi_callback_info info) {
    (void)info;
    const char *name = NULL;
    napi_status st = node_api_get_module_file_name(env, &name);
    napi_value it[3] = {num(env, st), str(env, name),
                        num(env, node_api_get_module_file_name(env, NULL))};
    return list(env, 3, it);
}

/* ---- NULL arguments of the core entrypoints ------------------------------------------------ */

static napi_value f_noop(napi_env env, napi_callback_info info) {
    (void)info;
    return undef(env);
}

/* coreNulls() -> the statuses of the core entrypoints with a NULL result or value argument */
static napi_value f_core_nulls(napi_env env, napi_callback_info info) {
    napi_value v = num(env, 1), s = str(env, "s"), o, ab, fn, out;
    napi_create_object(env, &o);
    napi_create_arraybuffer(env, 8, NULL, &ab);
    napi_create_function(env, "f", NAPI_AUTO_LENGTH, f_noop, NULL, &fn);
    double d;
    bool b;
    uint32_t u;
    int64_t i64;
    int t;
    size_t len, argc = 1;
    void *data;
    char buf[4];
    napi_value argv[1];
    napi_ref r;
    napi_create_reference(env, o, 1, &r);
    double st[] = {
        napi_get_undefined(env, NULL),
        napi_get_null(env, NULL),
        napi_get_global(env, NULL),
        napi_get_boolean(env, true, NULL),
        napi_typeof(env, NULL, &t),
        napi_typeof(env, v, NULL),
        napi_create_uint32(env, 1, NULL),
        napi_create_double(env, 1, NULL),
        napi_get_value_uint32(env, NULL, &u),
        napi_get_value_uint32(env, v, NULL),
        napi_get_value_string_utf8(env, NULL, buf, 4, &len),
        napi_coerce_to_string(env, NULL, &out),
        napi_coerce_to_string(env, v, NULL),
        napi_create_object(env, NULL),
        napi_create_array_with_length(env, 1, NULL),
        napi_set_named_property(env, NULL, "k", v),
        napi_set_named_property(env, o, NULL, v),
        napi_set_named_property(env, o, "k", NULL),
        napi_get_named_property(env, NULL, "k", &out),
        napi_get_named_property(env, o, NULL, &out),
        napi_get_named_property(env, o, "k", NULL),
        napi_set_element(env, NULL, 0, v),
        napi_set_element(env, o, 0, NULL),
        napi_get_element(env, NULL, 0, &out),
        napi_get_element(env, o, 0, NULL),
        napi_create_function(env, "f", NAPI_AUTO_LENGTH, NULL, NULL, &out),
        napi_create_function(env, "f", NAPI_AUTO_LENGTH, f_noop, NULL, NULL),
        napi_get_cb_info(env, NULL, &argc, argv, NULL, NULL),
        napi_get_cb_info(env, info, NULL, argv, NULL, NULL),
        napi_call_function(env, v, NULL, 0, NULL, &out),
        napi_call_function(env, v, fn, 0, NULL, NULL),
        napi_create_arraybuffer(env, 1, NULL, NULL),
        napi_create_typedarray(env, 1, 1, NULL, 0, &out),
        napi_create_typedarray(env, 1, 1, ab, 0, NULL),
        napi_create_buffer(env, 1, &data, NULL),
        napi_create_buffer_copy(env, 1, "x", &data, NULL),
        napi_create_external_buffer(env, 1, buf, NULL, NULL, NULL),
        napi_get_buffer_info(env, NULL, &data, &len),
        napi_get_value_bool(env, NULL, &b),
        napi_get_value_bool(env, v, NULL),
        napi_get_value_double(env, NULL, &d),
        napi_get_value_double(env, v, NULL),
        napi_get_value_int64(env, NULL, &i64),
        napi_get_value_int64(env, v, NULL),
        napi_is_array(env, NULL, &b),
        napi_is_array(env, v, NULL),
        napi_get_array_length(env, NULL, &u),
        napi_get_array_length(env, v, NULL),
        napi_get_property_names(env, NULL, &out),
        napi_get_property_names(env, o, NULL),
        napi_throw(env, NULL),
        napi_is_error(env, NULL, &b),
        napi_is_error(env, v, NULL),
        napi_is_exception_pending(env, NULL),
        napi_get_and_clear_last_exception(env, NULL),
        napi_create_reference(env, NULL, 1, &r),
        napi_create_reference(env, o, 1, NULL),
        napi_get_reference_value(env, r, NULL),
        napi_get_value_string_utf8(env, s, NULL, 0, NULL),
    };
    napi_delete_reference(env, r);
    return nums(env, sizeof st / sizeof st[0], st);
}

/* typedArrayOver(buffer, type, length, offset) -> [status, the typed array, exception] */
static napi_value f_typed_array_over(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    napi_value ta = NULL;
    napi_status st = napi_create_typedarray(env, (int)argd(env, &a, 1), (size_t)argd(env, &a, 2),
                                            arg(env, &a, 0), (size_t)argd(env, &a, 3), &ta);
    napi_value exc = take_exception(env);
    napi_value it[3] = {num(env, st), or_undef(env, ta), exc};
    return list(env, 3, it);
}

/* buffers() -> [napi_create_buffer, _copy("abc"), _external, node_api_create_buffer_from_arraybuffer] */
static char g_ext_bytes[] = "ext";
static napi_value f_buffers(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value it[4], ab;
    void *data;
    napi_create_buffer(env, 2, &data, &it[0]);
    napi_create_buffer_copy(env, 3, "abc", &data, &it[1]);
    napi_create_external_buffer(env, 3, g_ext_bytes, NULL, NULL, &it[2]);
    napi_create_arraybuffer(env, 4, &data, &ab);
    node_api_create_buffer_from_arraybuffer(env, ab, 1, 2, &it[3]);
    return list(env, 4, it);
}

static void throwing_finalize(napi_env env, void *data, void *hint) {
    (void)data;
    (void)hint;
    napi_throw_error(env, NULL, "thrown by a finalizer");
}
/* throwingFinalizer(obj) -> status: a finalizer that throws */
static napi_value f_throwing_finalizer(napi_env env, napi_callback_info info) {
    Args a = args(env, info);
    return num(env, napi_add_finalizer(env, arg(env, &a, 0), NULL, throwing_finalize, NULL, NULL));
}

/* ---- every entrypoint with a NULL env is napi_invalid_arg ------------------------------- */

static napi_value f_null_env(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value v = num(env, 1), out;
    const napi_extended_error_info *ei;
    double d;
    double st[] = {
        napi_get_undefined(NULL, &out),
        napi_get_value_double(NULL, v, &d),
        napi_coerce_to_string(NULL, v, &out),
        napi_create_error(NULL, NULL, v, &out),
        napi_throw_type_error(NULL, NULL, "m"),
        napi_get_last_error_info(NULL, &ei),
        napi_fatal_exception(NULL, v),
        napi_create_int32(NULL, 1, &out),
        napi_get_value_bigint_words(NULL, v, NULL, NULL, NULL),
        napi_create_bigint_words(NULL, 0, 0, NULL, &out),
        napi_create_date(NULL, 0, &out),
        napi_create_string_latin1(NULL, "x", 1, &out),
        node_api_create_external_string_latin1(NULL, NULL, 0, NULL, NULL, &out, NULL),
        napi_create_symbol(NULL, NULL, &out),
        napi_coerce_to_bool(NULL, v, &out),
        napi_create_array(NULL, &out),
        napi_set_property(NULL, v, v, v),
        napi_has_element(NULL, v, 0, NULL),
        napi_object_freeze(NULL, v),
        napi_strict_equals(NULL, v, v, NULL),
        napi_is_buffer(NULL, v, NULL),
        napi_get_arraybuffer_info(NULL, v, NULL, NULL),
        napi_detach_arraybuffer(NULL, v),
        napi_create_dataview(NULL, 0, v, 0, &out),
        napi_create_promise(NULL, NULL, NULL),
        napi_create_external(NULL, NULL, NULL, NULL, &out),
        napi_new_instance(NULL, v, 0, NULL, &out),
        napi_run_script(NULL, v, &out),
        napi_open_handle_scope(NULL, NULL),
        napi_close_handle_scope(NULL, NULL),
        napi_escape_handle(NULL, NULL, v, &out),
        napi_set_instance_data(NULL, NULL, NULL, NULL),
        napi_get_instance_data(NULL, NULL),
        napi_add_env_cleanup_hook(NULL, NULL, NULL),
        napi_remove_env_cleanup_hook(NULL, NULL, NULL),
        napi_add_async_cleanup_hook(NULL, NULL, NULL, NULL),
        napi_adjust_external_memory(NULL, 0, NULL),
        node_api_get_module_file_name(NULL, NULL),
        napi_cancel_async_work(NULL, NULL),
        napi_close_callback_scope(NULL, NULL),
        napi_create_async_work(NULL, NULL, v, NULL, NULL, NULL, NULL),
        napi_async_init(NULL, NULL, v, NULL),
        napi_make_callback(NULL, NULL, v, v, 0, NULL, &out),
        napi_get_uv_event_loop(NULL, NULL),
        napi_create_threadsafe_function(NULL, NULL, NULL, v, 0, 1, NULL, NULL, NULL, NULL, NULL),
    };
    return nums(env, sizeof st / sizeof st[0], st);
}

static const struct {
    const char *name;
    napi_callback cb;
} FNS[] = {
    {"throwKind", f_throw_kind},
    {"createKind", f_create_kind},
    {"errorChecks", f_error_checks},
    {"lastErrorInfo", f_last_error_info},
    {"fatalException", f_fatal_exception},
    {"numbers", f_numbers},
    {"getInt32", f_get_int32},
    {"bigints", f_bigints},
    {"getBigInt64", f_get_bigint64},
    {"getBigUint64", f_get_biguint64},
    {"getWords", f_get_words},
    {"bigintChecks", f_bigint_checks},
    {"dates", f_dates},
    {"strings", f_strings},
    {"getLatin1", f_get_latin1},
    {"getUtf16", f_get_utf16},
    {"getStringChecks", f_get_string_checks},
    {"externalStrings", f_external_strings},
    {"extFinalized", f_ext_finalized},
    {"symbols", f_symbols},
    {"coerce", f_coerce},
    {"coerceChecks", f_coerce_checks},
    {"createArray", f_create_array},
    {"props", f_props},
    {"propChecks", f_prop_checks},
    {"elements", f_elements},
    {"elementChecks", f_element_checks},
    {"allNames", f_all_names},
    {"proto", f_proto},
    {"integrity", f_integrity},
    {"objectChecks", f_object_checks},
    {"compare", f_compare},
    {"instanceOf", f_instance_of},
    {"kinds", f_kinds},
    {"compareChecks", f_compare_checks},
    {"abInfo", f_ab_info},
    {"taInfo", f_ta_info},
    {"dataView", f_data_view},
    {"dvInfo", f_dv_info},
    {"detach", f_detach},
    {"bufferFromAb", f_buffer_from_ab},
    {"viewChecks", f_view_checks},
    {"externalAb", f_external_ab},
    {"extAbPoke", f_ext_ab_poke},
    {"extAbFinalized", f_ext_ab_finalized},
    {"externalAbNoResult", f_external_ab_no_result},
    {"promise", f_promise},
    {"deferLater", f_defer_later},
    {"settleLater", f_settle_later},
    {"promiseChecks", f_promise_checks},
    {"externals", f_externals},
    {"externalFinalized", f_external_finalized},
    {"externalTags", f_external_tags},
    {"externalTagNulls", f_external_tag_nulls},
    {"newInstance", f_new_instance},
    {"newInstanceChecks", f_new_instance_checks},
    {"runScript", f_run_script},
    {"handleScopes", f_handle_scopes},
    {"escapeChecks", f_escape_checks},
    {"externalMemory", f_external_memory},
    {"moduleFileName", f_module_file_name},
    {"coreNulls", f_core_nulls},
    {"typedArrayOver", f_typed_array_over},
    {"buffers", f_buffers},
    {"throwingFinalizer", f_throwing_finalizer},
    {"nullEnv", f_null_env},
};

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    for (size_t i = 0; i < sizeof FNS / sizeof FNS[0]; i++) {
        napi_value f;
        napi_create_function(env, FNS[i].name, NAPI_AUTO_LENGTH, FNS[i].cb, NULL, &f);
        napi_set_named_property(env, exports, FNS[i].name, f);
    }
    return exports;
}

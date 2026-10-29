/* A node-addon-api-style ObjectWrap class: napi_define_class with the property descriptors
 * node-addon-api's ObjectWrap<T>::DefineClass builds from InstanceMethod / InstanceAccessor /
 * InstanceValue / StaticMethod / StaticAccessor / StaticValue (each member's callback data is a
 * record naming the member, one wrapper callback per kind, as node-addon-api does), and the
 * ObjectWrap constructor (napi_get_new_target, napi_wrap with a finalizer and a reference).
 * Also: napi_define_class / napi_define_properties argument checks and napi_define_properties
 * on a plain object. */
#include "napi.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    double x, y;
    napi_ref ref;
} Point;

static double g_created, g_live, g_precision = 2;

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
static napi_value undef(napi_env env) {
    napi_value v;
    napi_get_undefined(env, &v);
    return v;
}
static napi_value list(napi_env env, size_t n, const double *items) {
    napi_value arr;
    napi_create_array_with_length(env, n, &arr);
    for (size_t i = 0; i < n; i++) napi_set_element(env, arr, (uint32_t)i, num(env, items[i]));
    return arr;
}
static double argn(napi_env env, napi_value v) {
    double d = -1;
    napi_get_value_double(env, v, &d);
    return d;
}

/* ---- the ObjectWrap machinery (as node-addon-api's ObjectWrap<T>) ---------------------------- */

typedef napi_value (*PointMethod)(napi_env, Point *, size_t, napi_value *);
typedef napi_value (*PointGetter)(napi_env, Point *);
typedef void (*PointSetter)(napi_env, Point *, napi_value);
typedef struct {
    PointMethod method;
    PointGetter getter;
    PointSetter setter;
} Member;

/* The Point behind `this`, or NULL with an Error thrown (ObjectWrap::Unwrap). */
static Point *this_point(napi_env env, napi_callback_info info, size_t *argc, napi_value *argv,
                         Member **member) {
    napi_value self;
    napi_get_cb_info(env, info, argc, argv, &self, (void **)member);
    Point *p = NULL;
    if (napi_unwrap(env, self, (void **)&p) != napi_ok) {
        napi_throw_error(env, NULL, "not a Point");
        return NULL;
    }
    return p;
}
static napi_value instance_method_wrapper(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2];
    Member *m;
    Point *p = this_point(env, info, &argc, argv, &m);
    return p ? m->method(env, p, argc, argv) : NULL;
}
static napi_value instance_getter_wrapper(napi_env env, napi_callback_info info) {
    size_t argc = 0;
    Member *m;
    Point *p = this_point(env, info, &argc, NULL, &m);
    return p ? m->getter(env, p) : NULL;
}
static napi_value instance_setter_wrapper(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1];
    Member *m;
    Point *p = this_point(env, info, &argc, argv, &m);
    if (p) m->setter(env, p, argv[0]);
    return NULL;
}

static void point_finalize(napi_env env, void *data, void *hint) {
    (void)hint;
    Point *p = data;
    napi_delete_reference(env, p->ref);
    free(p);
    g_live--;
}
/* ConstructorCallbackWrapper: a call without `new` throws; `new` wraps a fresh Point. */
static napi_value point_ctor(napi_env env, napi_callback_info info) {
    napi_value new_target, self, argv[2];
    size_t argc = 2;
    if (napi_get_new_target(env, info, &new_target) != napi_ok) return NULL;
    if (new_target == NULL) {
        napi_throw_error(env, NULL, "Class constructors cannot be invoked without 'new'");
        return NULL;
    }
    napi_get_cb_info(env, info, &argc, argv, &self, NULL);
    Point *p = calloc(1, sizeof *p);
    p->x = argn(env, argv[0]);
    p->y = argn(env, argv[1]);
    if (napi_wrap(env, self, p, point_finalize, NULL, &p->ref) != napi_ok) {
        free(p);
        napi_throw_error(env, NULL, "wrap failed");
        return NULL;
    }
    g_created++;
    g_live++;
    return self;
}

/* ---- the members ------------------------------------------------------------------------- */

static napi_value m_norm2(napi_env env, Point *p, size_t argc, napi_value *argv) {
    (void)argc;
    (void)argv;
    return num(env, p->x * p->x + p->y * p->y);
}
static napi_value m_scale(napi_env env, Point *p, size_t argc, napi_value *argv) {
    double k = argc > 0 ? argn(env, argv[0]) : 1;
    p->x *= k;
    p->y *= k;
    return undef(env);
}
static napi_value m_to_primitive(napi_env env, Point *p, size_t argc, napi_value *argv) {
    (void)argc;
    (void)argv;
    return num(env, p->x + p->y);
}
static napi_value g_x(napi_env env, Point *p) { return num(env, p->x); }
static void s_x(napi_env env, Point *p, napi_value v) { p->x = argn(env, v); }
static napi_value g_y(napi_env env, Point *p) { return num(env, p->y); }
static void s_both(napi_env env, Point *p, napi_value v) { p->x = p->y = argn(env, v); }

static Member NORM2 = {m_norm2, NULL, NULL}, SCALE = {m_scale, NULL, NULL},
              TO_PRIMITIVE = {m_to_primitive, NULL, NULL}, X = {NULL, g_x, s_x},
              Y = {NULL, g_y, NULL}, BOTH = {NULL, NULL, s_both};

/* Static members: StaticMethod becomes a function value (node-addon-api creates it first);
 * a plain C addon passes the method itself. `data` is the descriptor's data. */
static napi_value st_created(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, g_created);
}
static napi_value st_tag(napi_env env, napi_callback_info info) {
    void *data;
    napi_get_cb_info(env, info, NULL, NULL, NULL, &data);
    return str(env, data);
}
static napi_value st_live(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, g_live);
}
static napi_value st_get_precision(napi_env env, napi_callback_info info) {
    (void)info;
    return num(env, g_precision);
}
static napi_value st_set_precision(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value v;
    napi_get_cb_info(env, info, &argc, &v, NULL, NULL);
    g_precision = argn(env, v);
    return NULL;
}

static napi_value define_point(napi_env env) {
    napi_value global, symbol, to_primitive, created, kind, dims;
    napi_get_global(env, &global);
    napi_get_named_property(env, global, "Symbol", &symbol);
    napi_get_named_property(env, symbol, "toPrimitive", &to_primitive);
    napi_create_function(env, "created", NAPI_AUTO_LENGTH, st_created, NULL, &created);
    kind = str(env, "point");
    dims = num(env, 2);
    const napi_property_descriptor props[] = {
        /* InstanceMethod(name, fn) and InstanceMethod(name, fn, napi_default_method) */
        {"norm2", NULL, instance_method_wrapper, NULL, NULL, NULL, napi_default, &NORM2},
        {"scale", NULL, instance_method_wrapper, NULL, NULL, NULL, napi_default_method, &SCALE},
        /* InstanceMethod(Napi::Symbol, ...): a name given as a napi_value */
        {NULL, to_primitive, instance_method_wrapper, NULL, NULL, NULL, napi_default,
         &TO_PRIMITIVE},
        /* InstanceAccessor: get + set, get only, set only */
        {"x", NULL, NULL, instance_getter_wrapper, instance_setter_wrapper, NULL, napi_enumerable,
         &X},
        {"y", NULL, NULL, instance_getter_wrapper, NULL, NULL, napi_configurable, &Y},
        {"both", NULL, NULL, NULL, instance_setter_wrapper, NULL, napi_default, &BOTH},
        /* InstanceValue */
        {"kind", NULL, NULL, NULL, NULL, kind, napi_default, NULL},
        /* StaticMethod (as node-addon-api: a function value), a C static method with data,
         * StaticAccessor (get only, get + set), StaticValue */
        {"created", NULL, NULL, NULL, NULL, created, napi_static | napi_default_method, NULL},
        {"tag", NULL, st_tag, NULL, NULL, NULL, napi_static | napi_enumerable, "static data"},
        {"live", NULL, NULL, st_live, NULL, NULL, napi_static, NULL},
        {"precision", NULL, NULL, st_get_precision, st_set_precision, NULL,
         napi_static | napi_configurable, NULL},
        {"dimensions", NULL, NULL, NULL, NULL, dims, napi_static | napi_enumerable, NULL},
    };
    napi_value cls;
    napi_status st = napi_define_class(env, "Point", NAPI_AUTO_LENGTH, point_ctor, NULL,
                                       sizeof props / sizeof props[0], props, &cls);
    if (st != napi_ok) {
        char msg[64];
        snprintf(msg, sizeof msg, "napi_define_class failed: %d", st);
        napi_throw_error(env, NULL, msg);
        return NULL;
    }
    return cls;
}

/* ---- argument checks --------------------------------------------------------------------- */

static napi_value noop(napi_env env, napi_callback_info info) {
    (void)info;
    return undef(env);
}

/* defineClassChecks() -> the statuses of napi_define_class with: a NULL result, a NULL
 * constructor, properties missing for a count, a NULL name, a non-name napi_value name (on the
 * prototype, then static: the class is still returned), a pending exception; then the class the
 * failing static definition returned. */
static napi_value f_define_class_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value cls = NULL, bad_name = num(env, 1), exc, partial;
    napi_property_descriptor bad = {NULL, bad_name, noop, NULL, NULL, NULL, napi_default, NULL};
    napi_property_descriptor bad_static = bad;
    bad_static.attributes = napi_static;
    double st[7];
    st[0] = napi_define_class(env, "C", NAPI_AUTO_LENGTH, noop, NULL, 0, NULL, NULL);
    st[1] = napi_define_class(env, "C", NAPI_AUTO_LENGTH, NULL, NULL, 0, NULL, &cls);
    st[2] = napi_define_class(env, "C", NAPI_AUTO_LENGTH, noop, NULL, 1, NULL, &cls);
    st[3] = napi_define_class(env, NULL, 0, noop, NULL, 0, NULL, &cls);
    st[4] = napi_define_class(env, "C", NAPI_AUTO_LENGTH, noop, NULL, 1, &bad, &cls);
    cls = NULL;
    st[5] = napi_define_class(env, "PartialName", 7, noop, NULL, 1, &bad_static, &cls);
    partial = cls ? cls : undef(env);
    napi_throw_error(env, NULL, "pending");
    st[6] = napi_define_class(env, "C", NAPI_AUTO_LENGTH, noop, NULL, 0, NULL, &cls);
    napi_get_and_clear_last_exception(env, &exc);
    napi_value res;
    napi_create_array_with_length(env, 2, &res);
    napi_set_element(env, res, 0, list(env, 7, st));
    napi_set_element(env, res, 1, partial);
    return res;
}

/* newTargetChecks() -> statuses of napi_get_new_target with a NULL info and a NULL result. */
static napi_value f_new_target_checks(napi_env env, napi_callback_info info) {
    napi_value v;
    double st[2] = {napi_get_new_target(env, NULL, &v), napi_get_new_target(env, info, NULL)};
    return list(env, 2, st);
}

/* hostOnlyChecks() -> statuses of descriptors Node aborts or crashes on, which the host
 * refuses: an object InstanceValue (V8 templates take only primitives), a NULL InstanceValue,
 * no name at all (NULL utf8name and NULL name), a NULL value for napi_define_properties. */
static napi_value f_host_only_checks(napi_env env, napi_callback_info info) {
    (void)info;
    napi_value cls, obj;
    napi_create_object(env, &obj);
    napi_property_descriptor o = {"o", NULL, NULL, NULL, NULL, obj, napi_default, NULL};
    napi_property_descriptor null_value = {"n", NULL, NULL, NULL, NULL, NULL, napi_default, NULL};
    napi_property_descriptor no_name = {NULL, NULL, noop, NULL, NULL, NULL, napi_default, NULL};
    double st[4] = {
        napi_define_class(env, "C", NAPI_AUTO_LENGTH, noop, NULL, 1, &o, &cls),
        napi_define_class(env, "C", NAPI_AUTO_LENGTH, noop, NULL, 1, &null_value, &cls),
        napi_define_class(env, "C", NAPI_AUTO_LENGTH, noop, NULL, 1, &no_name, &cls),
        napi_define_properties(env, obj, 1, &null_value),
    };
    return list(env, 4, st);
}

/* defineProperties(obj) -> status: data properties (the all-true fast path, a read-only one), a
 * method with data and an accessor pair on `obj`. */
static napi_value f_define_properties(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value obj;
    napi_get_cb_info(env, info, &argc, &obj, NULL, NULL);
    const napi_property_descriptor props[] = {
        {"a", NULL, NULL, NULL, NULL, num(env, 1), napi_default_jsproperty, NULL},
        {"b", NULL, NULL, NULL, NULL, num(env, 2), napi_enumerable, NULL},
        {"m", NULL, st_tag, NULL, NULL, NULL, napi_writable, "method data"},
        {"p", NULL, NULL, st_get_precision, st_set_precision, NULL, napi_enumerable, NULL},
    };
    return num(env, napi_define_properties(env, obj, 4, props));
}
/* definePropertiesChecks(frozen) -> statuses: NULL object, properties missing for a count, a
 * non-name name, then a data property, a method and an accessor on the frozen object. */
static napi_value f_define_properties_checks(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value frozen, obj;
    napi_get_cb_info(env, info, &argc, &frozen, NULL, NULL);
    napi_create_object(env, &obj);
    napi_property_descriptor bad = {NULL, num(env, 1), noop, NULL, NULL, NULL, napi_default, NULL};
    napi_property_descriptor data = {"d", NULL, NULL, NULL, NULL, num(env, 1), napi_default, NULL};
    napi_property_descriptor fast = data;
    fast.attributes = napi_default_jsproperty;
    napi_property_descriptor method = {"m", NULL, noop, NULL, NULL, NULL, napi_default, NULL};
    napi_property_descriptor acc = {"g", NULL, NULL, noop, NULL, NULL, napi_default, NULL};
    double st[7] = {
        napi_define_properties(env, NULL, 0, NULL),
        napi_define_properties(env, obj, 1, NULL),
        napi_define_properties(env, obj, 1, &bad),
        napi_define_properties(env, frozen, 1, &data),
        napi_define_properties(env, frozen, 1, &fast),
        napi_define_properties(env, frozen, 1, &method),
        napi_define_properties(env, frozen, 1, &acc),
    };
    return list(env, 7, st);
}

/* defineAccessorHalf(obj, setterOnly) -> status: napi_define_properties of a configurable
 * accessor "p" with only a getter (or only a setter); the missing half is left out of the
 * definition, so an existing accessor keeps it. */
static napi_value f_define_accessor_half(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2];
    napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
    bool setter_only = false;
    napi_get_value_bool(env, argv[1], &setter_only);
    napi_property_descriptor p = {"p", NULL, NULL, NULL, NULL, NULL, napi_configurable, NULL};
    if (setter_only)
        p.setter = noop;
    else
        p.getter = noop;
    return num(env, napi_define_properties(env, argv[0], 1, &p));
}

static const struct {
    const char *name;
    napi_callback cb;
} FNS[] = {
    {"defineAccessorHalf", f_define_accessor_half},
    {"defineClassChecks", f_define_class_checks},
    {"newTargetChecks", f_new_target_checks},
    {"hostOnlyChecks", f_host_only_checks},
    {"defineProperties", f_define_properties},
    {"definePropertiesChecks", f_define_properties_checks},
};

napi_value napi_register_module_v1(napi_env env, napi_value exports) {
    for (size_t i = 0; i < sizeof FNS / sizeof FNS[0]; i++) {
        napi_value f;
        napi_create_function(env, FNS[i].name, NAPI_AUTO_LENGTH, FNS[i].cb, NULL, &f);
        napi_set_named_property(env, exports, FNS[i].name, f);
    }
    napi_value cls = define_point(env);
    if (cls) napi_set_named_property(env, exports, "Point", cls);
    return exports;
}

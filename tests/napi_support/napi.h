/* Node-API declarations for the turbo-test napi host integration tests (written for these
 * fixtures, not Node's headers). Only what the host exports is declared; every one is left
 * undefined in the compiled .node and resolved against the turbo-test binary at
 * dlopen time (macOS: -undefined dynamic_lookup, Linux: default for -shared). */
#ifndef TT_NAPI_H
#define TT_NAPI_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef int napi_status;
typedef void *napi_value;
typedef void *napi_env;
typedef void *napi_ref;
typedef void *napi_callback_info;
typedef napi_value (*napi_callback)(napi_env, napi_callback_info);
typedef void (*napi_finalize)(napi_env, void *, void *);

#define napi_ok 0
#define napi_invalid_arg 1
#define napi_object_expected 2
#define napi_name_expected 4
#define napi_generic_failure 9
#define napi_pending_exception 10

#define NAPI_AUTO_LENGTH SIZE_MAX

extern napi_status napi_get_undefined(napi_env, napi_value *);
extern napi_status napi_get_null(napi_env, napi_value *);
extern napi_status napi_get_global(napi_env, napi_value *);
extern napi_status napi_get_boolean(napi_env, bool, napi_value *);
extern napi_status napi_typeof(napi_env, napi_value, int *);
extern napi_status napi_create_uint32(napi_env, uint32_t, napi_value *);
extern napi_status napi_create_double(napi_env, double, napi_value *);
extern napi_status napi_get_value_uint32(napi_env, napi_value, uint32_t *);
extern napi_status napi_create_string_utf8(napi_env, const char *, size_t, napi_value *);
extern napi_status napi_get_value_string_utf8(napi_env, napi_value, char *, size_t, size_t *);
extern napi_status napi_coerce_to_string(napi_env, napi_value, napi_value *);
extern napi_status napi_coerce_to_bool(napi_env, napi_value, napi_value *);
extern napi_status napi_coerce_to_number(napi_env, napi_value, napi_value *);
extern napi_status napi_coerce_to_object(napi_env, napi_value, napi_value *);
extern napi_status napi_create_string_latin1(napi_env, const char *, size_t, napi_value *);
extern napi_status napi_create_string_utf16(napi_env, const uint16_t *, size_t, napi_value *);
extern napi_status napi_get_value_string_latin1(napi_env, napi_value, char *, size_t, size_t *);
extern napi_status napi_get_value_string_utf16(napi_env, napi_value, uint16_t *, size_t,
                                               size_t *);
extern napi_status node_api_create_property_key_latin1(napi_env, const char *, size_t,
                                                       napi_value *);
extern napi_status node_api_create_property_key_utf8(napi_env, const char *, size_t,
                                                     napi_value *);
extern napi_status node_api_create_property_key_utf16(napi_env, const uint16_t *, size_t,
                                                      napi_value *);
extern napi_status node_api_create_external_string_latin1(napi_env, char *, size_t,
                                                          napi_finalize, void *, napi_value *,
                                                          bool *);
extern napi_status node_api_create_external_string_utf16(napi_env, uint16_t *, size_t,
                                                         napi_finalize, void *, napi_value *,
                                                         bool *);
extern napi_status napi_create_symbol(napi_env, napi_value, napi_value *);
extern napi_status node_api_symbol_for(napi_env, const char *, size_t, napi_value *);
extern napi_status napi_create_array(napi_env, napi_value *);
extern napi_status napi_create_object(napi_env, napi_value *);
extern napi_status napi_create_array_with_length(napi_env, size_t, napi_value *);
extern napi_status napi_set_named_property(napi_env, napi_value, const char *, napi_value);
extern napi_status napi_get_named_property(napi_env, napi_value, const char *, napi_value *);
extern napi_status napi_set_element(napi_env, napi_value, uint32_t, napi_value);
extern napi_status napi_set_property(napi_env, napi_value, napi_value, napi_value);
extern napi_status napi_get_property(napi_env, napi_value, napi_value, napi_value *);
extern napi_status napi_has_property(napi_env, napi_value, napi_value, bool *);
extern napi_status napi_delete_property(napi_env, napi_value, napi_value, bool *);
extern napi_status napi_has_own_property(napi_env, napi_value, napi_value, bool *);
extern napi_status napi_has_named_property(napi_env, napi_value, const char *, bool *);
extern napi_status napi_has_element(napi_env, napi_value, uint32_t, bool *);
extern napi_status napi_delete_element(napi_env, napi_value, uint32_t, bool *);
extern napi_status napi_get_all_property_names(napi_env, napi_value, int, int, int, napi_value *);
extern napi_status napi_get_prototype(napi_env, napi_value, napi_value *);
extern napi_status napi_object_freeze(napi_env, napi_value);
extern napi_status napi_object_seal(napi_env, napi_value);
extern napi_status napi_create_function(napi_env, const char *, size_t, napi_callback, void *,
                                        napi_value *);
extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t *, napi_value *,
                                    napi_value *, void **);
extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t,
                                      const napi_value *, napi_value *);
extern napi_status napi_create_arraybuffer(napi_env, size_t, void **, napi_value *);
extern napi_status napi_create_typedarray(napi_env, int, size_t, napi_value, size_t,
                                          napi_value *);
extern napi_status napi_create_external_arraybuffer(napi_env, void *, size_t, napi_finalize,
                                                    void *, napi_value *);
extern napi_status napi_create_buffer(napi_env, size_t, void **, napi_value *);
extern napi_status napi_create_buffer_copy(napi_env, size_t, const void *, void **,
                                           napi_value *);
extern napi_status napi_create_external_buffer(napi_env, size_t, void *, napi_finalize, void *,
                                               napi_value *);
extern napi_status napi_get_buffer_info(napi_env, napi_value, void **, size_t *);
extern napi_status napi_get_arraybuffer_info(napi_env, napi_value, void **, size_t *);
extern napi_status napi_get_typedarray_info(napi_env, napi_value, int *, size_t *, void **,
                                            napi_value *, size_t *);
extern napi_status napi_create_dataview(napi_env, size_t, napi_value, size_t, napi_value *);
extern napi_status napi_get_dataview_info(napi_env, napi_value, size_t *, void **, napi_value *,
                                          size_t *);
extern napi_status napi_detach_arraybuffer(napi_env, napi_value);
extern napi_status node_api_create_buffer_from_arraybuffer(napi_env, napi_value, size_t, size_t,
                                                           napi_value *);
extern napi_status napi_get_value_bool(napi_env, napi_value, bool *);
extern napi_status napi_get_value_double(napi_env, napi_value, double *);
extern napi_status napi_get_value_int64(napi_env, napi_value, int64_t *);
extern napi_status napi_create_int32(napi_env, int32_t, napi_value *);
extern napi_status napi_create_int64(napi_env, int64_t, napi_value *);
extern napi_status napi_get_value_int32(napi_env, napi_value, int32_t *);
extern napi_status napi_create_bigint_int64(napi_env, int64_t, napi_value *);
extern napi_status napi_create_bigint_uint64(napi_env, uint64_t, napi_value *);
extern napi_status napi_create_bigint_words(napi_env, int, size_t, const uint64_t *, napi_value *);
extern napi_status napi_get_value_bigint_int64(napi_env, napi_value, int64_t *, bool *);
extern napi_status napi_get_value_bigint_uint64(napi_env, napi_value, uint64_t *, bool *);
extern napi_status napi_get_value_bigint_words(napi_env, napi_value, int *, size_t *, uint64_t *);
extern napi_status napi_create_date(napi_env, double, napi_value *);
extern napi_status napi_is_date(napi_env, napi_value, bool *);
extern napi_status napi_get_date_value(napi_env, napi_value, double *);
extern napi_status napi_is_array(napi_env, napi_value, bool *);
extern napi_status napi_strict_equals(napi_env, napi_value, napi_value, bool *);
extern napi_status napi_instanceof(napi_env, napi_value, napi_value, bool *);
extern napi_status napi_is_arraybuffer(napi_env, napi_value, bool *);
extern napi_status napi_is_typedarray(napi_env, napi_value, bool *);
extern napi_status napi_is_dataview(napi_env, napi_value, bool *);
extern napi_status napi_is_buffer(napi_env, napi_value, bool *);
extern napi_status napi_is_detached_arraybuffer(napi_env, napi_value, bool *);
extern napi_status napi_get_array_length(napi_env, napi_value, uint32_t *);
extern napi_status napi_get_element(napi_env, napi_value, uint32_t, napi_value *);
extern napi_status napi_get_property_names(napi_env, napi_value, napi_value *);
extern napi_status napi_wrap(napi_env, napi_value, void *, napi_finalize, void *, napi_ref *);
extern napi_status napi_unwrap(napi_env, napi_value, void **);
extern napi_status napi_remove_wrap(napi_env, napi_value, void **);
typedef void *napi_deferred;
extern napi_status napi_create_promise(napi_env, napi_deferred *, napi_value *);
extern napi_status napi_resolve_deferred(napi_env, napi_deferred, napi_value);
extern napi_status napi_reject_deferred(napi_env, napi_deferred, napi_value);
extern napi_status napi_is_promise(napi_env, napi_value, bool *);
extern napi_status napi_create_external(napi_env, void *, napi_finalize, void *, napi_value *);
extern napi_status napi_get_value_external(napi_env, napi_value, void **);
extern napi_status napi_new_instance(napi_env, napi_value, size_t, const napi_value *,
                                     napi_value *);
extern napi_status napi_throw(napi_env, napi_value);
extern napi_status napi_throw_error(napi_env, const char *, const char *);
extern napi_status napi_throw_type_error(napi_env, const char *, const char *);
extern napi_status napi_throw_range_error(napi_env, const char *, const char *);
extern napi_status node_api_throw_syntax_error(napi_env, const char *, const char *);
extern napi_status napi_create_error(napi_env, napi_value, napi_value, napi_value *);
extern napi_status napi_create_type_error(napi_env, napi_value, napi_value, napi_value *);
extern napi_status napi_create_range_error(napi_env, napi_value, napi_value, napi_value *);
extern napi_status node_api_create_syntax_error(napi_env, napi_value, napi_value, napi_value *);
typedef struct {
    const char *error_message;
    void *engine_reserved;
    uint32_t engine_error_code;
    napi_status error_code;
} napi_extended_error_info;
extern napi_status napi_get_last_error_info(napi_env, const napi_extended_error_info **);
extern napi_status napi_fatal_exception(napi_env, napi_value);
extern void napi_fatal_error(const char *, size_t, const char *, size_t) __attribute__((noreturn));
extern napi_status napi_is_error(napi_env, napi_value, bool *);
extern napi_status napi_is_exception_pending(napi_env, bool *);
extern napi_status napi_get_and_clear_last_exception(napi_env, napi_value *);
extern napi_status napi_create_reference(napi_env, napi_value, uint32_t, napi_ref *);
extern napi_status napi_get_reference_value(napi_env, napi_ref, napi_value *);
extern napi_status napi_delete_reference(napi_env, napi_ref);
extern napi_status napi_reference_ref(napi_env, napi_ref, uint32_t *);
extern napi_status napi_reference_unref(napi_env, napi_ref, uint32_t *);
extern napi_status napi_add_env_cleanup_hook(napi_env, void (*)(void *), void *);
extern napi_status napi_remove_env_cleanup_hook(napi_env, void (*)(void *), void *);
typedef void *napi_async_cleanup_hook_handle;
extern napi_status napi_add_async_cleanup_hook(napi_env,
                                               void (*)(napi_async_cleanup_hook_handle, void *),
                                               void *, napi_async_cleanup_hook_handle *);
extern napi_status napi_remove_async_cleanup_hook(napi_async_cleanup_hook_handle);
extern napi_status napi_set_instance_data(napi_env, void *, napi_finalize, void *);
extern napi_status napi_get_instance_data(napi_env, void **);
extern napi_status napi_adjust_external_memory(napi_env, int64_t, int64_t *);
typedef void *napi_handle_scope;
typedef void *napi_escapable_handle_scope;
extern napi_status napi_open_handle_scope(napi_env, napi_handle_scope *);
extern napi_status napi_close_handle_scope(napi_env, napi_handle_scope);
extern napi_status napi_open_escapable_handle_scope(napi_env, napi_escapable_handle_scope *);
extern napi_status napi_close_escapable_handle_scope(napi_env, napi_escapable_handle_scope);
extern napi_status napi_escape_handle(napi_env, napi_escapable_handle_scope, napi_value,
                                      napi_value *);
extern napi_status napi_run_script(napi_env, napi_value, napi_value *);
extern napi_status node_api_get_module_file_name(napi_env, const char **);
typedef napi_value (*napi_addon_register_func)(napi_env, napi_value);
typedef struct {
    int nm_version;
    unsigned int nm_flags;
    const char *nm_filename;
    napi_addon_register_func nm_register_func;
    const char *nm_modname;
    void *nm_priv;
    void *reserved[4];
} napi_module;
extern void napi_module_register(napi_module *);
extern napi_status napi_get_version(napi_env, uint32_t *);
typedef struct {
    uint32_t major;
    uint32_t minor;
    uint32_t patch;
    const char *release;
} napi_node_version;
extern napi_status napi_get_node_version(napi_env, const napi_node_version **);
extern napi_status napi_stub(void);
typedef enum {
    napi_default = 0,
    napi_writable = 1 << 0,
    napi_enumerable = 1 << 1,
    napi_configurable = 1 << 2,
    napi_static = 1 << 10,
    napi_default_method = napi_writable | napi_configurable,
    napi_default_jsproperty = napi_writable | napi_enumerable | napi_configurable,
} napi_property_attributes;

typedef struct {
    const char *utf8name;
    napi_value name;
    napi_callback method;
    napi_callback getter;
    napi_callback setter;
    napi_value value;
    napi_property_attributes attributes;
    void *data;
} napi_property_descriptor;

extern napi_status napi_define_class(napi_env, const char *, size_t, napi_callback, void *,
                                     size_t, const napi_property_descriptor *, napi_value *);
extern napi_status napi_define_properties(napi_env, napi_value, size_t,
                                          const napi_property_descriptor *);
extern napi_status napi_get_new_target(napi_env, napi_callback_info, napi_value *);

typedef struct {
    uint64_t lower;
    uint64_t upper;
} napi_type_tag;

extern napi_status napi_type_tag_object(napi_env, napi_value, const napi_type_tag *);
extern napi_status napi_check_object_type_tag(napi_env, napi_value, const napi_type_tag *,
                                              bool *);
extern napi_status napi_add_finalizer(napi_env, napi_value, void *, napi_finalize, void *,
                                      napi_ref *);
typedef void *napi_threadsafe_function;
typedef void (*napi_threadsafe_function_call_js)(napi_env, napi_value, void *, void *);
#define napi_tsfn_nonblocking 0
#define napi_tsfn_blocking 1
#define napi_tsfn_release 0
#define napi_tsfn_abort 1
extern napi_status napi_create_threadsafe_function(napi_env, napi_value, napi_value, napi_value,
                                                   size_t, size_t, void *, napi_finalize, void *,
                                                   napi_threadsafe_function_call_js,
                                                   napi_threadsafe_function *);
extern napi_status napi_get_threadsafe_function_context(napi_threadsafe_function, void **);
extern napi_status napi_call_threadsafe_function(napi_threadsafe_function, void *, int);
extern napi_status napi_acquire_threadsafe_function(napi_threadsafe_function);
extern napi_status napi_release_threadsafe_function(napi_threadsafe_function, int);
extern napi_status napi_ref_threadsafe_function(napi_env, napi_threadsafe_function);
extern napi_status napi_unref_threadsafe_function(napi_env, napi_threadsafe_function);
typedef void *napi_async_work;
typedef void (*napi_async_execute_callback)(napi_env, void *);
typedef void (*napi_async_complete_callback)(napi_env, napi_status, void *);
extern napi_status napi_create_async_work(napi_env, napi_value, napi_value,
                                          napi_async_execute_callback,
                                          napi_async_complete_callback, void *,
                                          napi_async_work *);
extern napi_status napi_delete_async_work(napi_env, napi_async_work);
extern napi_status napi_queue_async_work(napi_env, napi_async_work);
extern napi_status napi_cancel_async_work(napi_env, napi_async_work);
typedef void *napi_async_context;
typedef void *napi_callback_scope;
extern napi_status napi_async_init(napi_env, napi_value, napi_value, napi_async_context *);
extern napi_status napi_async_destroy(napi_env, napi_async_context);
extern napi_status napi_make_callback(napi_env, napi_async_context, napi_value, napi_value,
                                      size_t, const napi_value *, napi_value *);
extern napi_status napi_open_callback_scope(napi_env, napi_value, napi_async_context,
                                            napi_callback_scope *);
extern napi_status napi_close_callback_scope(napi_env, napi_callback_scope);
struct uv_loop_s;
extern napi_status napi_get_uv_event_loop(napi_env, struct uv_loop_s **);

#endif

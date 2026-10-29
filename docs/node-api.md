# Node-API (native addons)

turbo-test loads native `.node` addons itself (`src/napi_host.rs`, `src/napi_host_async.rs`):
the binary exports the Node-API functions, and an addon's imports are bound against them when
it is `require()`d. It implements **every function of Node 26's `js_native_api.h` and
`node_api.h` up to `NAPI_VERSION` 10**, the version it reports (`napi_get_version`,
`process.versions.napi`). The experimental functions (`NAPI_EXPERIMENTAL`:
`node_api_post_finalizer`, `node_api_create_object_with_properties`, `node_api_set_prototype`,
`node_api_create_sharedarraybuffer`, `node_api_create_external_sharedarraybuffer`,
`node_api_is_sharedarraybuffer`) are not provided.

Loading:

- Every import is bound when the addon loads. An addon importing a function the process does
  not provide (an experimental one, a `uv_*` function, a typo) fails its `require()` with an
  error naming the import (`it imports napi_x, which turbo-test's Node-API host does not
  provide`); the run goes on.
- The init is the addon's `napi_register_module_v1`, or the module it registered with
  `napi_module_register` from a static constructor. Its `node_api_module_get_api_version_v1`
  sets the module API version as in Node: none or below 8 is 8, above 10 fails the
  `require()` (but `NAPI_VERSION_EXPERIMENTAL`).
- Each addon gets its own env per isolate. Calls into addon code are serialized across the
  worker threads (one process-wide lock; a same-thread re-entry takes none).

Semantics follow Node's `js_native_api_v8.cc` / `node_api.cc` (statuses, argument checks,
coercions, pending exceptions, `napi_get_last_error_info`). Asynchronous work runs on a pool of
four threads; its completions and threadsafe-function calls run on the JS thread between
macrotasks, and while async work is in flight or a referenced threadsafe function is open the
event loop waits for them (up to the next timer's delay). At teardown (the isolate's
disposal) the env cleanup hooks, threadsafe functions and finalizers run in Node's order at
exit.

The expected values of the tests are Node 26.11's: each fixture under `tests/napi_support/`
(`*.c`, compiled at test time) is run by its spec both under turbo-test
(`tests/napi_host.rs`) and under Node with the same addon, apart from the cases marked
host-only in the specs (where Node aborts, crashes, deadlocks or uses freed memory).

## Inventory

Version: the first `NAPI_VERSION` that declares the function. Tests: the specs under
`tests/napi_support/` whose fixture calls it. Notes: where the host differs from Node.

| Function | Version | Header | Status | Tests | Notes |
|---|---|---|---|---|---|
| `napi_adjust_external_memory` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_call_function` | 1 | js_native_api.h | implemented | apiversion.test.ts, async.test.ts, full.test.ts, init.test.ts, wrap.test.ts |  |
| `napi_close_escapable_handle_scope` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_close_handle_scope` | 1 | js_native_api.h | implemented | api.test.ts | an unbalanced callback is not an abort |
| `napi_coerce_to_bool` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_coerce_to_number` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_coerce_to_object` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_coerce_to_string` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts |  |
| `napi_create_array` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_array_with_length` | 1 | js_native_api.h | implemented | api.test.ts, apiversion.test.ts, async.test.ts, cleanup.test.ts, full.test.ts, objectwrap.test.ts, refs.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_create_arraybuffer` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts |  |
| `napi_create_dataview` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_double` | 1 | js_native_api.h | implemented | api.test.ts, apiversion.test.ts, async.test.ts, cleanup.test.ts, full.test.ts, objectwrap.test.ts, refs.test.ts, selfreg.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_create_error` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts |  |
| `napi_create_external` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_external_arraybuffer` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts | the addon's memory, not copied (see napi_create_external_buffer) |
| `napi_create_function` | 1 | js_native_api.h | implemented | `fatal_error_aborts_the_process`, addons.test.ts, api.test.ts, apiversion.test.ts, async.test.ts, cleanup.test.ts, full.test.ts, init_lock.test.ts, objectwrap.test.ts, refs.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_create_int32` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_int64` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_object` | 1 | js_native_api.h | implemented | addons.test.ts, api.test.ts, async.test.ts, full.test.ts, init.test.ts, objectwrap.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_create_promise` | 1 | js_native_api.h | implemented | api.test.ts, async.test.ts |  |
| `napi_create_range_error` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_reference` | 1 | js_native_api.h | implemented | addons.test.ts, apiversion.test.ts, full.test.ts, refs.test.ts, wrap.test.ts |  |
| `napi_create_string_latin1` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_string_utf16` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_string_utf8` | 1 | js_native_api.h | implemented | addons.test.ts, api.test.ts, apiversion.test.ts, async.test.ts, full.test.ts, init.test.ts, objectwrap.test.ts, refs.test.ts, selfreg.test.ts |  |
| `napi_create_symbol` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_type_error` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_typedarray` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts | over an ArrayBuffer or a SharedArrayBuffer |
| `napi_create_uint32` | 1 | js_native_api.h | implemented | full.test.ts, init_lock.test.ts |  |
| `napi_define_class` | 1 | js_native_api.h | implemented | full.test.ts, objectwrap.test.ts, wrap.test.ts |  |
| `napi_define_properties` | 1 | js_native_api.h | implemented | objectwrap.test.ts |  |
| `napi_delete_element` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_delete_property` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_delete_reference` | 1 | js_native_api.h | implemented | full.test.ts, objectwrap.test.ts, refs.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_escape_handle` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_and_clear_last_exception` | 1 | js_native_api.h | implemented | api.test.ts, async.test.ts, full.test.ts, objectwrap.test.ts, typetag.test.ts |  |
| `napi_get_array_length` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_get_arraybuffer_info` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_boolean` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts, init.test.ts |  |
| `napi_get_cb_info` | 1 | js_native_api.h | implemented | api.test.ts, apiversion.test.ts, async.test.ts, cleanup.test.ts, full.test.ts, objectwrap.test.ts, refs.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_get_dataview_info` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_element` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_get_global` | 1 | js_native_api.h | implemented | api.test.ts, apiversion.test.ts, full.test.ts, init.test.ts, objectwrap.test.ts |  |
| `napi_get_last_error_info` | 1 | js_native_api.h | implemented | api.test.ts, async.test.ts |  |
| `napi_get_named_property` | 1 | js_native_api.h | implemented | addons.test.ts, full.test.ts, init.test.ts, objectwrap.test.ts, wrap.test.ts |  |
| `napi_get_new_target` | 1 | js_native_api.h | implemented | objectwrap.test.ts |  |
| `napi_get_null` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_get_property` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_property_names` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_get_prototype` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_reference_value` | 1 | js_native_api.h | implemented | addons.test.ts, apiversion.test.ts, full.test.ts, refs.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_get_typedarray_info` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_undefined` | 1 | js_native_api.h | implemented | api.test.ts, apiversion.test.ts, async.test.ts, full.test.ts, init.test.ts, objectwrap.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_get_value_bool` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts, objectwrap.test.ts |  |
| `napi_get_value_double` | 1 | js_native_api.h | implemented | api.test.ts, async.test.ts, full.test.ts, objectwrap.test.ts, refs.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_get_value_external` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_value_int32` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_value_int64` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_get_value_string_latin1` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_value_string_utf16` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_value_string_utf8` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts |  |
| `napi_get_value_uint32` | 1 | js_native_api.h | implemented | apiversion.test.ts, full.test.ts, refs.test.ts |  |
| `napi_get_version` | 1 | js_native_api.h | implemented | apiversion.test.ts | 10 |
| `napi_has_element` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_has_named_property` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_has_own_property` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_has_property` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_instanceof` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_is_array` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_is_arraybuffer` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_is_dataview` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_is_error` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_is_exception_pending` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_is_promise` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_is_typedarray` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_new_instance` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts |  |
| `napi_open_escapable_handle_scope` | 1 | js_native_api.h | implemented | api.test.ts | counted only (see napi_open_handle_scope); escapes once |
| `napi_open_handle_scope` | 1 | js_native_api.h | implemented | api.test.ts | counted only: values live until the native call returns |
| `napi_reference_ref` | 1 | js_native_api.h | implemented | refs.test.ts |  |
| `napi_reference_unref` | 1 | js_native_api.h | implemented | full.test.ts, refs.test.ts |  |
| `napi_reject_deferred` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_remove_wrap` | 1 | js_native_api.h | implemented | full.test.ts, wrap.test.ts |  |
| `napi_resolve_deferred` | 1 | js_native_api.h | implemented | api.test.ts, async.test.ts |  |
| `napi_run_script` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_set_element` | 1 | js_native_api.h | implemented | api.test.ts, apiversion.test.ts, async.test.ts, cleanup.test.ts, full.test.ts, objectwrap.test.ts, refs.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_set_named_property` | 1 | js_native_api.h | implemented | `fatal_error_aborts_the_process`, addons.test.ts, api.test.ts, apiversion.test.ts, async.test.ts, cleanup.test.ts, full.test.ts, init.test.ts, init_lock.test.ts, objectwrap.test.ts, refs.test.ts, selfreg.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_set_property` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_strict_equals` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_throw` | 1 | js_native_api.h | implemented | full.test.ts |  |
| `napi_throw_error` | 1 | js_native_api.h | implemented | api.test.ts, async.test.ts, full.test.ts, init.test.ts, objectwrap.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_throw_range_error` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_throw_type_error` | 1 | js_native_api.h | implemented | api.test.ts |  |
| `napi_typeof` | 1 | js_native_api.h | implemented | api.test.ts, full.test.ts, typetag.test.ts, wrap.test.ts |  |
| `napi_unwrap` | 1 | js_native_api.h | implemented | full.test.ts, objectwrap.test.ts, wrap.test.ts |  |
| `napi_wrap` | 1 | js_native_api.h | implemented | apiversion.test.ts, cleanup.test.ts, full.test.ts, objectwrap.test.ts, refs.test.ts, wrap.test.ts |  |
| `napi_async_destroy` | 1 | node_api.h | implemented | async.test.ts |  |
| `napi_async_init` | 1 | node_api.h | implemented | api.test.ts, async.test.ts | no async_hooks: the context identifies nothing more |
| `napi_cancel_async_work` | 1 | node_api.h | implemented | api.test.ts, async.test.ts |  |
| `napi_create_async_work` | 1 | node_api.h | implemented | api.test.ts, async.test.ts |  |
| `napi_create_buffer` | 1 | node_api.h | implemented | api.test.ts, full.test.ts |  |
| `napi_create_buffer_copy` | 1 | node_api.h | implemented | api.test.ts, full.test.ts |  |
| `napi_create_external_buffer` | 1 | node_api.h | implemented | api.test.ts, full.test.ts | the addon's memory, not copied; NULL data gets zero-filled memory of its own |
| `napi_delete_async_work` | 1 | node_api.h | implemented | async.test.ts |  |
| `napi_fatal_error` | 1 | node_api.h | implemented | `fatal_error_aborts_the_process` | prints `FATAL ERROR: <location> <message>` and aborts (no native stack trace) |
| `napi_get_buffer_info` | 1 | node_api.h | implemented | api.test.ts, full.test.ts |  |
| `napi_get_node_version` | 1 | node_api.h | implemented | apiversion.test.ts | 24.0.0 "node", the version `process.version` names |
| `napi_is_buffer` | 1 | node_api.h | implemented | api.test.ts |  |
| `napi_make_callback` | 1 | node_api.h | implemented | api.test.ts, async.test.ts | nextTicks / microtasks run when the native call returns to JS or the event loop |
| `napi_module_register` | 1 | node_api.h | implemented | selfreg.test.ts | a library loaded again uses its saved module |
| `napi_queue_async_work` | 1 | node_api.h | implemented | async.test.ts | a pool of 4 threads (libuv's default) |
| `napi_get_uv_event_loop` | 2 | node_api.h | implemented | api.test.ts, async.test.ts | an opaque handle: there is no libuv (an addon importing `uv_*` fails to load) |
| `napi_add_env_cleanup_hook` | 3 | node_api.h | implemented | api.test.ts, cleanup.test.ts, full.test.ts | a duplicate (fun, arg) is `napi_invalid_arg` (Node aborts) |
| `napi_close_callback_scope` | 3 | node_api.h | implemented | api.test.ts, async.test.ts |  |
| `napi_fatal_exception` | 3 | node_api.h | implemented | api.test.ts | reported like an uncaught timer exception (`uncaught <message>` on stderr) |
| `napi_open_callback_scope` | 3 | node_api.h | implemented | async.test.ts |  |
| `napi_remove_env_cleanup_hook` | 3 | node_api.h | implemented | api.test.ts, cleanup.test.ts |  |
| `napi_acquire_threadsafe_function` | 4 | node_api.h | implemented | async.test.ts |  |
| `napi_call_threadsafe_function` | 4 | node_api.h | implemented | async.test.ts, full.test.ts |  |
| `napi_create_threadsafe_function` | 4 | node_api.h | implemented | api.test.ts, async.test.ts, cleanup.test.ts, full.test.ts | the calls run on the JS thread between macrotasks; a referenced one keeps the event loop alive |
| `napi_get_threadsafe_function_context` | 4 | node_api.h | implemented | async.test.ts |  |
| `napi_ref_threadsafe_function` | 4 | node_api.h | implemented | async.test.ts |  |
| `napi_release_threadsafe_function` | 4 | node_api.h | implemented | async.test.ts | finalized while threads still hold it, it is freed when the last one lets go (Node frees it at finalization; their later calls are a use after free) |
| `napi_unref_threadsafe_function` | 4 | node_api.h | implemented | async.test.ts, cleanup.test.ts, full.test.ts |  |
| `napi_add_finalizer` | 5 | js_native_api.h | implemented | api.test.ts, typetag.test.ts, wrap.test.ts | an exception the finalizer leaves pending is uncaught (reported from the event loop) |
| `napi_create_date` | 5 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_date_value` | 5 | js_native_api.h | implemented | api.test.ts |  |
| `napi_is_date` | 5 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_bigint_int64` | 6 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_bigint_uint64` | 6 | js_native_api.h | implemented | api.test.ts |  |
| `napi_create_bigint_words` | 6 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_all_property_names` | 6 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_instance_data` | 6 | js_native_api.h | implemented | api.test.ts, cleanup.test.ts |  |
| `napi_get_value_bigint_int64` | 6 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_value_bigint_uint64` | 6 | js_native_api.h | implemented | api.test.ts |  |
| `napi_get_value_bigint_words` | 6 | js_native_api.h | implemented | api.test.ts |  |
| `napi_set_instance_data` | 6 | js_native_api.h | implemented | api.test.ts, cleanup.test.ts |  |
| `napi_detach_arraybuffer` | 7 | js_native_api.h | implemented | api.test.ts |  |
| `napi_is_detached_arraybuffer` | 7 | js_native_api.h | implemented | api.test.ts |  |
| `napi_check_object_type_tag` | 8 | js_native_api.h | implemented | api.test.ts, typetag.test.ts |  |
| `napi_object_freeze` | 8 | js_native_api.h | implemented | api.test.ts |  |
| `napi_object_seal` | 8 | js_native_api.h | implemented | api.test.ts |  |
| `napi_type_tag_object` | 8 | js_native_api.h | implemented | api.test.ts, typetag.test.ts |  |
| `napi_add_async_cleanup_hook` | 8 | node_api.h | implemented | api.test.ts, cleanup.test.ts | the hook is called at teardown; the host does not wait for an asynchronous removal |
| `napi_remove_async_cleanup_hook` | 8 | node_api.h | implemented | cleanup.test.ts |  |
| `node_api_create_syntax_error` | 9 | js_native_api.h | implemented | api.test.ts |  |
| `node_api_symbol_for` | 9 | js_native_api.h | implemented | api.test.ts |  |
| `node_api_throw_syntax_error` | 9 | js_native_api.h | implemented | api.test.ts |  |
| `node_api_get_module_file_name` | 9 | node_api.h | implemented | api.test.ts |  |
| `node_api_create_external_string_latin1` | 10 | js_native_api.h | implemented | api.test.ts, cleanup.test.ts | external (`copied` false), as Node without the V8 sandbox |
| `node_api_create_external_string_utf16` | 10 | js_native_api.h | implemented | api.test.ts | external (`copied` false), as Node without the V8 sandbox |
| `node_api_create_property_key_latin1` | 10 | js_native_api.h | implemented | api.test.ts |  |
| `node_api_create_property_key_utf16` | 10 | js_native_api.h | implemented | api.test.ts |  |
| `node_api_create_property_key_utf8` | 10 | js_native_api.h | implemented | api.test.ts |  |
| `node_api_create_buffer_from_arraybuffer` | 10 | node_api.h | implemented | api.test.ts | a Buffer over the ArrayBuffer |

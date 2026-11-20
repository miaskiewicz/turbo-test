//! One snapshot family, with backend payload activation scoped to a selected context.
use std::cell::Cell;
use std::sync::{Mutex, OnceLock};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeProfile {
    Node,
    Dom,
}
thread_local! { static CURRENT: Cell<RuntimeProfile> = const { Cell::new(RuntimeProfile::Node) }; }
pub(crate) struct ProfileGuard(RuntimeProfile);
impl RuntimeProfile {
    pub(crate) fn from_dom(dom: bool) -> Self {
        if dom {
            Self::Dom
        } else {
            Self::Node
        }
    }
    pub(crate) fn enter(self) -> ProfileGuard {
        ProfileGuard(CURRENT.with(|current| current.replace(self)))
    }
}
impl Drop for ProfileGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.0));
    }
}
// A DOM context that activated the backend payload on first use is a Node context for the rest
// of the file (backend I/O pacing, transport cleanup); the ProfileGuard restores the outer value.
pub(crate) fn activate_node() {
    CURRENT.with(|current| current.set(RuntimeProfile::Node));
}
pub(crate) fn is_node() -> bool {
    CURRENT.with(|current| current.get() == RuntimeProfile::Node)
}

// Cached data contains no persistent V8 handles. Every context uses the process's one
// immutable build/flags/snapshot family; V8 rejects incompatible bytecode automatically.
fn compile_backend<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: v8::Local<'s, v8::String>,
    bytes: &[u8],
) -> v8::Local<'s, v8::Script> {
    use v8::script_compiler as sc;
    let mut source = sc::Source::new_with_cached_data(code, None, v8::CachedData::new(bytes));
    sc::compile(
        scope,
        &mut source,
        sc::CompileOptions::ConsumeCodeCache,
        sc::NoCacheReason::NoReason,
    )
    .expect("backend runtime compilation")
}
pub(crate) fn install_backend(scope: &mut v8::PinScope) {
    static SOURCE: OnceLock<String> = OnceLock::new();
    static CACHE: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    let source = SOURCE.get_or_init(crate::runtime_sources::backend_runtime_source);
    let code = v8::String::new(scope, source).unwrap();
    let bytes = CACHE.lock().unwrap().clone();
    let script = if bytes.is_empty() {
        v8::Script::compile(scope, code, None).expect("backend runtime compilation")
    } else {
        compile_backend(scope, code, &bytes)
    };
    script.run(scope).expect("backend runtime initialization");
    if let Some(cache) = script.get_unbound_script(scope).create_code_cache() {
        *CACHE.lock().unwrap() = cache.to_vec();
    }
}
pub(crate) fn install_dom(scope: &mut v8::PinScope) {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "__ttRuntimeDomInstalled").unwrap();
    if global
        .get(scope, key.into())
        .is_some_and(|value| value.is_true())
    {
        return;
    }
    let code = v8::String::new(scope, crate::runtime_sources::dom_runtime_source()).unwrap();
    v8::Script::compile(scope, code, None)
        .expect("DOM runtime compilation")
        .run(scope)
        .expect("DOM runtime initialization");
    let installed = v8::Boolean::new(scope, true);
    global.set(scope, key.into(), installed.into());
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_profile_guards_restore_the_owner() {
        let _node = RuntimeProfile::Node.enter();
        assert!(is_node());
        {
            let _dom = RuntimeProfile::Dom.enter();
            assert!(!is_node());
            {
                let _inner = RuntimeProfile::Node.enter();
                assert!(is_node());
            }
            assert!(!is_node());
        }
        assert!(is_node());
        assert_eq!(RuntimeProfile::from_dom(true), RuntimeProfile::Dom);
    }
}

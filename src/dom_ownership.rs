//! Weak native lookups with V8-private document and subtree ownership.
use super::*;
#[path = "realm_ownership.rs"]
mod graph;
pub(super) type Graph = std::rc::Rc<std::cell::RefCell<graph::DocumentGraph>>;

pub(super) fn helper_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'_, v8::Function>,
) -> Option<v8::Weak<v8::Function>> {
    let function = v8::Local::new(scope, function);
    retain_helper(scope, function.into())?;
    Some(v8::Weak::new(scope, function))
}
pub(super) fn helper_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<v8::Weak<v8::Object>> {
    let object = v8::Local::new(scope, object);
    retain_helper(scope, object.into())?;
    Some(v8::Weak::new(scope, object))
}
fn retain_helper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<()> {
    if scope.is_execution_terminating() {
        return None;
    }
    let graph = DOM.with(|state| state.borrow().as_ref()?.ownership.clone())?;
    // V8 may materialize termination while allocating the private edge. Keep
    // that exception pending and let the installer abort without publishing it.
    let retained = graph.borrow_mut().retain_helper(scope, value);
    retained
}
pub(super) trait NativeHandle<T> {
    fn local<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>>;
}
impl<T> NativeHandle<T> for &v8::Global<T> {
    fn local<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        Some(v8::Local::new(scope, self))
    }
}
impl<T> NativeHandle<T> for v8::Global<T> {
    fn local<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        Some(v8::Local::new(scope, self))
    }
}
impl<T> NativeHandle<T> for &v8::Weak<T> {
    fn local<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        self.to_local(scope)
    }
}
impl<T> NativeHandle<T> for v8::Weak<T> {
    fn local<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, T>> {
        self.to_local(scope)
    }
}
pub(super) fn local<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: impl NativeHandle<T>,
) -> v8::Local<'s, T> {
    handle
        .local(scope)
        .expect("native caller retains its document owner")
}
pub(super) fn observe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: Handle,
    object: v8::Local<'s, v8::Object>,
) {
    let root = with_tree(Tree::root).unwrap();
    let graph = DOM.with(|state| {
        state
            .borrow()
            .as_ref()
            .and_then(|state| state.ownership.clone())
    });
    let graph = match graph {
        Some(graph) => graph,
        None => {
            if handle != root {
                return;
            }
            let graph = std::rc::Rc::new(std::cell::RefCell::new(
                graph::DocumentGraph::new(scope, object).unwrap(),
            ));
            DOM.with(|state| state.borrow_mut().as_mut().unwrap().ownership = Some(graph.clone()));
            graph
        }
    };
    let composed = with_tree(|tree| {
        let mut root = handle;
        while let Some(parent) = tree
            .parent(root)
            .or_else(|| tree.attribute_owner(root))
            .or_else(|| tree.shadow_host(root))
        {
            root = parent;
        }
        root
    })
    .unwrap();
    graph.borrow_mut().observe_component(
        scope,
        handle.raw(),
        object,
        composed.raw(),
        composed == root,
    );
}
pub(super) fn reconcile_current(scope: &mut v8::PinScope) {
    let Some((graph, roots, connected)) = DOM.with(|state| {
        let state = state.borrow();
        let state = state.as_ref()?;
        let graph = state.ownership.clone()?;
        let mut roots = std::collections::HashMap::new();
        let mut connected = std::collections::HashSet::new();
        for handle in state.cache.keys() {
            let mut root = *handle;
            while let Some(parent) = state
                .tree
                .parent(root)
                .or_else(|| state.tree.attribute_owner(root))
                .or_else(|| state.tree.shadow_host(root))
            {
                root = parent;
            }
            roots.insert(handle.raw(), root.raw());
            if root == state.tree.root() {
                connected.insert(handle.raw());
            }
        }
        Some((graph, roots, connected))
    }) else {
        return;
    };
    let mut graph = graph.borrow_mut();
    if !graph.document_alive() {
        return;
    }
    graph.sweep();
    if graph.reconcile(scope, &connected).is_none() {
        return;
    }
    graph.reconcile_components(scope, |id| *roots.get(&id).unwrap());
}

pub(super) fn link_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    let name = v8::String::new(scope, "turbo-dom.ownership.inline-style").unwrap();
    let key = v8::Private::for_api(scope, Some(name));
    node.set_private(scope, key, style.into());
    let name = v8::String::new(scope, "turbo-dom.ownership.style-owner").unwrap();
    let key = v8::Private::for_api(scope, Some(name));
    style.set_private(scope, key, node.into());
}
pub(super) fn pin_callback_owners<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    arguments: impl IntoIterator<Item = v8::Local<'s, v8::Value>>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let current = DOM_ID.with(|id| id.get());
    let mut owners = std::collections::HashSet::from([current]);
    for value in std::iter::once(receiver.into()).chain(arguments) {
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        if object.internal_field_count() != 2 || handle_of(scope, object).is_none() {
            continue;
        }
        if let Some(id) = object
            .get_internal_field(scope, 1)
            .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        {
            owners.insert(id.u64_value().0);
        }
    }
    let mut pins = Vec::with_capacity(owners.len());
    for owner in owners {
        let document = if owner == current {
            DOM.with(|state| {
                let state = state.borrow();
                let state = state.as_ref()?;
                state.cache.get(&state.tree.root())?.to_local(scope)
            })
        } else {
            OTHER_DOMS.with(|states| {
                let states = states.borrow();
                let state = states.get(&owner)?.dom.as_ref()?;
                state.cache.get(&state.tree.root())?.to_local(scope)
            })
        };
        if let Some(document) = document {
            pins.push(document);
        }
    }
    pins
}

/// Above this many live wrappers a mutating DOM call stops reconciling ownership on its own.
/// A reconcile is O(wrappers) in Rust and allocates a V8 Map per ownership component, so
/// reconciling after EVERY call made a render that touches n nodes cost O(n^2): one React file
/// with a few thousand nodes spent >90% of its time here (profiled with `sample`). Small trees
/// (every GC-exactness test) stay on the exact per-call path.
const EAGER_WRAPPER_LIMIT: usize = 256;
/// Past the limit, reconcile once per `wrappers / DEFER_DIVISOR` mutating calls: amortised O(1)
/// ownership work per call instead of O(wrappers).
const DEFER_DIVISOR: usize = 32;
/// Bound on wrappers pinned while reconciles are deferred (see `reconcile_after_call`).
const MAX_PINNED: usize = 4096;

thread_local! {
    static DEFERRED_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Receivers/arguments of calls whose reconcile was deferred. Their wrappers (and, through the
    /// component edges, every wrapper of the subtree they were last known to own) stay strongly
    /// reachable until the pending reconcile has re-derived ownership, so a node that was just
    /// attached cannot lose its wrapper (and expandos, e.g. React's fiber pointers) to a GC that
    /// lands between the mutation and the reconcile.
    static DEFERRED_PINS: std::cell::RefCell<Vec<v8::Global<v8::Object>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Epilogue of every mutating native call: reconcile now (small trees, exact), or batch.
pub(super) fn reconcile_after_call<'a>(
    scope: &mut v8::PinScope,
    objects: impl IntoIterator<Item = v8::Local<'a, v8::Value>>,
) {
    let wrappers = DOM.with(|state| {
        let state = state.borrow();
        let state = state.as_ref()?;
        state.ownership_dirty.then_some(state.cache.len())
    });
    let Some(wrappers) = wrappers.filter(|n| *n > EAGER_WRAPPER_LIMIT) else {
        return flush(scope);
    };
    let pinned = DEFERRED_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        for value in objects {
            if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
                pins.push(v8::Global::new(scope, object));
            }
        }
        pins.len()
    });
    let calls = DEFERRED_CALLS.with(|calls| {
        calls.set(calls.get() + 1);
        calls.get()
    });
    if calls >= wrappers / DEFER_DIVISOR || pinned >= MAX_PINNED {
        flush(scope);
    }
}

/// Drop the deferral state with the DOM it belonged to (`browser_env::reset`).
pub(super) fn forget_deferred() {
    let _ = DEFERRED_CALLS.try_with(|calls| calls.set(0));
    let _ = DEFERRED_PINS.try_with(|pins| pins.borrow_mut().clear());
}

/// Reconcile ownership immediately and release the deferral pins. The event loop calls this
/// between tasks so nothing stays deferred once control leaves a synchronous run of DOM calls.
pub(super) fn flush(scope: &mut v8::PinScope) {
    reconcile_all(scope);
    DEFERRED_CALLS.with(|calls| calls.set(0));
    DEFERRED_PINS.with(|pins| pins.borrow_mut().clear());
}

thread_local! {
    static GC_TRACKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static GC_EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SWEPT_EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

unsafe extern "C" fn gc_epilogue(
    _isolate: v8::UnsafeRawIsolatePtr,
    _type: v8::GCType,
    _flags: v8::GCCallbackFlags,
    _data: *mut std::ffi::c_void,
) {
    GC_EPOCH.with(|epoch| epoch.set(epoch.get() + 1));
}

/// Let `reconcile_all` skip its collected-listener/document sweeps until a GC has run. Those
/// sweeps look for weak handles V8 has cleared, which only happens during a GC, yet they ran after
/// every native call and walk every registered listener: `sweep_collected_listeners` was the top
/// Rust frame when profiling a React suite. Call once per isolate; an isolate that never calls it
/// keeps sweeping every time (what embedders' GC-exactness tests rely on).
pub(super) fn track_gc(isolate: &mut v8::Isolate) {
    isolate.add_gc_epilogue_callback(gc_epilogue, std::ptr::null_mut(), v8::GCType::kGCTypeAll);
    GC_TRACKED.with(|tracked| tracked.set(true));
}

/// False only when GC tracking is on and no GC has run since the last sweep.
fn collected_since_sweep() -> bool {
    !GC_TRACKED.with(|tracked| tracked.get())
        || GC_EPOCH.with(|epoch| epoch.get()) != SWEPT_EPOCH.with(|swept| swept.get())
}

pub(super) fn reconcile_all(scope: &mut v8::PinScope) {
    if scope.is_execution_terminating() {
        return;
    }
    if collected_since_sweep() {
        SWEPT_EPOCH.with(|swept| swept.set(GC_EPOCH.with(|epoch| epoch.get())));
        super::event_listeners::sweep_collected_listeners(scope);
        sweep_collected();
    }
    let current = DOM_ID.with(|id| id.get());
    let mut owners = OTHER_DOMS.with(|states| {
        states
            .borrow()
            .iter()
            .filter(|(_, state)| state.dom.as_ref().is_some_and(|dom| dom.ownership_dirty))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
    });
    if DOM.with(|state| {
        state
            .borrow()
            .as_ref()
            .is_some_and(|state| state.ownership_dirty)
    }) {
        owners.push(current);
    }
    for owner in owners {
        let _dom = DomScope::enter(owner);
        reconcile_current(scope);
        DOM.with(|state| {
            if let Some(state) = state.borrow_mut().as_mut() {
                state.ownership_dirty = false;
            }
        });
    }
}

fn listener_roots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Map>> {
    let name = v8::String::new(scope, "turbo-dom.ownership.node-listeners")?;
    let key = v8::Private::for_api(scope, Some(name));
    if let Some(map) = node
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
    {
        return Some(map);
    }
    let map = v8::Map::new(scope);
    (node.set_private(scope, key, map.into()) == Some(true)).then_some(map)
}
pub(super) fn retain_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    id: u32,
    record: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let roots = listener_roots(scope, node)?;
    let id = v8::Integer::new_from_unsigned(scope, id);
    roots.set(scope, id.into(), record.into())?;
    Some(())
}
pub(super) fn release_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    id: u32,
) {
    if let Some(roots) = listener_roots(scope, node) {
        let id = v8::Integer::new_from_unsigned(scope, id);
        roots.delete(scope, id.into());
    }
}

pub(super) fn sweep_collected() -> usize {
    let stale = OTHER_DOMS.with(|states| {
        states
            .borrow()
            .iter()
            .filter(|(_, state)| {
                state
                    .dom
                    .as_ref()
                    .and_then(|dom| dom.ownership.as_ref())
                    .is_some_and(|graph| !graph.borrow().document_alive())
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
    });
    OTHER_DOMS.with(|states| {
        let mut states = states.borrow_mut();
        for id in &stale {
            states.remove(id);
        }
    });
    DOM_CONTEXTS.with(|contexts| {
        let mut contexts = contexts.borrow_mut();
        for id in &stale {
            contexts.remove(id);
        }
    });
    stale.len()
}

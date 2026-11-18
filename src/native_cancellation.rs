//! Owner-signalled cancellation for native loops that cannot service V8 interrupts.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Default)]
struct State {
    vm_timeouts: AtomicUsize,
    heap_limit: AtomicBool,
}
#[derive(Clone, Default)]
pub(crate) struct NativeCancellation(Arc<State>);
impl NativeCancellation {
    pub(crate) fn cancelled(&self) -> bool {
        self.0.heap_limit.load(Ordering::Acquire) || self.0.vm_timeouts.load(Ordering::Acquire) != 0
    }
    pub(crate) fn request_vm_timeout(&self) {
        self.0.vm_timeouts.fetch_add(1, Ordering::AcqRel);
    }
    pub(crate) fn finish_vm_timeout(&self) {
        self.0.vm_timeouts.fetch_sub(1, Ordering::AcqRel);
    }
    pub(crate) fn request_heap_limit(&self) {
        self.0.heap_limit.store(true, Ordering::Release);
    }
}
pub(crate) fn for_isolate(isolate: &mut v8::Isolate) -> NativeCancellation {
    if let Some(value) = isolate.get_slot::<NativeCancellation>() {
        return value.clone();
    }
    let value = NativeCancellation::default();
    isolate.set_slot(value.clone());
    value
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeout_owners_do_not_clear_each_other_or_heap_termination() {
        let cancellation = NativeCancellation::default();
        assert!(!cancellation.cancelled());
        cancellation.request_vm_timeout();
        cancellation.request_vm_timeout();
        cancellation.finish_vm_timeout();
        assert!(cancellation.cancelled());
        cancellation.finish_vm_timeout();
        assert!(!cancellation.cancelled());
        cancellation.request_vm_timeout();
        cancellation.request_heap_limit();
        cancellation.finish_vm_timeout();
        assert!(cancellation.cancelled());
    }
}

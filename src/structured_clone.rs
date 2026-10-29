//! `structuredClone` for the test runtime (runtime.js calls `__ttStructuredClone(value,
//! transfer)`): V8's ValueSerializer / ValueDeserializer, the HTML structured clone algorithm
//! Node's structuredClone runs too (Maps, Sets, Dates, RegExps, errors, typed arrays, cycles,
//! ...). ArrayBuffers in the transfer list move to the clone (the originals are detached);
//! SharedArrayBuffers are shared, not copied. What cannot be cloned or transferred throws a
//! DOMException named DataCloneError with Node's message.

use std::cell::RefCell;
use std::rc::Rc;
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

/// The SharedArrayBuffers met while serializing, by id, for the deserializer to share.
type Shared = Rc<RefCell<Vec<v8::Global<v8::SharedArrayBuffer>>>>;

struct Serializer(Shared);

impl v8::ValueSerializerImpl for Serializer {
    fn throw_data_clone_error<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        message: v8::Local<'s, v8::String>,
    ) {
        throw_data_clone_error(scope, message);
    }

    fn get_shared_array_buffer_id<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        sab: v8::Local<'s, v8::SharedArrayBuffer>,
    ) -> Option<u32> {
        let mut sabs = self.0.borrow_mut();
        sabs.push(v8::Global::new(scope, sab));
        Some(sabs.len() as u32 - 1)
    }
}

struct Deserializer(Shared);

impl v8::ValueDeserializerImpl for Deserializer {
    fn get_shared_array_buffer_from_id<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        transfer_id: u32,
    ) -> Option<v8::Local<'s, v8::SharedArrayBuffer>> {
        let sabs = self.0.borrow();
        Some(v8::Local::new(scope, &sabs[transfer_id as usize]))
    }
}

/// Throw `new DOMException(message, 'DataCloneError')`, what Node throws.
fn throw_data_clone_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: v8::Local<'s, v8::String>,
) {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "DOMException").unwrap();
    let name = v8::String::new(scope, "DataCloneError").unwrap();
    let ctor = global
        .get(scope, key.into())
        .and_then(|c| v8::Local::<v8::Function>::try_from(c).ok());
    let exc = ctor
        .and_then(|c| c.new_instance(scope, &[message.into(), name.into()]))
        .map_or_else(|| v8::Exception::error(scope, message), Into::into);
    scope.throw_exception(exc);
}

fn throw_message(scope: &mut v8::PinScope, message: &str) {
    let message = v8::String::new(scope, message).unwrap();
    throw_data_clone_error(scope, message);
}

/// The transfer list's ArrayBuffers, checked as Node checks them: anything else (a
/// SharedArrayBuffer included) is "Found invalid value in transferList.", a repeat "Transfer
/// list contains duplicate ArrayBuffer", a detached or non-detachable buffer "Cannot transfer
/// object of unsupported type." None (thrown) on the first bad one.
fn transfer_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Array>,
) -> Option<Vec<v8::Local<'s, v8::ArrayBuffer>>> {
    let mut out: Vec<v8::Local<v8::ArrayBuffer>> = Vec::new();
    for i in 0..list.length() {
        let item = list.get_index(scope, i)?;
        let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(item) else {
            throw_message(scope, "Found invalid value in transferList.");
            return None;
        };
        if out.iter().any(|seen| seen.strict_equals(ab.into())) {
            throw_message(scope, "Transfer list contains duplicate ArrayBuffer");
            return None;
        }
        if !ab.is_detachable() || ab.was_detached() {
            throw_message(scope, "Cannot transfer object of unsupported type.");
            return None;
        }
        out.push(ab);
    }
    Some(out)
}

/// `__ttStructuredClone(value, transfer)`: the structured clone of `value`, the ArrayBuffers of
/// the `transfer` array moved into it.
pub fn native_structured_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let value = args.get(0);
    let list =
        v8::Local::<v8::Array>::try_from(args.get(1)).unwrap_or_else(|_| v8::Array::new(scope, 0));
    let Some(transfer) = transfer_list(scope, list) else {
        return;
    };
    let context = scope.get_current_context();
    let shared = Shared::default();
    let serializer = v8::ValueSerializer::new(scope, Box::new(Serializer(shared.clone())));
    serializer.write_header();
    for (id, ab) in transfer.iter().enumerate() {
        serializer.transfer_array_buffer(id as u32, *ab);
    }
    if serializer.write_value(context, value) != Some(true) {
        return; // the DataCloneError is thrown
    }
    let data = serializer.release();
    // the transferred memory moves to new ArrayBuffers; the originals are detached
    let moved: Vec<v8::Local<v8::ArrayBuffer>> = transfer
        .iter()
        .map(|ab| {
            let store = ab.get_backing_store();
            ab.detach(None);
            v8::ArrayBuffer::with_backing_store(scope, &store)
        })
        .collect();
    let deserializer = v8::ValueDeserializer::new(scope, Box::new(Deserializer(shared)), &data);
    deserializer.read_header(context);
    for (id, ab) in moved.into_iter().enumerate() {
        deserializer.transfer_array_buffer(id as u32, ab);
    }
    if let Some(clone) = deserializer.read_value(context) {
        rv.set(clone);
    }
}

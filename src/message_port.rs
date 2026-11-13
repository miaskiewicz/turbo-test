//! Tagged V8 MessagePort handles and transactional structured-clone host-object transfer.
//! Endpoint queues live in the owning JS context; no persistent Rust handles outlive a clone.
use std::cell::RefCell;
use std::rc::Rc;
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

static PORT_TAG: u8 = 0;

fn tag_pointer() -> *mut std::ffi::c_void {
    (&PORT_TAG as *const u8).cast_mut().cast()
}

pub fn is_port(scope: &v8::PinScope, value: v8::Local<v8::Value>) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    if object.internal_field_count() != 5 {
        return false;
    }
    object
        .get_internal_field(scope, 0)
        .and_then(|value| v8::Local::<v8::External>::try_from(value).ok())
        .is_some_and(|value| value.value() == tag_pointer())
}

fn port_object<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !is_port(scope, value) {
        return None;
    }
    v8::Local::<v8::Object>::try_from(value).ok()
}

fn field<'s>(
    scope: &v8::PinScope<'s, '_>,
    port: v8::Local<v8::Object>,
    index: usize,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::Local::<v8::Value>::try_from(port.get_internal_field(scope, index)?).ok()
}

fn throw_type(scope: &mut v8::PinScope, message: &str) {
    let message = v8::String::new(scope, message).unwrap();
    let error = v8::Exception::type_error(scope, message);
    scope.throw_exception(error);
}

pub fn throw_clone(scope: &mut v8::PinScope, message: &str) {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "DOMException").unwrap();
    let name = v8::String::new(scope, "DataCloneError").unwrap();
    let message = v8::String::new(scope, message).unwrap();
    let ctor = global
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    let error = ctor
        .and_then(|ctor| ctor.new_instance(scope, &[message.into(), name.into()]))
        .map_or_else(|| v8::Exception::error(scope, message), Into::into);
    scope.throw_exception(error);
}

fn make_port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: v8::Local<'s, v8::Value>,
    prototype: v8::Local<'s, v8::Value>,
    commit: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(5);
    let object = template.new_instance(scope)?;
    let tag = v8::External::new(scope, tag_pointer());
    let detached = v8::Boolean::new(scope, false);
    object.set_internal_field(0, tag.into());
    object.set_internal_field(1, id.into());
    object.set_internal_field(2, detached.into());
    object.set_internal_field(3, commit.into());
    object.set_internal_field(4, prototype.into());
    object.set_prototype(scope, prototype)?;
    Some(object)
}

fn copy_port<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    old: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = field(scope, old, 1)?;
    let commit = field(scope, old, 3)?;
    let prototype = field(scope, old, 4)?;
    make_port(scope, id, prototype, commit)
}

pub fn native_create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    if !args.get(0).is_uint32() || !args.get(1).is_object() || !args.get(2).is_function() {
        return throw_type(scope, "Invalid MessagePort construction");
    }
    if let Some(port) = make_port(scope, args.get(0), args.get(1), args.get(2)) {
        rv.set(port.into());
    }
}

pub fn native_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let Some(port) = port_object(scope, args.get(0)) else {
        return throw_type(scope, "Invalid MessagePort receiver");
    };
    rv.set(field(scope, port, 1).unwrap());
}

pub fn native_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let Some(port) = port_object(scope, args.get(0)) else {
        return throw_type(scope, "Invalid MessagePort receiver");
    };
    rv.set(field(scope, port, 2).unwrap());
}

pub fn native_detach<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue,
) {
    let Some(port) = port_object(scope, args.get(0)) else {
        return throw_type(scope, "Invalid MessagePort receiver");
    };
    let detached = v8::Boolean::new(scope, true);
    port.set_internal_field(2, detached.into());
}

#[derive(Default)]
pub struct PortTransfers {
    source: RefCell<Vec<v8::Global<v8::Object>>>,
    moved: RefCell<Vec<Option<v8::Global<v8::Object>>>>,
}

impl PortTransfers {
    pub fn add(&self, scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Option<()> {
        let port = port_object(scope, value)?;
        if field(scope, port, 2)?.boolean_value(scope) {
            throw_clone(scope, "MessagePort in transfer list is already detached");
            return None;
        }
        let mut source = self.source.borrow_mut();
        if source
            .iter()
            .any(|old| v8::Local::new(scope, old).strict_equals(port.into()))
        {
            throw_clone(scope, "Transfer list contains duplicate MessagePort");
            return None;
        }
        source.push(v8::Global::new(scope, port));
        self.moved.borrow_mut().push(None);
        Some(())
    }

    pub fn write<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        port: v8::Local<'s, v8::Object>,
        serializer: &dyn ValueSerializerHelper,
    ) -> Option<bool> {
        if !is_port(scope, port.into()) {
            throw_clone(scope, "Object could not be cloned");
            return None;
        }
        if field(scope, port, 2)?.boolean_value(scope) {
            throw_clone(scope, "MessagePort in transfer list is already detached");
            return None;
        }
        let source = self.source.borrow();
        let index = source
            .iter()
            .position(|old| v8::Local::new(scope, old).strict_equals(port.into()));
        let Some(index) = index else {
            throw_clone(scope, "MessagePort must be listed in transferList");
            return None;
        };
        serializer.write_uint32(index as u32);
        Some(true)
    }

    pub fn read<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        deserializer: &dyn ValueDeserializerHelper,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let mut index = 0;
        if !deserializer.read_uint32(&mut index) {
            return None;
        }
        self.destination(scope, index as usize)
    }

    fn destination<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        index: usize,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if let Some(port) = self.moved.borrow().get(index)?.as_ref() {
            return Some(v8::Local::new(scope, port));
        }
        let source = self.source.borrow();
        let old = v8::Local::new(scope, source.get(index)?);
        let port = copy_port(scope, old)?;
        self.moved.borrow_mut()[index] = Some(v8::Global::new(scope, port));
        Some(port)
    }

    pub fn commit<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        capture: v8::Local<'s, v8::Value>,
    ) -> Option<()> {
        self.validate_sources(scope)?;
        let length = self.source.borrow().len();
        if length == 0 {
            return Some(());
        }
        let (before, after) = self.transfer_arrays(scope, length)?;
        self.commit_callback(scope, before, after)?;
        capture_transferred(scope, capture, after)
    }

    fn transfer_arrays<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        length: usize,
    ) -> Option<(v8::Local<'s, v8::Array>, v8::Local<'s, v8::Array>)> {
        let before = v8::Array::new(scope, length as i32);
        let after = v8::Array::new(scope, length as i32);
        for index in 0..length {
            let destination = self.destination(scope, index)?;
            let source = v8::Local::new(scope, &self.source.borrow()[index]);
            set_index_own(scope, before, index as u32, source.into())?;
            set_index_own(scope, after, index as u32, destination.into())?;
        }
        Some((before, after))
    }

    fn validate_sources(&self, scope: &mut v8::PinScope) -> Option<()> {
        for source in self.source.borrow().iter() {
            let old = v8::Local::new(scope, source);
            if field(scope, old, 2)?.boolean_value(scope) {
                throw_clone(scope, "MessagePort in transfer list is already detached");
                return None;
            }
        }
        Some(())
    }

    fn commit_callback<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        before: v8::Local<'s, v8::Array>,
        after: v8::Local<'s, v8::Array>,
    ) -> Option<()> {
        let old = v8::Local::new(scope, &self.source.borrow()[0]);
        let callback = v8::Local::<v8::Function>::try_from(field(scope, old, 3)?).ok()?;
        let receiver = v8::undefined(scope);
        callback.call(scope, receiver.into(), &[before.into(), after.into()])?;
        let detached = v8::Boolean::new(scope, true);
        for old in self.source.borrow().iter() {
            v8::Local::new(scope, old).set_internal_field(2, detached.into());
        }
        Some(())
    }
}

fn set_index_own<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
) -> Option<()> {
    let key = v8::String::new(scope, &index.to_string()).unwrap();
    array.create_data_property(scope, key.into(), value)?;
    Some(())
}

fn capture_transferred<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    capture: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
) -> Option<()> {
    let Ok(capture) = v8::Local::<v8::Array>::try_from(capture) else {
        return Some(());
    };
    for index in 0..ports.length() {
        let port = ports.get_index(scope, index)?;
        set_index_own(scope, capture, index, port)?;
    }
    Some(())
}

pub type SharedPorts = Rc<PortTransfers>;

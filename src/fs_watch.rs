//! Context-owned OS filesystem watchers used by the Node fs.watch shim.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde_json::{json, Value};

struct Handle {
    _watcher: notify::RecommendedWatcher,
    events: Receiver<notify::Result<Event>>,
    path: PathBuf,
    directory: bool,
    overflow: Arc<AtomicBool>,
}
#[derive(Default)]
struct Table {
    next: u64,
    handles: HashMap<u64, Handle>,
}
thread_local! { static TABLE: RefCell<Table> = RefCell::new(Table::default()); }

pub fn reset() {
    TABLE.with(|table| table.borrow_mut().handles.clear());
}

fn open(args: &Value) -> Result<Value, String> {
    let requested = PathBuf::from(args["path"].as_str().ok_or("Missing watch path")?);
    let path = crate::canon::canonicalize(&requested).map_err(|error| error.to_string())?;
    let directory = path.is_dir();
    let (sender, events) = mpsc::sync_channel(1024);
    let overflow = Arc::new(AtomicBool::new(false));
    let flag = overflow.clone();
    let mut watcher = notify::recommended_watcher(move |event| publish(&sender, &flag, event))
        .map_err(|error| error.to_string())?;
    let mode = if args["recursive"].as_bool().unwrap_or(false) {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher
        .watch(&path, mode)
        .map_err(|error| error.to_string())?;
    TABLE.with(|table| {
        let mut table = table.borrow_mut();
        table.next += 1;
        let id = table.next;
        table.handles.insert(
            id,
            Handle {
                _watcher: watcher,
                events,
                path,
                directory,
                overflow,
            },
        );
        Ok(json!(id))
    })
}

fn publish(
    sender: &SyncSender<notify::Result<Event>>,
    overflow: &AtomicBool,
    event: notify::Result<Event>,
) {
    if matches!(sender.try_send(event), Err(TrySendError::Full(_))) {
        overflow.store(true, Ordering::Relaxed);
    }
}

fn filename(handle: &Handle, path: &Path) -> Option<String> {
    if handle.directory {
        return path
            .strip_prefix(&handle.path)
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned());
    }
    handle
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn event_kind(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::Create(_)
        | EventKind::Remove(_)
        | EventKind::Modify(notify::event::ModifyKind::Name(_)) => "rename",
        _ => "change",
    }
}

fn poll(args: &Value) -> Result<Value, String> {
    let id = args["id"].as_u64().ok_or("Missing watch id")?;
    TABLE.with(|table| {
        let table = table.borrow();
        let handle = table.handles.get(&id).ok_or("Watcher is closed")?;
        if handle.overflow.swap(false, Ordering::Relaxed) {
            return Err("Filesystem watch event queue overflowed".into());
        }
        let mut values = Vec::new();
        if let Ok(event) = handle.events.recv_timeout(Duration::from_millis(5)) {
            values.push(event);
        }
        values.extend(handle.events.try_iter().take(255));
        let mut out = Vec::new();
        for event in values {
            append_event(handle, event.map_err(|error| error.to_string())?, &mut out);
        }
        Ok(json!(out))
    })
}

fn append_event(handle: &Handle, event: Event, out: &mut Vec<Value>) {
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    if event.paths.is_empty() {
        out.push(json!({"event": event_kind(&event.kind), "filename": null}));
    }
    for path in &event.paths {
        out.push(json!({"event": event_kind(&event.kind), "filename": filename(handle, path)}));
    }
}

fn operation(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "open" => open(args),
        "poll" => poll(args),
        "close" => {
            let id = args["id"].as_u64().ok_or("Missing watch id")?;
            TABLE.with(|table| table.borrow_mut().handles.remove(&id));
            Ok(Value::Null)
        }
        _ => Err("Unknown fs.watch operation".into()),
    }
}

pub fn native_watch(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let encoded = args.get(1).to_rust_string_lossy(scope);
    let result = serde_json::from_str(&encoded)
        .map_err(|e| e.to_string())
        .and_then(|args| operation(&name, &args));
    let value = match result {
        Ok(value) => json!({"value": value}),
        Err(message) => json!({"error": {"code":"EIO", "message": message}}),
    };
    if let Some(text) = v8::String::new(scope, &value.to_string()) {
        rv.set(text.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_queue_reports_overflow() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let overflow = AtomicBool::new(false);
        publish(&sender, &overflow, Ok(Event::new(EventKind::Any)));
        assert!(!overflow.load(Ordering::Relaxed));
        publish(&sender, &overflow, Ok(Event::new(EventKind::Any)));
        assert!(overflow.load(Ordering::Relaxed));
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn maps_events_and_rejects_invalid_operations() {
        assert_eq!(
            event_kind(&EventKind::Create(notify::event::CreateKind::Any)),
            "rename"
        );
        assert_eq!(
            event_kind(&EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any
            ))),
            "change"
        );
        assert!(operation("invalid", &json!({})).is_err());
        assert!(operation("poll", &json!({})).is_err());
        assert!(operation("poll", &json!({"id": u64::MAX})).is_err());
        assert!(operation("open", &json!({})).is_err());
        assert!(operation("close", &json!({})).is_err());
        assert!(operation("close", &json!({"id": u64::MAX})).is_ok());
        reset();
    }
}

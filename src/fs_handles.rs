//! Native halves of fs.promises.access / chmod / rename / open and its FileHandle
//! (turbo-test#420): real syscalls on real file descriptors, failing with Node's errors
//! (`ENOENT: no such file or directory, access '<path>'`, `EBADF: bad file descriptor, write`).
//! The JS side (runtime.js) validates arguments, tracks a handle's closed state and shapes the
//! results.
use crate::runner::{stat_fields, throw_fs_dest_error, throw_fs_error, throw_fs_error_at};
use std::fs::File;
use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, PermissionsExt};
use std::os::unix::io::FromRawFd;

fn text(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments, index: i32) -> String {
    args.get(index).to_rust_string_lossy(scope)
}

fn integer(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments, index: i32) -> i64 {
    args.get(index).integer_value(scope).unwrap_or(0)
}

/// Set the result of a path call, or throw Node's error for it.
fn path_result(scope: &mut v8::PinScope, result: std::io::Result<()>, syscall: &str, path: &str) {
    if let Err(error) = result {
        throw_fs_error(scope, &error, syscall, path);
    }
}

/// Set the result of a descriptor call (one number, or fstat's fields), or throw Node's error
/// for it (reported without a path).
fn fd_result(
    scope: &mut v8::PinScope,
    rv: &mut v8::ReturnValue,
    result: std::io::Result<Vec<f64>>,
    syscall: &str,
) {
    match result {
        Ok(values) if values.len() == 1 => rv.set_double(values[0]),
        Ok(values) => {
            let items: Vec<v8::Local<v8::Value>> = values
                .iter()
                .map(|v| v8::Number::new(scope, *v).into())
                .collect();
            rv.set(v8::Array::new_with_elements(scope, &items).into());
        }
        Err(error) => throw_fs_error_at(scope, &error, syscall, None),
    }
}

fn os_result(status: libc::c_int) -> std::io::Result<libc::c_int> {
    if status < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(status)
}

fn c_path(path: &str) -> std::ffi::CString {
    // the JS side rejects a path with a NUL byte before it gets here
    std::ffi::CString::new(std::ffi::OsStr::new(path).as_bytes()).unwrap_or_default()
}

/// native __fs_access(path, mode): access(2) with the R_OK / W_OK / X_OK bits
pub fn native_access(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let path = text(scope, &args, 0);
    let mode = integer(scope, &args, 1) as libc::c_int;
    let status = unsafe { libc::access(c_path(&path).as_ptr(), mode) };
    path_result(scope, os_result(status).map(drop), "access", &path);
}

/// native __fs_chmod(path, mode)
pub fn native_chmod(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let path = text(scope, &args, 0);
    let mode = std::fs::Permissions::from_mode(integer(scope, &args, 1) as u32);
    path_result(scope, std::fs::set_permissions(&path, mode), "chmod", &path);
}

/// native __fs_rename(from, to): Node's error names both paths
pub fn native_rename(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let from = text(scope, &args, 0);
    let to = text(scope, &args, 1);
    if let Err(error) = std::fs::rename(&from, &to) {
        throw_fs_dest_error(scope, &error, "rename", &from, &to);
    }
}

/// Node's stringToFlags: the open(2) flags of an `fs.open` flag string (None when invalid).
pub(crate) fn open_flags(flags: &str) -> Option<libc::c_int> {
    use libc::{O_APPEND, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_SYNC, O_TRUNC, O_WRONLY};
    let value = match flags {
        "r" => O_RDONLY,
        "rs" | "sr" => O_RDONLY | O_SYNC,
        "r+" => O_RDWR,
        "rs+" | "sr+" => O_RDWR | O_SYNC,
        "w" => O_TRUNC | O_CREAT | O_WRONLY,
        "wx" | "xw" => O_TRUNC | O_CREAT | O_WRONLY | O_EXCL,
        "w+" => O_TRUNC | O_CREAT | O_RDWR,
        "wx+" | "xw+" => O_TRUNC | O_CREAT | O_RDWR | O_EXCL,
        "a" => O_APPEND | O_CREAT | O_WRONLY,
        "ax" | "xa" => O_APPEND | O_CREAT | O_WRONLY | O_EXCL,
        "as" | "sa" => O_APPEND | O_CREAT | O_WRONLY | O_SYNC,
        "a+" => O_APPEND | O_CREAT | O_RDWR,
        "ax+" | "xa+" => O_APPEND | O_CREAT | O_RDWR | O_EXCL,
        "as+" | "sa+" => O_APPEND | O_CREAT | O_RDWR | O_SYNC,
        _ => return None,
    };
    Some(value)
}

/// native __fs_openFlags(flags): open_flags of a flag string, undefined when invalid
pub fn native_open_flags(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if let Some(flags) = open_flags(&text(scope, &args, 0)) {
        rv.set_int32(flags);
    }
}

/// native __fs_open(path, flags, mode) -> fd (close-on-exec, as libuv opens files)
pub fn native_open(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let path = text(scope, &args, 0);
    let flags = integer(scope, &args, 1) as libc::c_int | libc::O_CLOEXEC;
    let mode = integer(scope, &args, 2) as libc::c_uint;
    let fd = unsafe { libc::open(c_path(&path).as_ptr(), flags, mode) };
    match os_result(fd) {
        Ok(fd) => rv.set_int32(fd),
        Err(error) => throw_fs_error(scope, &error, "open", &path),
    }
}

/// The open descriptor `fd` (>= 0, checked by the JS side) as a File that is not closed on drop.
fn borrowed(fd: i64) -> ManuallyDrop<File> {
    // OwnedFd rejects -1; a negative or out-of-range descriptor fails its syscall with EBADF
    let fd = libc::c_int::try_from(fd)
        .ok()
        .filter(|fd| *fd >= 0)
        .unwrap_or(libc::c_int::MIN);
    ManuallyDrop::new(unsafe { File::from_raw_fd(fd) })
}

/// The bytes `[offset, offset + length)` of a byte view, clamped to the view (the JS side
/// reports Node's range errors first; the clamp keeps a direct native call in bounds). Not a
/// view: no bytes.
fn view_bytes<'a>(value: v8::Local<'a, v8::Value>, offset: usize, length: usize) -> &'a mut [u8] {
    let view = v8::Local::<v8::ArrayBufferView>::try_from(value).ok();
    let (data, size) = view.map_or((std::ptr::null_mut(), 0), |v| (v.data(), v.byte_length()));
    let start = offset.min(size);
    let length = length.min(size - start);
    if length == 0 {
        return &mut [];
    }
    unsafe { std::slice::from_raw_parts_mut(data.cast::<u8>().add(start), length) }
}

struct Transfer<'a> {
    file: ManuallyDrop<File>,
    bytes: &'a mut [u8],
    position: i64,
}

fn transfer<'a>(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments<'a>,
) -> Transfer<'a> {
    let offset = integer(scope, args, 2).max(0) as usize;
    let length = integer(scope, args, 3).max(0) as usize;
    Transfer {
        file: borrowed(integer(scope, args, 0)),
        bytes: view_bytes(args.get(1), offset, length),
        position: integer(scope, args, 4),
    }
}

/// native __fs_read(fd, view, offset, length, position) -> bytes read; position < 0 reads at
/// (and advances) the file position
pub fn native_read(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let t = transfer(scope, &args);
    let read = if t.position < 0 {
        (&*t.file).read(t.bytes)
    } else {
        t.file.read_at(t.bytes, t.position as u64)
    };
    fd_result(scope, &mut rv, read.map(|n| vec![n as f64]), "read");
}

/// native __fs_write(fd, view, offset, length, position) -> bytes written (one write, as Node)
pub fn native_write(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let t = transfer(scope, &args);
    let written = if t.position < 0 {
        (&*t.file).write(t.bytes)
    } else {
        t.file.write_at(t.bytes, t.position as u64)
    };
    fd_result(scope, &mut rv, written.map(|n| vec![n as f64]), "write");
}

/// native __fs_fdOp(op, fd, value): close / fstat / ftruncate / fsync / fdatasync / fchmod of an
/// open descriptor (the op is the syscall Node names); fstat returns fs_stat's fields
pub fn native_fd_op(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let op = text(scope, &args, 0);
    let fd = integer(scope, &args, 1);
    let value = integer(scope, &args, 2);
    let file = borrowed(fd);
    let none = |()| Vec::new();
    let result = match op.as_str() {
        "fstat" => file.metadata().map(|m| stat_fields(&m).to_vec()),
        "close" => os_result(unsafe { libc::close(fd as libc::c_int) }).map(|_| Vec::new()),
        "ftruncate" => file.set_len(value as u64).map(none),
        "fsync" => file.sync_all().map(none),
        "fdatasync" => file.sync_data().map(none),
        _ => file
            .set_permissions(PermissionsExt::from_mode(value as u32))
            .map(none),
    };
    fd_result(scope, &mut rv, result, &op);
}

#[cfg(test)]
#[path = "fs_handles_tests.rs"]
mod tests;

//! `files` and its handles, ported from `APIS/FilesAPI.java` and `simulation/FS/DiskSystem.java`.

use std::os::raw::c_int;
use std::path::PathBuf;

use super::{trace, *};
use crate::disk::Disk;
use crate::ffi::*;
use crate::filehandle::{FileHandle, Mode};
use crate::host::Host;
use crate::vm::host_of;

pub unsafe fn install(l: *mut lua_State) {
    lua_createtable(l, 0, 17);
    set_fn(l, "open", open);
    set_fn(l, "getChildren", get_children);
    set_fn(l, "getDisks", get_disks);
    set_fn(l, "getDiskID", get_disk_id);
    set_fn(l, "getNumberOfDisks", get_number_of_disks);
    set_fn(l, "getPartitions", get_partitions);
    set_fn(l, "getPartition", get_partition);
    set_fn(l, "createPartition", unsupported);
    set_fn(l, "deletePartition", unsupported);
    set_fn(l, "setPartitionHidden", unsupported);
    set_fn(l, "setPartitionReadOnly", unsupported);
    set_fn(l, "makeDir", make_dir);
    set_fn(l, "exists", exists);
    set_fn(l, "isFile", is_file);
    set_fn(l, "isDir", is_dir);
    set_fn(l, "delete", delete);
    set_fn(l, "getBootPath", get_boot_path);
    set_global_table(l, "files");
}

/// An absent id means the boot disk.
unsafe fn disk_at(l: *mut lua_State, host: &Host, idx: c_int) -> &Disk {
    match host.disks.get(opt_int(l, idx)) {
        Some(disk) => disk,
        None => raise(l, "Disk not found"),
    }
}

/// Resolves `path` on the addressed disk, or `None` for a `NullFilepath`.
unsafe fn resolve(l: *mut lua_State, disk_idx: c_int, path_idx: c_int) -> Option<(PathBuf, bool)> {
    let path = check_string(l, path_idx, "path");
    let host = host_of(l);
    let disk = disk_at(l, host, disk_idx);
    disk.resolve(&path).map(|r| (r.real, r.writable))
}

unsafe extern "C" fn open(l: *mut lua_State) -> c_int {
    let mode = Mode::parse(&opt_str(l, 2).map_or("r".to_string(), |b| {
        String::from_utf8_lossy(&b).into_owned()
    }));
    let resolved = resolve(l, 3, 1);

    // The order of these checks is `FilesAPI.open`'s.
    if resolved.is_none() {
        trace!(
            "files",
            "open({:?}) -> unresolved",
            check_string(l, 1, "path")
        );
    }
    let Some((real, writable)) = resolved else {
        if mode.invalid {
            raise(l, "Invalid open mode");
        }
        raise(
            l,
            if mode.create {
                "Access denied"
            } else {
                "Not a file"
            },
        );
    };
    if real.is_dir() {
        trace!(
            "files",
            "open({:?}) -> Not a file: is a directory",
            check_string(l, 1, "path")
        );
        raise(l, "Not a file");
    }
    if mode.invalid {
        raise(l, "Invalid open mode");
    }
    if !real.exists() && !mode.create {
        raise(l, "Not a file");
    }
    if mode.can_write && !writable {
        raise(l, "Access denied");
    }

    trace!(
        "files",
        "open({:?}, {:?}) -> ok",
        check_string(l, 1, "path"),
        mode
    );
    match FileHandle::open(&real, mode) {
        Ok(handle) => {
            let slot = host_of(l).add_handle(handle);
            trace!(
                "handles",
                "slot {slot}, {} allocated",
                host_of(l).handles.len()
            );
            push_handle(l, slot);
            1
        }
        Err(message) => raise(l, &message),
    }
}

/// A handle is a plain table of closures over its slot.
unsafe fn push_handle(l: *mut lua_State, slot: i64) {
    lua_createtable(l, 0, 5);
    set_bound_fn(l, "read", handle_read, slot);
    set_bound_fn(l, "write", handle_write, slot);
    set_bound_fn(l, "seek", handle_seek, slot);
    set_bound_fn(l, "flush", handle_flush, slot);
    set_bound_fn(l, "close", handle_close, slot);
}

unsafe fn handle_of(l: *mut lua_State) -> &'static mut FileHandle {
    let slot = bound_id(l);
    match host_of(l).handle_mut(slot) {
        Some(handle) => handle,
        None => raise(l, "Attempt to use a closed file"),
    }
}

/// `read()` is `read("l")`; a number reads that many bytes.
unsafe extern "C" fn handle_read(l: *mut lua_State) -> c_int {
    let numeric =
        (!is_absent(l, 1) && lua_type(l, 1) == LUA_TNUMBER).then(|| check_int(l, 1, "amount"));
    let format = match numeric {
        Some(_) => None,
        None => Some(opt_str(l, 1).map_or("l".to_string(), |b| {
            String::from_utf8_lossy(&b).into_owned()
        })),
    };
    let handle = handle_of(l);
    let read = match (numeric, &format) {
        (Some(n), _) => handle.read_amount(n),
        (None, Some(f)) => handle.read_format(f),
        (None, None) => unreachable!(),
    };
    match read {
        Ok(Some(bytes)) => push_bytes(l, &bytes),
        Ok(None) => lua_pushnil(l),
        Err(message) => raise(l, &message),
    }
    1
}

/// A number writes a single byte.
unsafe extern "C" fn handle_write(l: *mut lua_State) -> c_int {
    let data = if lua_type(l, 1) == LUA_TNUMBER {
        vec![check_int(l, 1, "byte") as u8]
    } else {
        check_str(l, 1, "data")
    };
    if let Err(message) = handle_of(l).write(&data) {
        raise(l, &message);
    }
    0
}

unsafe extern "C" fn handle_seek(l: *mut lua_State) -> c_int {
    let whence = opt_str(l, 1).map(|b| String::from_utf8_lossy(&b).into_owned());
    let offset = opt_int(l, 2).unwrap_or(0);
    match handle_of(l).seek(whence.as_deref(), offset) {
        Ok(position) => lua_pushinteger(l, position),
        Err(message) => raise(l, &message),
    }
    1
}

unsafe extern "C" fn handle_flush(l: *mut lua_State) -> c_int {
    if let Err(message) = handle_of(l).flush() {
        raise(l, &message);
    }
    0
}

unsafe extern "C" fn handle_close(l: *mut lua_State) -> c_int {
    if let Err(message) = handle_of(l).close() {
        raise(l, &message);
    }
    0
}

unsafe extern "C" fn get_children(l: *mut lua_State) -> c_int {
    let Some((real, _)) = resolve(l, 2, 1) else {
        raise(l, "Invalid file path");
    };
    if !real.exists() {
        raise(l, "File does not exist");
    }
    if !real.is_dir() {
        raise(l, "Not a directory");
    }
    let mut names: Vec<String> = match std::fs::read_dir(&real) {
        Ok(entries) => entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect(),
        Err(e) => raise(l, &e.to_string()),
    };
    names.sort();
    lua_createtable(l, names.len() as c_int, 0);
    for (i, name) in names.iter().enumerate() {
        push_str(l, name);
        lua_rawseti(l, -2, i as i64 + 1);
    }
    1
}

unsafe extern "C" fn get_disks(l: *mut lua_State) -> c_int {
    let indices = host_of(l).disks.indices();
    lua_createtable(l, indices.len() as c_int, 0);
    for (i, slot) in indices.iter().enumerate() {
        lua_pushinteger(l, *slot);
        lua_rawseti(l, -2, i as i64 + 1);
    }
    1
}

unsafe extern "C" fn get_number_of_disks(l: *mut lua_State) -> c_int {
    lua_pushinteger(l, host_of(l).disks.len() as i64);
    1
}

unsafe extern "C" fn get_disk_id(l: *mut lua_State) -> c_int {
    let host = host_of(l);
    let uuid = disk_at(l, host, 1).uuid.clone();
    push_str(l, &uuid);
    1
}

unsafe extern "C" fn get_partitions(l: *mut lua_State) -> c_int {
    let host = host_of(l);
    let names: Vec<String> = disk_at(l, host, 1)
        .entries()
        .into_iter()
        .map(|e| e.name)
        .collect();
    lua_createtable(l, names.len() as c_int, 0);
    for (i, name) in names.iter().enumerate() {
        push_str(l, name);
        lua_rawseti(l, -2, i as i64 + 1);
    }
    1
}

unsafe extern "C" fn get_partition(l: *mut lua_State) -> c_int {
    let name = check_string(l, 1, "name");
    let host = host_of(l);
    match disk_at(l, host, 2).entry(&name) {
        None => lua_pushnil(l),
        Some(entry) => {
            let (path, readonly, hidden) = (entry.name, entry.readonly, entry.hidden);
            lua_createtable(l, 0, 3);
            push_str(l, &path);
            lua_setfield(l, -2, c"name".as_ptr());
            lua_pushboolean(l, readonly as c_int);
            lua_setfield(l, -2, c"readonly".as_ptr());
            lua_pushboolean(l, hidden as c_int);
            lua_setfield(l, -2, c"hidden".as_ptr());
        }
    }
    1
}

unsafe extern "C" fn make_dir(l: *mut lua_State) -> c_int {
    let made = match resolve(l, 2, 1) {
        Some((real, true)) => std::fs::create_dir_all(&real).is_ok(),
        _ => false,
    };
    lua_pushboolean(l, made as c_int);
    1
}

unsafe extern "C" fn exists(l: *mut lua_State) -> c_int {
    let found = resolve(l, 2, 1).is_some_and(|(real, _)| real.exists());
    trace!(
        "files",
        "exists({:?}) -> {}",
        check_string(l, 1, "path"),
        found
    );
    lua_pushboolean(l, found as c_int);
    1
}

unsafe extern "C" fn is_file(l: *mut lua_State) -> c_int {
    let found = resolve(l, 2, 1).is_some_and(|(real, _)| real.is_file());
    trace!(
        "files",
        "isFile({:?}) -> {}",
        check_string(l, 1, "path"),
        found
    );
    lua_pushboolean(l, found as c_int);
    1
}

unsafe extern "C" fn is_dir(l: *mut lua_State) -> c_int {
    let found = resolve(l, 2, 1).is_some_and(|(real, _)| real.is_dir());
    trace!(
        "files",
        "isDir({:?}) -> {}",
        check_string(l, 1, "path"),
        found
    );
    lua_pushboolean(l, found as c_int);
    1
}

/// Refuses to remove the entrypoint.
unsafe extern "C" fn delete(l: *mut lua_State) -> c_int {
    let path = check_string(l, 1, "path");
    let host = host_of(l);
    let disk = disk_at(l, host, 2);
    if crate::disk::split_path(&path) == crate::disk::split_path(disk.entrypoint()) {
        raise(l, "Access denied");
    }
    let removed = match disk.resolve(&path) {
        Some(r) if r.writable && r.real.is_dir() => std::fs::remove_dir_all(&r.real).is_ok(),
        Some(r) if r.writable => std::fs::remove_file(&r.real).is_ok(),
        _ => false,
    };
    lua_pushboolean(l, removed as c_int);
    1
}

unsafe extern "C" fn get_boot_path(l: *mut lua_State) -> c_int {
    let host = host_of(l);
    let entrypoint = disk_at(l, host, 1).entrypoint().to_string();
    push_str(l, &entrypoint);
    1
}

/// Partition editing is not implemented.
unsafe extern "C" fn unsupported(l: *mut lua_State) -> c_int {
    lua_pushboolean(l, 0);
    1
}

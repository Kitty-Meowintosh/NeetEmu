//! `io`, the peripheral table, ported from `APIS/IOAPI.java`.

use std::os::raw::c_int;

use super::*;
use crate::events::Value;
use crate::ffi::*;
use crate::vm::host_of;

pub unsafe fn install(l: *mut lua_State) {
    lua_createtable(l, 0, 11);
    set_fn(l, "getPeripherals", get_peripherals);
    set_fn(l, "getType", get_type);
    set_fn(l, "getTag", get_tag);
    set_fn(l, "setTag", set_tag);
    set_fn(l, "isCompatibility", is_compatibility);
    set_fn(l, "queryTag", query_tag);
    set_fn(l, "queryType", query_type);
    set_fn(l, "wrapPeripheral", wrap_peripheral);
    set_fn(l, "callFunction", call_function);
    set_fn(l, "isViewed", is_viewed);
    set_fn(l, "broadcastLocal", broadcast_local);
    set_global_table(l, "io");
}

/// Every lookup validates the uuid first, then fails the same way when nothing matches.
unsafe fn uuid_at(l: *mut lua_State, idx: c_int) -> String {
    let uuid = check_string(l, idx, "uuid");
    if !is_uuid(&uuid) {
        raise(l, "UUID invalidly formatted");
    }
    uuid
}

/// `UUID.fromString` accepts the 8-4-4-4-12 hex form.
fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&groups)
            .all(|(n, g)| g.len() == *n && g.chars().all(|c| c.is_ascii_hexdigit()))
}

unsafe fn push_list(l: *mut lua_State, items: &[String]) {
    lua_createtable(l, items.len() as c_int, 0);
    for (i, item) in items.iter().enumerate() {
        push_str(l, item);
        lua_rawseti(l, -2, i as i64 + 1);
    }
}

unsafe extern "C" fn get_peripherals(l: *mut lua_State) -> c_int {
    let ids = host_of(l).peripherals.ids();
    push_list(l, &ids);
    1
}

unsafe extern "C" fn get_type(l: *mut lua_State) -> c_int {
    let uuid = uuid_at(l, 1);
    match host_of(l).peripherals.type_of(&uuid) {
        Some(name) => push_str(l, name),
        None => raise(l, "Peripheral not found"),
    }
    1
}

unsafe extern "C" fn get_tag(l: *mut lua_State) -> c_int {
    let uuid = uuid_at(l, 1);
    match host_of(l).peripherals.tag_of(&uuid) {
        Some(tag) => push_str(l, tag),
        None => raise(l, "Peripheral not found"),
    }
    1
}

/// An absent tag clears it.
unsafe extern "C" fn set_tag(l: *mut lua_State) -> c_int {
    let uuid = uuid_at(l, 1);
    let tag = opt_str(l, 2)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    if !host_of(l).peripherals.set_tag(&uuid, &tag) {
        raise(l, "Peripheral not found");
    }
    0
}

unsafe extern "C" fn is_compatibility(l: *mut lua_State) -> c_int {
    let uuid = uuid_at(l, 1);
    match host_of(l).peripherals.is_compatibility(&uuid) {
        Some(yes) => lua_pushboolean(l, yes as c_int),
        None => raise(l, "Peripheral not found"),
    }
    1
}

unsafe extern "C" fn query_tag(l: *mut lua_State) -> c_int {
    let tag = check_string(l, 1, "tag");
    if tag.trim().is_empty() {
        raise(l, "tag cant be blank");
    }
    let found = host_of(l).peripherals.query_tag(tag.trim());
    push_list(l, &found);
    1
}

/// A bare name is namespaced, so `queryType("drive_bay")` finds the mod's own.
unsafe extern "C" fn query_type(l: *mut lua_State) -> c_int {
    let mut name = check_string(l, 1, "type");
    if !name.contains(':') {
        name = format!("neetcomputers:{name}");
    }
    let found = host_of(l).peripherals.query_type(&name);
    push_list(l, &found);
    1
}

/// One table of closures, each carrying its peripheral's uuid and its own name.
unsafe extern "C" fn wrap_peripheral(l: *mut lua_State) -> c_int {
    let uuid = uuid_at(l, 1);
    let Some(functions) = host_of(l).peripherals.functions(&uuid) else {
        raise(l, "Peripheral not found");
    };
    lua_createtable(l, 0, functions.len() as c_int);
    for name in &functions {
        let key = std::ffi::CString::new(name.as_str()).expect("name has a NUL");
        push_str(l, &uuid);
        push_str(l, name);
        lua_pushcclosure(l, wrapped, 2);
        lua_setfield(l, -2, key.as_ptr());
    }
    1
}

/// A closure from `wrapPeripheral`, whose upvalues stand in for its first two arguments.
unsafe extern "C" fn wrapped(l: *mut lua_State) -> c_int {
    let uuid = upvalue_string(l, 1);
    let name = upvalue_string(l, 2);
    let args = collect_args(l, 1, &uuid, &name);
    dispatch(l, &uuid, &name, &args)
}

unsafe extern "C" fn call_function(l: *mut lua_State) -> c_int {
    let uuid = uuid_at(l, 1);
    let name = check_string(l, 2, "functionName");
    let args = collect_args(l, 3, &uuid, &name);
    dispatch(l, &uuid, &name, &args)
}

unsafe fn collect_args(l: *mut lua_State, first: c_int, uuid: &str, name: &str) -> Vec<Value> {
    if host_of(l).peripherals.primitive_only(uuid, name) {
        check_primitives(l, first);
    }
    (first..=lua_gettop(l))
        .map(|i| super::event::value_at(l, i))
        .collect()
}

/// Refuses a table or function in an `@Primative` varargs parameter.
pub unsafe fn check_primitives(l: *mut lua_State, first: c_int) {
    for i in first..=lua_gettop(l) {
        let found = match lua_type(l, i) {
            LUA_TNIL | LUA_TNONE | LUA_TBOOLEAN | LUA_TNUMBER | LUA_TSTRING => continue,
            LUA_TTABLE => "table",
            LUA_TFUNCTION => "function",
            _ => "userdata",
        };
        raise(
            l,
            &format!("#{} Expected primitive, got {found}", i - first + 1),
        );
    }
}

unsafe fn dispatch(l: *mut lua_State, uuid: &str, name: &str, args: &[Value]) -> c_int {
    trace!("io", "{uuid} {name}({} args)", args.len());
    match host_of(l).call_peripheral(uuid, name, args) {
        None => raise(l, "Peripheral not found"),
        Some(Err(message)) => raise(l, &message),
        Some(Ok(returns)) => {
            for value in &returns {
                super::event::push_value(l, value);
            }
            returns.len() as c_int
        }
    }
}

unsafe fn upvalue_string(l: *mut lua_State, index: c_int) -> String {
    let mut len = 0usize;
    let p = lua_tolstring(l, lua_upvalueindex(index), &mut len);
    if p.is_null() {
        return String::new();
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned()
}

/// True when a player has the screen open, which is never.
unsafe extern "C" fn is_viewed(l: *mut lua_State) -> c_int {
    lua_pushboolean(l, 0);
    1
}

/// Queues the arguments for delivery to every peer machine after this tick.
unsafe extern "C" fn broadcast_local(l: *mut lua_State) -> c_int {
    check_primitives(l, 1);
    let args: Vec<Value> = (1..=lua_gettop(l))
        .map(|i| super::event::value_at(l, i))
        .collect();
    host_of(l)
        .outbox
        .push(crate::host::Outgoing::Broadcast(args));
    0
}

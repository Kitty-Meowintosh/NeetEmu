//! `event`, ported from `APIS/EventAPI.java`.

use std::os::raw::c_int;

use super::*;
use crate::events::{Event, Label, Value};
use crate::ffi::*;
use crate::vm::host_of;

pub unsafe fn install(l: *mut lua_State) {
    lua_createtable(l, 0, 4);
    set_fn(l, "queueEvent", queue_event);
    set_fn(l, "getQueue", get_queue);
    set_fn(l, "getFirst", get_first);
    set_fn(l, "clear", clear);
    set_global_table(l, "event");
}

/// Case-insensitive, as in `EventAPI.decodeEventLabel`.
unsafe fn label_at(l: *mut lua_State, idx: c_int) -> Label {
    let name = check_string(l, idx, "category");
    match Label::parse(&name) {
        Some(label) => label,
        None => raise(l, &format!("Invalid event category '{name}'")),
    }
}

/// Reads one Lua value as an event argument.
pub unsafe fn value_at(l: *mut lua_State, idx: c_int) -> Value {
    match lua_type(l, idx) {
        LUA_TNIL | LUA_TNONE => Value::Nil,
        LUA_TBOOLEAN => Value::Bool(lua_toboolean(l, idx) != 0),
        LUA_TNUMBER => {
            let mut is_int: c_int = 0;
            let i = lua_tointegerx(l, idx, &mut is_int);
            if is_int != 0 {
                Value::Int(i)
            } else {
                Value::Num(lua_tonumberx(l, idx, std::ptr::null_mut()))
            }
        }
        _ => {
            // Only valid UTF-8 becomes `Str`, so binary round-trips.
            let raw = opt_str(l, idx).unwrap_or_default();
            match String::from_utf8(raw) {
                Ok(text) => Value::Str(text),
                Err(e) => Value::Bytes(e.into_bytes()),
            }
        }
    }
}

pub unsafe fn push_value(l: *mut lua_State, value: &Value) {
    match value {
        Value::Nil => lua_pushnil(l),
        Value::Bool(b) => lua_pushboolean(l, *b as c_int),
        Value::Int(i) => lua_pushinteger(l, *i),
        Value::Num(n) => lua_pushnumber(l, *n),
        Value::Str(s) => push_str(l, s),
        Value::Bytes(b) => push_bytes(l, b),
        Value::Map(pairs) => {
            lua_createtable(l, 0, pairs.len() as c_int);
            for (key, value) in pairs {
                push_str(l, key);
                push_str(l, value);
                lua_settable(l, -3);
            }
        }
        Value::Fn(f) => super::internet::push_host_fn(l, *f),
    }
}

/// One event as a fresh `{ name, arg1, ... }`.
unsafe fn push_event(l: *mut lua_State, event: &Event) {
    lua_createtable(l, event.args.len() as c_int + 1, 0);
    push_str(l, &event.name);
    lua_rawseti(l, -2, 1);
    for (i, arg) in event.args.iter().enumerate() {
        push_value(l, arg);
        lua_rawseti(l, -2, i as i64 + 2);
    }
}

unsafe fn push_events(l: *mut lua_State, events: &[Event]) {
    lua_createtable(l, events.len() as c_int, 0);
    for (i, event) in events.iter().enumerate() {
        push_event(l, event);
        lua_rawseti(l, -2, i as i64 + 1);
    }
}

unsafe extern "C" fn queue_event(l: *mut lua_State) -> c_int {
    let category = check_string(l, 1, "category");
    let name = check_string(l, 2, "eventName");
    let label = if category.eq_ignore_ascii_case("all") {
        Label::Unlabeled
    } else {
        match Label::parse(&category) {
            Some(label) => label,
            None => raise(l, &format!("Invalid event category '{category}'")),
        }
    };
    let args = (3..=lua_gettop(l)).map(|i| value_at(l, i)).collect();
    host_of(l).queue_event(label, Event::new(name, args));
    0
}

/// Drains the queue as it reads, unlike `readQueue`.
unsafe extern "C" fn get_queue(l: *mut lua_State) -> c_int {
    let label = label_at(l, 1);
    let filter = opt_str(l, 2).map(|b| String::from_utf8_lossy(&b).into_owned());
    let host = host_of(l);
    let events = match &filter {
        Some(f) => host.events.take_filtered(label, f),
        None => host.events.take(label),
    };
    push_events(l, &events);
    1
}

unsafe extern "C" fn get_first(l: *mut lua_State) -> c_int {
    let label = label_at(l, 1);
    let filter = opt_str(l, 2).map(|b| String::from_utf8_lossy(&b).into_owned());
    let host = host_of(l);
    let event = match &filter {
        Some(f) => host.events.take_first_filtered(label, f),
        None => host.events.take_first(label),
    };
    match event {
        Some(event) => push_event(l, &event),
        None => lua_pushnil(l),
    }
    1
}

unsafe extern "C" fn clear(l: *mut lua_State) -> c_int {
    let host = host_of(l);
    if is_absent(l, 1) {
        host.events.reset();
    } else {
        let label = label_at(l, 1);
        host.events.clear(label);
    }
    0
}

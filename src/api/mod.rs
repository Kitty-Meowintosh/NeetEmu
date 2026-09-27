//! The host tables and the stack helpers they share.

pub mod bit32;
pub mod chip;
pub mod event;
pub mod files;
pub mod internet;
pub mod io;
pub mod screen;

use std::ffi::CString;
use std::os::raw::c_int;

use crate::ffi::*;

/// True when `NEETEMU_TRACE` names this table.
pub fn tracing(table: &str) -> bool {
    std::env::var("NEETEMU_TRACE").is_ok_and(|v| v == "all" || v.split(',').any(|t| t == table))
}

/// Logs one host call to stderr when its table is traced.
macro_rules! trace {
    ($table:literal, $($arg:tt)*) => {
        if $crate::api::tracing($table) {
            eprintln!("[{}] {}", $table, format_args!($($arg)*));
        }
    };
}
pub(crate) use trace;

/// Raises `msg` bare, as `APILoader.sandboxFunction` delivers it, and never returns.
pub unsafe fn raise(l: *mut lua_State, msg: &str) -> ! {
    trace!("errors", "{msg}");
    lua_pushlstring(l, msg.as_ptr() as *const i8, msg.len());
    lua_error(l);
    unreachable!("lua_error returned")
}

/// Installs `f` as `name` on the table at the top of the stack.
pub unsafe fn set_fn(l: *mut lua_State, name: &str, f: lua_CFunction) {
    let key = CString::new(name).expect("name has a NUL");
    lua_pushcfunction(l, f);
    lua_setfield(l, -2, key.as_ptr());
}

/// Installs `f` as `name`, with `id` as its single upvalue.
pub unsafe fn set_bound_fn(l: *mut lua_State, name: &str, f: lua_CFunction, id: i64) {
    let key = CString::new(name).expect("name has a NUL");
    lua_pushinteger(l, id);
    lua_pushcclosure(l, f, 1);
    lua_setfield(l, -2, key.as_ptr());
}

/// The slot a bound closure was created for.
pub unsafe fn bound_id(l: *mut lua_State) -> i64 {
    lua_tointegerx(l, lua_upvalueindex(1), std::ptr::null_mut())
}

/// Installs the table at the top of the stack as a global, popping it.
pub unsafe fn set_global_table(l: *mut lua_State, name: &str) {
    let key = CString::new(name).expect("name has a NUL");
    lua_setglobal(l, key.as_ptr());
}

pub unsafe fn push_str(l: *mut lua_State, s: &str) {
    lua_pushlstring(l, s.as_ptr() as *const i8, s.len());
}

pub unsafe fn push_bytes(l: *mut lua_State, b: &[u8]) {
    lua_pushlstring(l, b.as_ptr() as *const i8, b.len());
}

/// An absent argument, which `@CanBeNull` parameters accept.
pub unsafe fn is_absent(l: *mut lua_State, idx: c_int) -> bool {
    idx > lua_gettop(l) || lua_type(l, idx) == LUA_TNIL
}

pub unsafe fn opt_int(l: *mut lua_State, idx: c_int) -> Option<i64> {
    if is_absent(l, idx) {
        return None;
    }
    let mut ok: c_int = 0;
    let v = lua_tointegerx(l, idx, &mut ok);
    if ok != 0 {
        return Some(v);
    }
    // A float truncates toward zero.
    let mut ok: c_int = 0;
    let f = lua_tonumberx(l, idx, &mut ok);
    (ok != 0).then_some(f as i64)
}

pub unsafe fn check_int(l: *mut lua_State, idx: c_int, what: &str) -> i64 {
    match opt_int(l, idx) {
        Some(v) => v,
        None => raise(l, &format!("#{idx} {what}: number expected")),
    }
}

pub unsafe fn opt_str(l: *mut lua_State, idx: c_int) -> Option<Vec<u8>> {
    if is_absent(l, idx) {
        return None;
    }
    // `lua_tolstring` coerces a number in place, which would confuse a later `lua_type`.
    if !matches!(lua_type(l, idx), LUA_TSTRING | LUA_TNUMBER) {
        return None;
    }
    let mut len = 0usize;
    let p = lua_tolstring(l, idx, &mut len);
    (!p.is_null()).then(|| std::slice::from_raw_parts(p as *const u8, len).to_vec())
}

pub unsafe fn check_str(l: *mut lua_State, idx: c_int, what: &str) -> Vec<u8> {
    match opt_str(l, idx) {
        Some(v) => v,
        None => raise(l, &format!("#{idx} {what}: string expected")),
    }
}

/// A string argument decoded lossily, for paths and mode strings.
pub unsafe fn check_string(l: *mut lua_State, idx: c_int, what: &str) -> String {
    String::from_utf8_lossy(&check_str(l, idx, what)).into_owned()
}

pub unsafe fn opt_bool(l: *mut lua_State, idx: c_int) -> Option<bool> {
    (!is_absent(l, idx)).then(|| lua_toboolean(l, idx) != 0)
}

/// `RangeArgumentError` as `LuaMaster.generateError` renders it, without the ruleset.
pub unsafe fn range_error(l: *mut lua_State, position: c_int, min: i64, max: i64, value: i64) -> ! {
    raise(
        l,
        &format!(
            "#{} Number {value} not in range of {min}-{max}",
            position + 1
        ),
    )
}

/// Installs every host table on the globals of `l`.
pub unsafe fn install(l: *mut lua_State) {
    bit32::install(l);
    chip::install(l);
    event::install(l);
    files::install(l);
    internet::install(l);
    io::install(l);
    screen::install(l);
}

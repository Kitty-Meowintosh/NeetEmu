//! `bit32`, ported from `YSLua/Bit32Compat.java`.

use std::os::raw::c_int;

use super::*;
use crate::ffi::*;

pub unsafe fn install(l: *mut lua_State) {
    lua_createtable(l, 0, 13);
    set_fn(l, "band", band);
    set_fn(l, "bor", bor);
    set_fn(l, "bxor", bxor);
    set_fn(l, "bnot", bnot);
    set_fn(l, "btest", btest);
    set_fn(l, "lshift", lshift);
    set_fn(l, "rshift", rshift);
    set_fn(l, "arshift", arshift);
    set_fn(l, "lrotate", lrotate);
    set_fn(l, "rrotate", rrotate);
    set_fn(l, "extract", extract);
    set_fn(l, "replace", replace);
    set_global_table(l, "bit32");
}

/// Pushes a 32-bit result as an exact unsigned value.
unsafe fn push_bits(l: *mut lua_State, x: u32) {
    lua_pushinteger(l, x as i64);
}

unsafe fn arg(l: *mut lua_State, idx: c_int, fname: &str) -> u32 {
    if is_absent(l, idx) {
        raise(
            l,
            &format!("bad argument #{idx} to '{fname}' (number expected, got no value)"),
        );
    }
    match opt_int(l, idx) {
        // `Bit32Compat.java` narrows to `int`.
        Some(v) => v as u32,
        None => {
            let t = std::ffi::CStr::from_ptr(lua_typename(l, lua_type(l, idx))).to_string_lossy();
            raise(
                l,
                &format!("bad argument #{idx} to '{fname}' (number expected, got {t})"),
            )
        }
    }
}

unsafe fn opt_arg(l: *mut lua_State, idx: c_int, fallback: u32, fname: &str) -> u32 {
    if is_absent(l, idx) {
        fallback
    } else {
        arg(l, idx, fname)
    }
}

/// Folds every argument, of which there must be at least one.
unsafe fn fold(l: *mut lua_State, fname: &str, init: u32, f: fn(u32, u32) -> u32) -> u32 {
    let n = lua_gettop(l);
    if n == 0 {
        raise(
            l,
            &format!("bad argument #1 to '{fname}' (number expected, got no value)"),
        );
    }
    let mut acc = init;
    for i in 1..=n {
        acc = f(acc, arg(l, i, fname));
    }
    acc
}

unsafe extern "C" fn band(l: *mut lua_State) -> c_int {
    let v = fold(l, "band", !0, |a, b| a & b);
    push_bits(l, v);
    1
}

unsafe extern "C" fn bor(l: *mut lua_State) -> c_int {
    let v = fold(l, "bor", 0, |a, b| a | b);
    push_bits(l, v);
    1
}

unsafe extern "C" fn bxor(l: *mut lua_State) -> c_int {
    let v = fold(l, "bxor", 0, |a, b| a ^ b);
    push_bits(l, v);
    1
}

unsafe extern "C" fn btest(l: *mut lua_State) -> c_int {
    let v = fold(l, "btest", !0, |a, b| a & b);
    lua_pushboolean(l, (v != 0) as c_int);
    1
}

unsafe extern "C" fn bnot(l: *mut lua_State) -> c_int {
    let v = !arg(l, 1, "bnot");
    push_bits(l, v);
    1
}

/// A displacement of 32 or more clears the value, in either direction.
fn shift_left(x: u32, disp: i32) -> u32 {
    if !(-31..=31).contains(&disp) {
        return 0;
    }
    if disp >= 0 {
        x << disp
    } else {
        x >> -disp
    }
}

unsafe extern "C" fn lshift(l: *mut lua_State) -> c_int {
    let v = shift_left(arg(l, 1, "lshift"), arg(l, 2, "lshift") as i32);
    push_bits(l, v);
    1
}

unsafe extern "C" fn rshift(l: *mut lua_State) -> c_int {
    let v = shift_left(arg(l, 1, "rshift"), -(arg(l, 2, "rshift") as i32));
    push_bits(l, v);
    1
}

unsafe extern "C" fn arshift(l: *mut lua_State) -> c_int {
    let x = arg(l, 1, "arshift");
    let disp = arg(l, 2, "arshift") as i32;
    let v = if disp >= 0 {
        if disp >= 32 {
            if (x as i32) < 0 {
                !0
            } else {
                0
            }
        } else {
            ((x as i32) >> disp) as u32
        }
    } else {
        shift_left(x, -disp)
    };
    push_bits(l, v);
    1
}

unsafe extern "C" fn lrotate(l: *mut lua_State) -> c_int {
    let v = arg(l, 1, "lrotate").rotate_left((arg(l, 2, "lrotate") as i32).rem_euclid(32) as u32);
    push_bits(l, v);
    1
}

unsafe extern "C" fn rrotate(l: *mut lua_State) -> c_int {
    let v = arg(l, 1, "rrotate").rotate_right((arg(l, 2, "rrotate") as i32).rem_euclid(32) as u32);
    push_bits(l, v);
    1
}

/// A width of 32 shifts in 64 bits.
fn field_mask(width: u32) -> u32 {
    if width >= 32 {
        !0
    } else {
        (1u32 << width) - 1
    }
}

unsafe fn check_field(l: *mut lua_State, field: i32, width: i32, fname: &str, argn: c_int) {
    if field < 0 {
        raise(
            l,
            &format!("bad argument #{argn} to '{fname}' (field cannot be negative)"),
        );
    }
    if width < 0 {
        raise(
            l,
            &format!(
                "bad argument #{} to '{fname}' (width must be positive)",
                argn + 1
            ),
        );
    }
    if field + width > 32 {
        raise(l, "trying to access non-existent bits");
    }
}

unsafe extern "C" fn extract(l: *mut lua_State) -> c_int {
    let n = arg(l, 1, "extract");
    let field = arg(l, 2, "extract") as i32;
    let width = opt_arg(l, 3, 1, "extract") as i32;
    check_field(l, field, width, "extract", 2);
    push_bits(l, (n >> field) & field_mask(width as u32));
    1
}

unsafe extern "C" fn replace(l: *mut lua_State) -> c_int {
    let n = arg(l, 1, "replace");
    let v = arg(l, 2, "replace");
    let field = arg(l, 3, "replace") as i32;
    let width = opt_arg(l, 4, 1, "replace") as i32;
    check_field(l, field, width, "replace", 3);
    let mask = field_mask(width as u32) << field;
    push_bits(l, (n & !mask) | ((v << field) & mask));
    1
}

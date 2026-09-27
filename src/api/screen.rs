//! `screen` and its layers, ported from `APIS/graphics/ScreenAPI.java` and `GraphicalAPI.java`.

use std::os::raw::c_int;

use super::*;
use crate::ffi::*;
use crate::screen::{GfxError, Surface};
use crate::vm::host_of;

/// Slot 0 is the screen itself, which is also usable as a layer.
const SCREEN: i64 = 0;

pub unsafe fn install(l: *mut lua_State) {
    push_surface(l, SCREEN);
    set_fn(l, "draw", draw);
    set_global_table(l, "screen");
}

/// Every surface gets the twelve methods, and the screen gets `draw` as well.
unsafe fn push_surface(l: *mut lua_State, id: i64) {
    lua_createtable(l, 0, 13);
    set_bound_fn(l, "getSize", get_size, id);
    set_bound_fn(l, "writePixel", write_pixel, id);
    set_bound_fn(l, "readPixel", read_pixel, id);
    set_bound_fn(l, "writeLine", write_line, id);
    set_bound_fn(l, "substitute", substitute, id);
    set_bound_fn(l, "clone", clone_rect, id);
    set_bound_fn(l, "readData", read_data, id);
    set_bound_fn(l, "writeData", write_data, id);
    set_bound_fn(l, "set", set, id);
    set_bound_fn(l, "fill", fill, id);
    set_bound_fn(l, "createLayer", create_layer, id);
}

unsafe fn surface_of(l: *mut lua_State) -> &'static mut Surface {
    let id = bound_id(l);
    match host_of(l).surfaces.get_mut(id as usize) {
        Some(surface) => surface,
        None => raise(l, "Layer no longer exists"),
    }
}

/// Raises a `GfxError`.
unsafe fn fail(l: *mut lua_State, error: GfxError) -> ! {
    match error {
        GfxError::Exposed(message) => raise(l, &message),
        GfxError::Range(position, min, max, value) => range_error(l, position, min, max, value),
    }
}

unsafe extern "C" fn get_size(l: *mut lua_State) -> c_int {
    let (w, h) = surface_of(l).size();
    lua_pushinteger(l, w as i64);
    lua_pushinteger(l, h as i64);
    2
}

unsafe extern "C" fn write_pixel(l: *mut lua_State) -> c_int {
    let (x, y) = (check_int(l, 1, "x"), check_int(l, 2, "y"));
    let (r, g, b) = (
        check_int(l, 3, "red"),
        check_int(l, 4, "green"),
        check_int(l, 5, "blue"),
    );
    let alpha = opt_int(l, 6);
    surface_of(l).write_pixel(x, y, r, g, b, alpha);
    0
}

unsafe extern "C" fn read_pixel(l: *mut lua_State) -> c_int {
    let (x, y) = (check_int(l, 1, "x"), check_int(l, 2, "y"));
    match surface_of(l).read_pixel(x, y) {
        Ok((r, g, b)) => {
            lua_pushinteger(l, r as i64);
            lua_pushinteger(l, g as i64);
            lua_pushinteger(l, b as i64);
            3
        }
        Err(e) => fail(l, e),
    }
}

unsafe extern "C" fn write_line(l: *mut lua_State) -> c_int {
    let (x1, y1) = (check_int(l, 1, "x1"), check_int(l, 2, "y1"));
    let (x2, y2) = (check_int(l, 3, "x2"), check_int(l, 4, "y2"));
    let (r, g, b) = (
        check_int(l, 5, "red"),
        check_int(l, 6, "green"),
        check_int(l, 7, "blue"),
    );
    let alpha = opt_int(l, 8);
    surface_of(l).write_line(x1, y1, x2, y2, r, g, b, alpha);
    0
}

unsafe extern "C" fn substitute(l: *mut lua_State) -> c_int {
    let (r1, g1, b1) = (
        check_int(l, 1, "red1"),
        check_int(l, 2, "green1"),
        check_int(l, 3, "blue1"),
    );
    let (r2, g2, b2) = (
        check_int(l, 4, "red2"),
        check_int(l, 5, "green2"),
        check_int(l, 6, "blue2"),
    );
    surface_of(l).substitute(r1, g1, b1, r2, g2, b2);
    0
}

unsafe extern "C" fn clone_rect(l: *mut lua_State) -> c_int {
    let (x1, y1) = (check_int(l, 1, "x1"), check_int(l, 2, "y1"));
    let (x2, y2) = (check_int(l, 3, "x2"), check_int(l, 4, "y2"));
    let copy = match surface_of(l).clone_rect(x1, y1, x2, y2) {
        Ok(surface) => surface,
        Err(e) => fail(l, e),
    };
    let id = host_of(l).surfaces.insert(copy);
    push_surface(l, id as i64);
    1
}

unsafe extern "C" fn read_data(l: *mut lua_State) -> c_int {
    let (x1, y1) = (check_int(l, 1, "x1"), check_int(l, 2, "y1"));
    let (x2, y2) = (check_int(l, 3, "x2"), check_int(l, 4, "y2"));
    match surface_of(l).read_data(x1, y1, x2, y2) {
        Ok(bytes) => push_bytes(l, &bytes),
        Err(e) => fail(l, e),
    }
    1
}

unsafe extern "C" fn write_data(l: *mut lua_State) -> c_int {
    let (x, y) = (check_int(l, 1, "x"), check_int(l, 2, "y"));
    let data = check_str(l, 3, "buffer");
    let width = check_int(l, 4, "width");
    let replace = opt_bool(l, 5).unwrap_or(false);
    if let Err(e) = surface_of(l).write_data(x, y, &data, width, replace) {
        fail(l, e);
    }
    0
}

/// `set()` clears; `set(r, g, b, a?)` fills or blends.
unsafe extern "C" fn set(l: *mut lua_State) -> c_int {
    if lua_gettop(l) == 0 {
        surface_of(l).clear();
        return 0;
    }
    let (r, g, b) = (
        check_int(l, 1, "red"),
        check_int(l, 2, "green"),
        check_int(l, 3, "blue"),
    );
    let alpha = opt_int(l, 4);
    surface_of(l).set(r, g, b, alpha);
    0
}

unsafe extern "C" fn fill(l: *mut lua_State) -> c_int {
    let (x1, y1) = (check_int(l, 1, "x1"), check_int(l, 2, "y1"));
    let (x2, y2) = (check_int(l, 3, "x2"), check_int(l, 4, "y2"));
    let (r, g, b) = (
        check_int(l, 5, "red"),
        check_int(l, 6, "green"),
        check_int(l, 7, "blue"),
    );
    let alpha = opt_int(l, 8);
    let replace = opt_bool(l, 9).unwrap_or(false);
    if let Err(e) = surface_of(l).fill(x1, y1, x2, y2, r, g, b, alpha, replace) {
        fail(l, e);
    }
    0
}

unsafe extern "C" fn create_layer(l: *mut lua_State) -> c_int {
    let (w, h) = (check_int(l, 1, "sizex"), check_int(l, 2, "sizey"));
    let transparent = opt_bool(l, 3).unwrap_or(false);
    let id = match host_of(l).surfaces.create(w, h, transparent) {
        Ok(id) => id,
        Err(e) => fail(l, e),
    };
    push_surface(l, id as i64);
    1
}

/// Presents the screen; the display picks it up between ticks.
unsafe extern "C" fn draw(l: *mut lua_State) -> c_int {
    host_of(l).dirty = true;
    0
}

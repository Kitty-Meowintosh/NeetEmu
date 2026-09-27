//! Hand-written declarations for YSLua (Lua 5.5 plus the forced-yield extension).

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::os::raw::{c_char, c_int, c_uint, c_void};

pub enum lua_State {}

pub type lua_Number = f64;
pub type lua_Integer = i64;
pub type lua_KContext = isize;
pub type lua_CFunction = unsafe extern "C" fn(L: *mut lua_State) -> c_int;
pub type lua_KFunction =
    unsafe extern "C" fn(L: *mut lua_State, status: c_int, ctx: lua_KContext) -> c_int;
pub type lua_Hook = unsafe extern "C" fn(L: *mut lua_State, ar: *mut lua_Debug);
pub type lua_WarnFunction =
    unsafe extern "C" fn(ud: *mut c_void, msg: *const c_char, tocont: c_int);
pub type lua_Alloc = unsafe extern "C" fn(
    ud: *mut c_void,
    ptr: *mut c_void,
    osize: usize,
    nsize: usize,
) -> *mut c_void;

pub const LUA_OK: c_int = 0;
pub const LUA_YIELD: c_int = 1;
pub const LUA_ERRRUN: c_int = 2;
pub const LUA_ERRSYNTAX: c_int = 3;
pub const LUA_ERRMEM: c_int = 4;
pub const LUA_ERRERR: c_int = 5;

pub const LUA_TNONE: c_int = -1;
pub const LUA_TNIL: c_int = 0;
pub const LUA_TBOOLEAN: c_int = 1;
pub const LUA_TLIGHTUSERDATA: c_int = 2;
pub const LUA_TNUMBER: c_int = 3;
pub const LUA_TSTRING: c_int = 4;
pub const LUA_TTABLE: c_int = 5;
pub const LUA_TFUNCTION: c_int = 6;
pub const LUA_TUSERDATA: c_int = 7;
pub const LUA_TTHREAD: c_int = 8;

pub const LUA_MASKCOUNT: c_int = 1 << 3;

pub const LUA_GCSTEP: c_int = 5;

pub const LUA_REGISTRYINDEX: c_int = -(c_int::MAX / 2 + 1000);
/// Registry slots as Lua 5.5 numbers them.
pub const LUA_RIDX_GLOBALS: lua_Integer = 2;

pub const LUA_MULTRET: c_int = -1;

/// Only the prefix the host reads; the tail of the struct is opaque padding.
#[repr(C)]
pub struct lua_Debug {
    pub event: c_int,
    pub name: *const c_char,
    pub namewhat: *const c_char,
    pub what: *const c_char,
    pub source: *const c_char,
    pub srclen: usize,
    pub currentline: c_int,
    pub linedefined: c_int,
    pub lastlinedefined: c_int,
    pub nups: u8,
    pub nparams: u8,
    pub isvararg: c_char,
    pub istailcall: c_char,
    pub ftransfer: c_int,
    pub ntransfer: c_int,
    pub short_src: [c_char; 60],
    _private: [u8; 128],
}

extern "C" {
    pub fn luaL_newstate() -> *mut lua_State;
    pub fn lua_newstate(f: lua_Alloc, ud: *mut c_void, seed: c_uint) -> *mut lua_State;
    pub fn lua_close(L: *mut lua_State);
    pub fn luaL_openselectedlibs(L: *mut lua_State, load: c_int, preload: c_int);

    pub fn luaL_loadbufferx(
        L: *mut lua_State,
        buff: *const c_char,
        sz: usize,
        name: *const c_char,
        mode: *const c_char,
    ) -> c_int;

    pub fn lua_resume(
        L: *mut lua_State,
        from: *mut lua_State,
        narg: c_int,
        nres: *mut c_int,
    ) -> c_int;
    pub fn lua_yieldk(
        L: *mut lua_State,
        nresults: c_int,
        ctx: lua_KContext,
        k: Option<lua_KFunction>,
    ) -> c_int;
    pub fn lua_pcallk(
        L: *mut lua_State,
        nargs: c_int,
        nresults: c_int,
        errfunc: c_int,
        ctx: lua_KContext,
        k: *const c_void,
    ) -> c_int;
    pub fn lua_callk(
        L: *mut lua_State,
        nargs: c_int,
        nresults: c_int,
        ctx: lua_KContext,
        k: *const c_void,
    );

    pub fn lua_sethook(L: *mut lua_State, func: Option<lua_Hook>, mask: c_int, count: c_int);
    pub fn lua_setwarnf(L: *mut lua_State, f: Option<lua_WarnFunction>, ud: *mut c_void);
    pub fn lua_gc(L: *mut lua_State, what: c_int, ...) -> c_int;

    // The YSLua extension (lua/lua.h:197-357).
    pub fn lua_enablemainyield(L: *mut lua_State);
    pub fn lua_markforcedyield(L: *mut lua_State);
    pub fn lua_wasforcedyield(L: *mut lua_State) -> c_int;

    pub fn lua_gettop(L: *mut lua_State) -> c_int;
    pub fn lua_settop(L: *mut lua_State, idx: c_int);
    pub fn lua_pushvalue(L: *mut lua_State, idx: c_int);
    pub fn lua_rotate(L: *mut lua_State, idx: c_int, n: c_int);
    pub fn lua_absindex(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_checkstack(L: *mut lua_State, n: c_int) -> c_int;

    pub fn lua_type(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_typename(L: *mut lua_State, tp: c_int) -> *const c_char;
    pub fn lua_toboolean(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_tolstring(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char;
    pub fn lua_tonumberx(L: *mut lua_State, idx: c_int, isnum: *mut c_int) -> lua_Number;
    pub fn lua_tointegerx(L: *mut lua_State, idx: c_int, isnum: *mut c_int) -> lua_Integer;
    pub fn lua_touserdata(L: *mut lua_State, idx: c_int) -> *mut c_void;
    pub fn lua_rawlen(L: *mut lua_State, idx: c_int) -> u64;

    pub fn lua_pushnil(L: *mut lua_State);
    pub fn lua_pushnumber(L: *mut lua_State, n: lua_Number);
    pub fn lua_pushinteger(L: *mut lua_State, n: lua_Integer);
    pub fn lua_pushlstring(L: *mut lua_State, s: *const c_char, len: usize) -> *const c_char;
    pub fn lua_pushstring(L: *mut lua_State, s: *const c_char) -> *const c_char;
    pub fn lua_pushboolean(L: *mut lua_State, b: c_int);
    pub fn lua_pushcclosure(L: *mut lua_State, f: lua_CFunction, n: c_int);
    pub fn lua_pushlightuserdata(L: *mut lua_State, p: *mut c_void);

    pub fn lua_createtable(L: *mut lua_State, narr: c_int, nrec: c_int);
    pub fn lua_setfield(L: *mut lua_State, idx: c_int, k: *const c_char);
    pub fn lua_getfield(L: *mut lua_State, idx: c_int, k: *const c_char) -> c_int;
    pub fn lua_settable(L: *mut lua_State, idx: c_int);
    pub fn lua_gettable(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_next(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_rawseti(L: *mut lua_State, idx: c_int, n: lua_Integer);
    pub fn lua_rawgeti(L: *mut lua_State, idx: c_int, n: lua_Integer) -> c_int;
    pub fn lua_setglobal(L: *mut lua_State, name: *const c_char);
    pub fn lua_getglobal(L: *mut lua_State, name: *const c_char) -> c_int;

    pub fn lua_error(L: *mut lua_State) -> c_int;
    pub fn lua_getinfo(L: *mut lua_State, what: *const c_char, ar: *mut lua_Debug) -> c_int;
}

// The header spells these as macros; Rust cannot link to them.

/// `lua_pop(L, n)`
pub unsafe fn lua_pop(l: *mut lua_State, n: c_int) {
    lua_settop(l, -n - 1);
}

/// `lua_yield(L, n)`
pub unsafe fn lua_yield(l: *mut lua_State, nresults: c_int) -> c_int {
    lua_yieldk(l, nresults, 0, None)
}

/// `luaL_loadbuffer(L, s, sz, n)`
pub unsafe fn luaL_loadbuffer(
    l: *mut lua_State,
    buff: *const c_char,
    sz: usize,
    name: *const c_char,
) -> c_int {
    luaL_loadbufferx(l, buff, sz, name, std::ptr::null())
}

/// `lua_pcall(L, nargs, nresults, errfunc)`
pub unsafe fn lua_pcall(l: *mut lua_State, nargs: c_int, nresults: c_int, errfunc: c_int) -> c_int {
    lua_pcallk(l, nargs, nresults, errfunc, 0, std::ptr::null())
}

/// `lua_pushcfunction(L, f)`
pub unsafe fn lua_pushcfunction(l: *mut lua_State, f: lua_CFunction) {
    lua_pushcclosure(l, f, 0)
}

/// `lua_upvalueindex(i)`
pub const fn lua_upvalueindex(i: c_int) -> c_int {
    LUA_REGISTRYINDEX - i
}

/// `luaL_openlibs(L)` — a macro over `luaL_openselectedlibs` in 5.5.
pub unsafe fn luaL_openlibs(l: *mut lua_State) {
    luaL_openselectedlibs(l, !0, 0)
}

/// `lua_newtable(L)`
pub unsafe fn lua_newtable(l: *mut lua_State) {
    lua_createtable(l, 0, 0)
}

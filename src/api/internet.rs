//! `internet`, ported from `APIS/InternetAPI.java`.

use std::os::raw::c_int;

use super::*;
use crate::events::HostFn;
use crate::ffi::*;
use crate::vm::host_of;

pub unsafe fn install(l: *mut lua_State) {
    lua_createtable(l, 0, 5);
    set_fn(l, "GET", get);
    set_fn(l, "POST", post);
    set_fn(l, "CreateWebsocket", create_websocket);
    set_fn(l, "hasAccess", has_access);
    set_fn(l, "isReady", is_ready);
    set_global_table(l, "internet");
}

/// `new URI(URL)`, whose failure is reported as `Invalid URL`.
unsafe fn url_at(l: *mut lua_State, idx: c_int) -> String {
    let url = check_string(l, idx, "URL");
    let (scheme, rest) = match url.split_once("://") {
        Some(split) => split,
        None => raise(l, "Invalid URL"),
    };
    if scheme.is_empty() || rest.is_empty() || url.contains(char::is_whitespace) {
        raise(l, "Invalid URL");
    }
    url
}

/// An absent table is no headers; a non-string entry is rejected whole.
unsafe fn headers_at(l: *mut lua_State, idx: c_int) -> Vec<(String, String)> {
    if is_absent(l, idx) {
        return Vec::new();
    }
    if lua_type(l, idx) != LUA_TTABLE {
        raise(l, "Invalid Header (All values must be strings)");
    }
    let mut headers = Vec::new();
    lua_pushnil(l);
    while lua_next(l, idx) != 0 {
        // `lua_tolstring` would rewrite a number key in place and derail `lua_next`.
        let readable = |at: c_int| matches!(lua_type(l, at), LUA_TSTRING | LUA_TNUMBER);
        if !readable(-2) || !readable(-1) {
            raise(l, "Invalid Header (All values must be strings)");
        }
        let key = String::from_utf8_lossy(&opt_str(l, -2).unwrap_or_default()).into_owned();
        let value = String::from_utf8_lossy(&opt_str(l, -1).unwrap_or_default()).into_owned();
        headers.push((key, value));
        lua_pop(l, 1);
    }
    headers
}

unsafe extern "C" fn get(l: *mut lua_State) -> c_int {
    let url = url_at(l, 1);
    let headers = headers_at(l, 2);
    trace!("internet", "GET {url}");
    let id = host_of(l).internet.get(&url, headers);
    lua_pushinteger(l, id as i64);
    1
}

unsafe extern "C" fn post(l: *mut lua_State) -> c_int {
    let url = url_at(l, 1);
    let headers = headers_at(l, 2);
    let body = opt_str(l, 3).unwrap_or_default();
    trace!("internet", "POST {url} ({} bytes)", body.len());
    let id = host_of(l).internet.post(&url, headers, body);
    lua_pushinteger(l, id as i64);
    1
}

/// Raises on refusal instead of queueing a response.
unsafe extern "C" fn create_websocket(l: *mut lua_State) -> c_int {
    let url = url_at(l, 1);
    let headers = headers_at(l, 2);
    trace!("internet", "websocket {url}");
    match host_of(l).internet.open_socket(&url, headers) {
        Ok(id) => lua_pushinteger(l, id as i64),
        Err(message) => raise(l, &message),
    }
    1
}

unsafe extern "C" fn has_access(l: *mut lua_State) -> c_int {
    lua_pushboolean(l, host_of(l).internet.has_access() as c_int);
    1
}

unsafe extern "C" fn is_ready(l: *mut lua_State) -> c_int {
    let internet = &host_of(l).internet;
    lua_pushboolean(l, (internet.ready() && internet.has_access()) as c_int);
    1
}

/// Pushes one of the callables `WebsocketOpened` hands the guest.
pub unsafe fn push_host_fn(l: *mut lua_State, f: HostFn) {
    match f {
        HostFn::WebsocketSend(id) => {
            lua_pushinteger(l, id as i64);
            lua_pushcclosure(l, websocket_send, 1);
        }
        HostFn::WebsocketClose(id) => {
            lua_pushinteger(l, id as i64);
            lua_pushcclosure(l, websocket_close, 1);
        }
    }
}

unsafe extern "C" fn websocket_send(l: *mut lua_State) -> c_int {
    let id = bound_id(l) as i32;
    let data = check_str(l, 1, "data");
    let binary = opt_bool(l, 2).unwrap_or(false);
    host_of(l).internet.send(id, data, binary);
    0
}

unsafe extern "C" fn websocket_close(l: *mut lua_State) -> c_int {
    let id = bound_id(l) as i32;
    if let Err(message) = host_of(l).internet.close(id) {
        raise(l, &message);
    }
    0
}

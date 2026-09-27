//! `chip`, ported from `APIS/ChipAPI.java`.

use std::os::raw::c_int;

use super::*;
use crate::ffi::*;
use crate::host::Woke;
use crate::vm::{host_of, Outcome};

/// What `chip.version` reports.
pub const NEET_VERSION: &str = "0.5.3";
/// `DefaultComputerConfig.modelName`.
pub const MACHINE: &str = "NeetEmu";

/// Ticks in a Minecraft day.
const LUNAR_DAY: i64 = 24_000;

pub unsafe fn install(l: *mut lua_State) {
    lua_createtable(l, 0, 12);
    set_fn(l, "getUnixTime", get_unix_time);
    set_fn(l, "getTime", get_time);
    set_fn(l, "getLunarTime", get_lunar_time);
    set_fn(l, "getUUID", get_uuid);
    set_fn(l, "getMachine", get_machine);
    set_fn(l, "shutdown", shutdown);
    set_fn(l, "reboot", reboot);
    set_fn(l, "crash", crash);
    set_fn(l, "version", version);
    set_fn(l, "sleep", sleep);
    if host_of(l).config.clipboard {
        set_fn(l, "getClipboard", get_clipboard);
        set_fn(l, "setClipboard", set_clipboard);
    }
    set_global_table(l, "chip");
}

/// Seconds since the epoch as a float, finer than a millisecond.
unsafe extern "C" fn get_unix_time(l: *mut lua_State) -> c_int {
    lua_pushnumber(l, host_of(l).unix());
    1
}

unsafe extern "C" fn get_time(l: *mut lua_State) -> c_int {
    lua_pushnumber(l, host_of(l).uptime());
    1
}

unsafe extern "C" fn get_lunar_time(l: *mut lua_State) -> c_int {
    let ticks = (host_of(l).uptime() * 20.0) as i64;
    lua_pushinteger(l, ticks.rem_euclid(LUNAR_DAY));
    1
}

unsafe extern "C" fn get_uuid(l: *mut lua_State) -> c_int {
    let host = host_of(l);
    let uuid = format!("00000000-0000-4000-9000-{:012x}", host.id);
    push_str(l, &uuid);
    1
}

unsafe extern "C" fn get_machine(l: *mut lua_State) -> c_int {
    push_str(l, MACHINE);
    1
}

unsafe extern "C" fn version(l: *mut lua_State) -> c_int {
    push_str(l, NEET_VERSION);
    1
}

/// Parks until `seconds` elapse or an event arrives, with a forced yield that relays through nested resumes.
unsafe extern "C" fn sleep(l: *mut lua_State) -> c_int {
    let host = host_of(l);
    let until = opt_number(l, 1).map(|seconds| host.uptime() + seconds.max(0.0));

    // Work already waiting means there is nothing to wait for.
    if !host.events.is_empty() {
        push_str(l, Woke::Event.name());
        return 1;
    }
    // A zero-length sleep is a plain yield.
    if until.is_some_and(|deadline| deadline <= host.uptime()) {
        push_str(l, Woke::Timeout.name());
        return 1;
    }
    host.park_until(until);
    lua_markforcedyield(l);
    lua_yieldk(l, 0, 0, Some(woken))
}

/// Resumes `chip.sleep` once the host has unparked the machine.
unsafe extern "C" fn woken(l: *mut lua_State, _status: c_int, _ctx: lua_KContext) -> c_int {
    let reason = host_of(l).woke();
    push_str(l, reason.name());
    1
}

/// The host clipboard, empty when nothing has been copied.
unsafe extern "C" fn get_clipboard(l: *mut lua_State) -> c_int {
    push_str(l, &crate::clipboard::get());
    1
}

unsafe extern "C" fn set_clipboard(l: *mut lua_State) -> c_int {
    let text = check_string(l, 1, "text");
    crate::clipboard::set(&text);
    0
}

unsafe fn opt_number(l: *mut lua_State, idx: c_int) -> Option<f64> {
    if is_absent(l, idx) {
        return None;
    }
    let mut ok: c_int = 0;
    let v = lua_tonumberx(l, idx, &mut ok);
    (ok != 0).then_some(v)
}

unsafe extern "C" fn shutdown(l: *mut lua_State) -> c_int {
    host_of(l).set_exit(Outcome::Shutdown);
    0
}

unsafe extern "C" fn reboot(l: *mut lua_State) -> c_int {
    host_of(l).set_exit(Outcome::Reboot);
    0
}

unsafe extern "C" fn crash(l: *mut lua_State) -> c_int {
    let message = opt_str(l, 1)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    host_of(l).set_exit(Outcome::Crashed(message));
    0
}

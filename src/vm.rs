//! The lua_State, the count hook and the ticket-spending resume loop, after `Neet-YSLua/bridge/LuaBridge.cpp`.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

use crate::ffi::*;
use crate::host::Host;

/// One Minecraft tick.
pub const TICK: std::time::Duration = std::time::Duration::from_millis(50);

/// `simulation/config/DefaultComputerConfig.java`.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Instructions per hook interval, which is one ticket.
    pub instructions_per_batch: c_int,
    /// Tickets granted per tick.
    pub batches_per_tick: i64,
    /// How long one resume may run before the host takes a turn, unbounded when None.
    pub slice: Option<std::time::Duration>,
    pub screen_width: u32,
    pub screen_height: u32,
    /// Charges tickets but never forces a yield.
    pub no_preempt: bool,
    /// Gives the guest `chip.getClipboard` and `chip.setClipboard`.
    pub clipboard: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            instructions_per_batch: 3000,
            batches_per_tick: 3750,
            slice: None,
            screen_width: 800,
            screen_height: 600,
            no_preempt: false,
            clipboard: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Tickets ran out; the script is mid-flight.
    Ran,
    /// The entrypoint returned.
    Completed,
    Shutdown,
    Reboot,
    Crashed(String),
    Error(String),
}

pub struct Vm {
    l: *mut lua_State,
    host: Box<Host>,
    config: Config,
}

impl Vm {
    pub fn new(host: Host, config: Config) -> Result<Self, String> {
        let l = unsafe { luaL_newstate() };
        if l.is_null() {
            return Err("luaL_newstate returned NULL".into());
        }
        let mut vm = Vm {
            l,
            host: Box::new(host),
            config,
        };
        unsafe {
            *(lua_getextraspace(vm.l) as *mut *mut Host) = &mut *vm.host;
            luaL_openlibs(vm.l);
            vm.strip_stdlib();
            lua_setwarnf(vm.l, Some(warn_handler), std::ptr::null_mut());
            lua_pushcfunction(vm.l, lua_print);
            lua_setglobal(vm.l, c"print".as_ptr());
            lua_enablemainyield(vm.l);
        }
        Ok(vm)
    }

    pub fn state(&mut self) -> *mut lua_State {
        self.l
    }

    pub fn host(&mut self) -> &mut Host {
        &mut self.host
    }

    pub fn config(&self) -> Config {
        self.config
    }

    /// The stock globals `LuaBridge.cpp` removes.
    unsafe fn strip_stdlib(&mut self) {
        for name in [
            c"os",
            c"dofile",
            c"loadfile",
            c"collectgarbage",
            c"package",
            c"require",
            c"io",
            c"warn",
        ] {
            lua_pushnil(self.l);
            lua_setglobal(self.l, name.as_ptr());
        }
    }

    /// Loads the entrypoint, leaving it on the stack as the function to resume.
    pub fn load_entrypoint(&mut self, code: &[u8], chunkname: &str) -> Result<(), String> {
        let name = CString::new(chunkname).map_err(|_| "chunkname has a NUL".to_string())?;
        unsafe {
            lua_sethook(
                self.l,
                Some(clock_hook),
                LUA_MASKCOUNT,
                self.config.instructions_per_batch,
            );
            let rc = luaL_loadbuffer(
                self.l,
                code.as_ptr() as *const c_char,
                code.len(),
                name.as_ptr(),
            );
            if rc != LUA_OK {
                let msg = pop_string(self.l);
                return Err(msg);
            }
        }
        Ok(())
    }

    /// One 50 ms tick: grants tickets and resumes until they run out or `slice` ends it.
    pub fn tick(&mut self) -> Outcome {
        if !std::mem::take(&mut self.host.mid_tick) {
            unsafe {
                lua_gc(self.l, LUA_GCSTEP, 0);
            }
            self.host.tickets += self.config.batches_per_tick;
        }
        self.host.start_slice(self.config.slice);

        while self.host.tickets > 0 {
            if let Some(outcome) = self.host.take_exit() {
                return outcome;
            }
            // A parked machine is done for this tick.
            if self.host.is_parked() {
                return Outcome::Ran;
            }
            let mut nres: c_int = 0;
            let rc = unsafe { lua_resume(self.l, std::ptr::null_mut(), 0, &mut nres) };
            match rc {
                LUA_OK => {
                    return self.host.take_exit().unwrap_or(Outcome::Completed);
                }
                LUA_YIELD => {
                    // The script is never handed values back; drop whatever it yielded.
                    unsafe { lua_pop(self.l, nres) };
                    if self.host.slice_expired() {
                        self.host.mid_tick = true;
                        return Outcome::Ran;
                    }
                }
                _ => {
                    let msg = unsafe { pop_string(self.l) };
                    return Outcome::Error(msg);
                }
            }
        }
        self.host.take_exit().unwrap_or(Outcome::Ran)
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        unsafe { lua_close(self.l) };
    }
}

/// `lua_getextraspace(L)` — the pointer slot ahead of every thread, copied into coroutines.
unsafe fn lua_getextraspace(l: *mut lua_State) -> *mut c_void {
    (l as *mut u8).sub(std::mem::size_of::<*mut c_void>()) as *mut c_void
}

/// The host behind any thread of this state.
pub unsafe fn host_of(l: *mut lua_State) -> &'static mut Host {
    &mut **(lua_getextraspace(l) as *mut *mut Host)
}

unsafe extern "C" fn clock_hook(l: *mut lua_State, _ar: *mut lua_Debug) {
    let host = host_of(l);
    host.tickets -= 1;
    if host.tickets > 0 && !host.take_yield_request() && !host.slice_expired() {
        return;
    }
    if host.config.no_preempt {
        // Tickets still end the tick.
        return;
    }
    lua_markforcedyield(l);
    lua_yield(l, 0);
}

/// Annotation mismatches arrive here.
unsafe extern "C" fn warn_handler(_ud: *mut c_void, msg: *const c_char, _tocont: c_int) {
    if let Ok(text) = CStr::from_ptr(msg).to_str() {
        eprintln!("[warn] {text}");
    }
}

unsafe extern "C" fn lua_print(l: *mut lua_State) -> c_int {
    let n = lua_gettop(l);
    let mut out = String::new();
    for i in 1..=n {
        if i > 1 {
            out.push('\t');
        }
        out.push_str(&tostring_at(l, i));
    }
    println!("{out}");
    0
}

/// `tostring` without metamethods, for `print`.
unsafe fn tostring_at(l: *mut lua_State, idx: c_int) -> String {
    match lua_type(l, idx) {
        LUA_TNIL => "nil".into(),
        LUA_TBOOLEAN => if lua_toboolean(l, idx) != 0 {
            "true"
        } else {
            "false"
        }
        .into(),
        LUA_TNUMBER | LUA_TSTRING => {
            let mut len = 0usize;
            let p = lua_tolstring(l, idx, &mut len);
            if p.is_null() {
                String::new()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len))
                    .into_owned()
            }
        }
        t => {
            let name = CStr::from_ptr(lua_typename(l, t)).to_string_lossy();
            format!("{name}")
        }
    }
}

/// Pops the value at the top of the stack and renders it as an error message.
pub unsafe fn pop_string(l: *mut lua_State) -> String {
    let s = tostring_at(l, -1);
    lua_pop(l, 1);
    s
}

impl Vm {
    /// Reads a global as a number, for tests and diagnostics.
    pub fn global_number(&mut self, name: &str) -> Option<f64> {
        let n = CString::new(name).ok()?;
        unsafe {
            lua_getglobal(self.l, n.as_ptr());
            let mut ok: c_int = 0;
            let v = lua_tonumberx(self.l, -1, &mut ok);
            lua_pop(self.l, 1);
            (ok != 0).then_some(v)
        }
    }
}

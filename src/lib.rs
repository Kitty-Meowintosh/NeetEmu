//! NEET's computer runtime on the host: the YSLua interpreter and a reimplemented host API.

#![allow(clippy::missing_safety_doc)]

pub mod api;
pub mod clipboard;
pub mod disk;
pub mod display;
pub mod events;
pub mod ffi;
pub mod filehandle;
pub mod harness;
pub mod host;
pub mod input;
pub mod internet;
pub mod peripheral;
pub mod port;
pub mod screen;
pub mod serial;
pub mod vm;
pub mod world;

/// `_VERSION` and `YSLUA_RELEASE` as the linked interpreter reports them.
pub fn interpreter_version() -> (String, String) {
    unsafe {
        let l = ffi::luaL_newstate();
        ffi::luaL_openlibs(l);
        ffi::lua_getglobal(l, c"_VERSION".as_ptr());
        let mut len = 0usize;
        let p = ffi::lua_tolstring(l, -1, &mut len);
        let lua =
            String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned();
        ffi::lua_close(l);
        (lua, yslua_release())
    }
}

/// `YSLUA_RELEASE`, re-exported by build.rs.
fn yslua_release() -> String {
    format!(
        "YSLua {}.{}.{}",
        env!("YSLUA_VERSION_MAJOR"),
        env!("YSLUA_VERSION_MINOR"),
        env!("YSLUA_VERSION_RELEASE")
    )
}

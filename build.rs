use std::path::PathBuf;

/// Compiles YSLua's `lua/` tree directly.
fn main() {
    let lua_dir = PathBuf::from("third_party/Neet-YSLua/lua");
    if !lua_dir.join("lua.h").exists() {
        panic!("third_party/Neet-YSLua is empty — run `git submodule update --init`");
    }

    let mut sources: Vec<PathBuf> = std::fs::read_dir(&lua_dir)
        .expect("read lua/")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "c").then_some(p)
        })
        .filter(|p| {
            !matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some("lua.c" | "luac.c")
            )
        })
        .collect();
    sources.sort();

    let mut build = cc::Build::new();
    build
        .files(&sources)
        .include(&lua_dir)
        .std("c99")
        // As in lua/CMakeLists.txt's elseif(UNIX) branch.
        .define("LUA_USE_LINUX", None)
        .warnings(false)
        .flag_if_supported("-mmacosx-version-min=11.0");
    build.compile("lua55");

    export_yslua_version(&lua_dir.join("lua.h"));
    build_display();

    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=dl");
    println!("cargo:rerun-if-changed=third_party/Neet-YSLua/lua");
    for s in &sources {
        println!("cargo:rerun-if-changed={}", s.display());
    }
}

/// Compiles the SDL3 shim when SDL3 is installed, and sets `cfg(sdl)` when it is.
fn build_display() {
    println!("cargo:rerun-if-changed=csrc/display.c");
    println!("cargo:rerun-if-changed=csrc/menu.m");
    println!("cargo:rerun-if-env-changed=SDL3_DIR");
    println!("cargo:rustc-check-cfg=cfg(sdl)");

    let Some(prefix) = sdl3_prefix() else {
        println!("cargo:warning=SDL3 not found; neetemu will only run with --headless");
        return;
    };

    let mut build = cc::Build::new();
    build
        .file("csrc/display.c")
        .include(prefix.join("include"))
        .std("c99")
        .warnings(false)
        .flag_if_supported("-mmacosx-version-min=11.0");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build.file("csrc/menu.m");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
    build.compile("neetdisplay");

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=SDL3");
    println!("cargo:rustc-cfg=sdl");
}

/// Finds SDL3 through `SDL3_DIR`, then `pkg-config`, then the usual prefixes.
fn sdl3_prefix() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("SDL3_DIR") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(out) = std::process::Command::new("pkg-config")
        .args(["--variable=prefix", "sdl3"])
        .output()
    {
        if out.status.success() {
            candidates.push(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()));
        }
    }
    for dir in [
        "/opt/homebrew/opt/sdl3",
        "/opt/homebrew",
        "/usr/local",
        "/usr",
    ] {
        candidates.push(PathBuf::from(dir));
    }
    candidates
        .into_iter()
        .find(|p| p.join("include/SDL3/SDL.h").exists())
}

/// Re-exports `YSLUA_VERSION_*_N` from lua.h; the macro has no runtime accessor.
fn export_yslua_version(header: &std::path::Path) {
    let text = std::fs::read_to_string(header).expect("read lua.h");
    for part in ["MAJOR", "MINOR", "RELEASE"] {
        let needle = format!("#define YSLUA_VERSION_{part}_N");
        let value = text
            .lines()
            .find_map(|l| l.trim().strip_prefix(&needle))
            .unwrap_or_else(|| panic!("lua.h has no YSLUA_VERSION_{part}_N"))
            .trim();
        println!("cargo:rustc-env=YSLUA_VERSION_{part}={value}");
    }
    println!("cargo:rerun-if-changed={}", header.display());
}

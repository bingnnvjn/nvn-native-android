//! Compiles the two smoke shaders at build time (docs/ARCHITECTURE.md §3.6).
//!
//! SPIR-V is derived content and the red-line guard bans shader binaries in the
//! tree (`ci/guard/scan_tree.sh`: the `.spv` suffix and SPIR-V magic at offset
//! 0), so the product tree never commits one. The spike does the same build-time
//! step (`spike/abi-slice/build.rs`).
//!
//! A local build without `glslang-tools` can set `NVA_SKIP_SHADERS=1` to emit
//! empty files: enough to compile and run the non-Vulkan tests, and CI never
//! sets it.
//!
//! On Android the runtime also links `libandroid.so`: the window path takes its
//! own `ANativeWindow` reference (`src/window.rs`, #93), and that reference's
//! acquire/release live there. The host build never touches those symbols, so
//! the link is added only for the platform target.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=shaders/tri.vert");
    println!("cargo:rerun-if-changed=shaders/tri.frag");
    println!("cargo:rerun-if-env-changed=NVA_SKIP_SHADERS");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=android");
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    if env::var_os("NVA_SKIP_SHADERS").is_some() {
        for name in ["tri.vert.spv", "tri.frag.spv"] {
            std::fs::write(out_dir.join(name), []).expect("write the empty SPIR-V placeholder");
        }
        return;
    }

    for (source, target) in [
        ("shaders/tri.vert", "tri.vert.spv"),
        ("shaders/tri.frag", "tri.frag.spv"),
    ] {
        let output = out_dir.join(target);
        let status = Command::new("glslangValidator")
            .args(["-V", "--target-env", "vulkan1.1", source, "-o"])
            .arg(&output)
            .status()
            .unwrap_or_else(|error| {
                panic!(
                    "run glslangValidator for {source}: {error}; install glslang-tools, \
                     or set NVA_SKIP_SHADERS=1 for a compile-only build"
                )
            });
        assert!(status.success(), "glslangValidator failed for {source}");
    }
}

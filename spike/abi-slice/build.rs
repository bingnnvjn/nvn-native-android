use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if env::var_os("CARGO_FEATURE_VK").is_none() {
        return;
    }
    if env::var_os("NVA_SKIP_SHADERS").is_some() {
        // Local `cargo check` convenience only: CI never sets this.
        let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
        for dst in ["tri.vert.spv", "tri.frag.spv"] {
            let _ = std::fs::write(out.join(dst), []);
        }
        return;
    }
    println!("cargo:rerun-if-changed=shaders/tri.vert");
    println!("cargo:rerun-if-changed=shaders/tri.frag");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    for (src, dst) in [("shaders/tri.vert", "tri.vert.spv"), ("shaders/tri.frag", "tri.frag.spv")] {
        let outfile = out.join(dst);
        let status = Command::new("glslangValidator")
            .args(["-V", "--target-env", "vulkan1.1", src, "-o"])
            .arg(&outfile)
            .status()
            .unwrap_or_else(|e| panic!("run glslangValidator for {src}: {e}; install glslang-tools"));
        assert!(status.success(), "glslangValidator failed for {src}");
        println!("cargo:rerun-if-changed={src}");
    }
}

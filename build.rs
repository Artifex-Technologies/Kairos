//! Compiles shaders/*.comp to SPIR-V with glslc (shaderc) into OUT_DIR; the library embeds them.
use std::process::Command;

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    for name in ["luma", "motion", "smooth", "interp"] {
        let src = format!("shaders/{name}.comp");
        println!("cargo:rerun-if-changed={src}");
        let ok = Command::new("glslc")
            .args(["--target-env=vulkan1.0", "-O", &src, "-o", &format!("{out}/{name}.spv")])
            .status()
            .expect("glslc (from shaderc) is needed to build the shaders")
            .success();
        assert!(ok, "glslc failed on {src}");
    }
}

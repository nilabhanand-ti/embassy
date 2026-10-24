use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    #[cfg(feature = "am13e232n9")]
    let memory_x = include_bytes!("memory_am13e232n9.x");

    #[cfg(feature = "am13e23019")]
    let memory_x = include_bytes!("memory_am13e23019.x");

    fs::write(out.join("memory.x"), memory_x).unwrap();

    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=../link_ram_cortex_m.x");
    // copy main linker script.
    fs::write(out.join("link_ram.x"), include_bytes!("../link_ram_cortex_m.x")).unwrap();

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink_ram.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
    println!("cargo:rustc-link-arg-bins=-Tteleprobe.x");

    Ok(())
}

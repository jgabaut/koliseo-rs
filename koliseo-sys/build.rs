use std::env;
use std::path::PathBuf;

fn main() {
    let empty_dir = match std::fs::read_dir("./koliseo") {
        Ok(mut dir) => dir.next().is_none(),
        Err(_) => true,
    };
    if empty_dir {
        panic!("koliseo source does not exist in: `koliseo-sys/koliseo`.");
    }
    // Compile the C implementation.
    cc::Build::new()
        .file("koliseo/src/koliseo.c")
        .include("c")
        .compile("koliseo");

    // Generate Rust bindings from the C header.
    let bindings = bindgen::Builder::default()
        .header("koliseo/src/koliseo.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    // Write bindings to Cargo's generated output directory.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    // Rebuild if our C implementation changes.
    println!("cargo:rerun-if-changed=koliseo/src/koliseo.c");
    println!("cargo:rerun-if-changed=koliseo/src/koliseo.h");
}

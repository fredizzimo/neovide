extern crate gl_generator;

use gl_generator::*;
use std::env;
use std::fs::File;
use std::path::*;

fn main() {
    let target = env::var("TARGET").unwrap();
    let dest = PathBuf::from(&env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=build.rs");

    if target.contains("linux")
    {
        let mut file = File::create(&Path::new(&dest).join("glx.rs")).unwrap();
        Registry::new(Api::Glx, (1, 4), Profile::Core, Fallbacks::All, [
            "GLX_SGI_video_sync",
        ])
        .write_bindings(gl_generator::GlobalGenerator, &mut file)
        .unwrap();
    }
}

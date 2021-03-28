use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("elayday_descriptor.bin"))
        .compile(&["proto/elayday.proto"], &["proto"])
        .unwrap();
}

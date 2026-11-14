fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let descriptor = out.join("descriptor.bin");
    println!("cargo:rerun-if-changed=wear.proto");
    // Same prost/pbjson generation as AstroBox-NG-Module-Pb/build.rs.
    prost_build::Config::new()
        .protoc_executable(protoc_bin_vendored::protoc_bin_path().unwrap())
        .file_descriptor_set_path(&descriptor)
        .compile_protos(&["wear.proto"], &["."])
        .unwrap();
    pbjson_build::Builder::new()
        .register_descriptors(&std::fs::read(descriptor).unwrap())
        .unwrap()
        .build(&[".protocol"])
        .unwrap();
}

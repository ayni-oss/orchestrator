use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../../proto");
    let proto = root.join("ayni/orchestrator/events/v1/verification.proto");
    println!("cargo:rerun-if-changed={}", proto.display());
    let operation = root.join("ayni/orchestrator/events/v1/runtime_operation.proto");
    println!("cargo:rerun-if-changed={}", operation.display());
    let coding = root.join("ayni/orchestrator/events/v1/coding.proto");
    println!("cargo:rerun-if-changed={}", coding.display());
    let descriptor = PathBuf::from(env::var("OUT_DIR")?).join("events.bin");
    prost_build::Config::new()
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .file_descriptor_set_path(&descriptor)
        .compile_protos(&[proto, operation, coding], &[root])?;
    pbjson_build::Builder::new()
        .register_descriptors(&fs::read(descriptor)?)?
        .build(&[".ayni.orchestrator.events.v1"])?;
    Ok(())
}

use prost::Message as _;
use std::{env, error::Error, fs, path::PathBuf};
fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../../proto");
    let proto = root.join("ayni/orchestrator/v1/telemetry.proto");
    let output = PathBuf::from(env::var("OUT_DIR")?);
    let descriptor = output.join("telemetry.bin");
    let mut prost = prost_build::Config::new();
    prost.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    tonic_prost_build::configure()
        .file_descriptor_set_path(&descriptor)
        .compile_with_config(prost, &[&proto], &[&root])?;
    let bytes = fs::read(descriptor)?;
    pbjson_build::Builder::new()
        .register_descriptors(&bytes)?
        .build(&[".ayni.orchestrator.v1"])?;
    let descriptor = prost_types::FileDescriptorSet::decode(bytes.as_slice())?;
    let names: Vec<_> = descriptor
        .file
        .iter()
        .flat_map(|f| &f.message_type)
        .filter_map(|m| m.name.as_ref())
        .collect();
    fs::write(
        output.join("messages.rs"),
        format!("pub const MESSAGES: &[&str] = &{names:?};"),
    )?;
    println!("cargo:rerun-if-changed={}", proto.display());
    Ok(())
}

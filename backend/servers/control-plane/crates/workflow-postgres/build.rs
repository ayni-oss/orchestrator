use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let proto = "proto/watch.proto";
    println!("cargo:rerun-if-changed={proto}");
    let mut prost = prost_build::Config::new();
    prost.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    tonic_prost_build::configure().compile_with_config(prost, &[proto], &["proto"])?;
    Ok(())
}

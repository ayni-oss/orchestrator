use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
};

fn collect_proto_sources(directory: &Path, sources: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_proto_sources(&path, sources)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "proto")
        {
            sources.push(path);
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing manifest")?);
    let proto_root = manifest_dir.join("../../proto");
    let mut sources = Vec::new();
    collect_proto_sources(&proto_root, &mut sources)?;
    if sources.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no protobuf sources found").into());
    }

    let descriptor = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing output directory")?)
        .join("ayni_orchestrator_v1_descriptor.bin");

    let mut prost = prost_build::Config::new();
    prost.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    tonic_prost_build::configure()
        .file_descriptor_set_path(descriptor)
        .compile_with_config(
            prost,
            &sources,
            &[proto_root.clone(), protoc_bin_vendored::include_path()?],
        )?;

    println!("cargo:rerun-if-changed={}", proto_root.display());
    for source in sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    Ok(())
}

use std::{env, error::Error, io, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("CARGO_MANIFEST_DIR is not set"))?;
    let workspace_root = manifest_dir
        .parent()
        .and_then(|crates_dir| crates_dir.parent())
        .ok_or_else(|| {
            io::Error::other("agentro-proto is not under the workspace crates directory")
        })?;
    let proto_root = workspace_root.join("proto");
    let protos = [
        proto_root.join("agentro/common/v1/capability.proto"),
        proto_root.join("agentro/common/v1/error.proto"),
        proto_root.join("agentro/common/v1/pagination.proto"),
        proto_root.join("agentro/common/v1/resource.proto"),
        proto_root.join("agentro/execution/v1/run_service.proto"),
        proto_root.join("agentro/schedule/v1/schedule_service.proto"),
        proto_root.join("agentro/system/v1/system_service.proto"),
        proto_root.join("agentro/workflow/v1/workflow_service.proto"),
        proto_root.join("agentro/workspace/v1/workspace_service.proto"),
    ];
    let protoc_include = protoc_bin_vendored::include_path()?;
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let out_dir = env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("OUT_DIR is not set"))?;

    let mut prost_config = tonic_prost_build::Config::new();
    prost_config.protoc_executable(protoc);
    prost_config.include_file("agentro_modules.rs");
    prost_config.file_descriptor_set_path(out_dir.join("agentro_descriptor.bin"));

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(false)
        .compile_with_config(prost_config, &protos, &[proto_root.clone(), protoc_include])?;

    println!("cargo:rerun-if-changed={}", proto_root.display());
    Ok(())
}

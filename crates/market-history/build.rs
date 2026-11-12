fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    config.btree_map(["."]);
    tonic_prost_build::configure()
        .build_client(false)
        .build_server(false)
        .extern_path(".market", "::hub_protocol::wire")
        .compile_with_config(
            config,
            &[std::path::Path::new("../../archive.proto")],
            &[std::path::Path::new("../..")],
        )?;
    println!("cargo:rerun-if-changed=../../archive.proto");
    Ok(())
}

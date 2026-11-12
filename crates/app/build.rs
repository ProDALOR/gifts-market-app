fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    config.btree_map(["."]);
    tonic_prost_build::configure()
        .extern_path(".market", "::hub_protocol::wire")
        .compile_with_config(
            config,
            &[std::path::Path::new("../../api.proto")],
            &[std::path::Path::new("../..")],
        )?;
    println!("cargo:rerun-if-changed=../../api.proto");
    Ok(())
}

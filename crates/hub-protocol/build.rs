fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    config.btree_map(["."]);
    config.type_attribute("market.NodeRole", "#[derive(serde::Serialize, serde::Deserialize)]\n#[serde(try_from = \"i32\", into = \"i32\")]" );
    tonic_prost_build::configure()
        .emit_rerun_if_changed(false)
        .compile_with_config(config, &["../../market-hub.proto"], &["../.."])?;
    println!("cargo:rerun-if-changed=../../market-hub.proto");
    Ok(())
}

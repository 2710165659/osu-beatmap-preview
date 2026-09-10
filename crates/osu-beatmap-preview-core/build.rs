fn main() {
    println!("cargo:rustc-env=VERGEN_BUILD_TIMESTAMP=1970-01-01T00:00:00Z");

    let manifest_dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    // render 与 skin 是 core 和 CLI 的共享配置，统一放在工作区外层 assets。
    let config_path = manifest_dir.join("../../assets/shared_config.yml");
    println!("cargo:rerun-if-changed={}", config_path.display());
    let source = std::fs::read_to_string(config_path).expect("默认配置必须可读");
    let value: serde_json::Value = serde_yaml::from_str(&source).expect("默认配置必须是有效 YAML");
    let mut core_value = serde_json::Map::new();
    let object = value.as_object().expect("默认配置根节点必须是对象");
    for key in ["render", "skin"] {
        let section = object
            .get(key)
            .cloned()
            .expect("默认配置必须包含 core 配置段");
        core_value.insert(key.to_string(), section);
    }
    let json = serde_json::to_string(&serde_json::Value::Object(core_value))
        .expect("默认配置必须可转换为 JSON");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("default_config.json"), json).expect("必须写入嵌入式默认配置");
}

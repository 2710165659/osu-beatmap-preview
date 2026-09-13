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

    generate_hitsound_assets(&manifest_dir, &out_dir);
}

/// 把 `assets/hitsound/*.ogg` 内嵌进 core。
///
/// 这样 WASM 产物自带打击音资源，宿主不需要再向站点请求音效文件（也就不存在
/// 「忘记同步静态副本导致全部 404」的问题）；CLI 也用同一份来源，两边不会走偏。
/// 资源本身是压缩过的 ogg，36 个文件合计约 240 KiB。
fn generate_hitsound_assets(manifest_dir: &std::path::Path, out_dir: &std::path::Path) {
    let hitsound_dir = manifest_dir.join("../../assets/hitsound");
    println!("cargo:rerun-if-changed={}", hitsound_dir.display());

    let mut entries: Vec<(String, std::path::PathBuf)> = Vec::new();
    if hitsound_dir.is_dir() {
        for entry in std::fs::read_dir(&hitsound_dir).expect("打击音目录必须可读") {
            let path = entry.expect("目录项必须可读").path();
            if path.extension().and_then(|value| value.to_str()) != Some("ogg") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            println!("cargo:rerun-if-changed={}", path.display());
            entries.push((stem.to_string(), path));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut generated = String::from(
        "// 由 build.rs 自动生成的内嵌打击音资源表，请勿手动修改。\n\n\
         /// 内嵌的打击音 ogg 字节；名称为不带扩展名的文件名。\n\
         pub static HITSOUND_ASSETS: &[(&str, &[u8])] = &[\n",
    );
    for (name, path) in &entries {
        generated.push_str(&format!(
            "    ({:?}, include_bytes!({:?})),\n",
            name,
            path.to_string_lossy()
        ));
    }
    generated.push_str("];\n");
    std::fs::write(out_dir.join("hitsound_assets.rs"), generated)
        .expect("必须写入内嵌打击音资源表");
}

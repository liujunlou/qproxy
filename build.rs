use std::process::Command;

fn main() {
    // let out_dir = "./src/pb";

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let descriptor_path = std::path::PathBuf::from(out_dir).join("route_descriptor.bin");

    tonic_build::configure()
        .emit_rerun_if_changed(true)
        .build_server(true)
        .build_client(true)
        // 生成描述符文件，供 tonic-reflection 使用
        .file_descriptor_set_path(&descriptor_path)
        .compile(&["proto/route.proto"], &["proto"])
        .unwrap();
    Command::new("cargo").arg("fmt").output().unwrap();

    println!("cargo:rerun-if-changed=proto/route.proto");
    println!("cargo:rerun-if-changed=build.rs");
}

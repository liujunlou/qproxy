use std::process::Command;

fn main() {
    // let out_dir = "./src/pb";

    tonic_build::configure()
        .emit_rerun_if_changed(true)
        .build_server(true)
        .build_client(true)
        // .out_dir(out_dir)
        // 生成描述符文件，当使用gRPC Reflection功能时，可以从这个文件中获取服务描述信息来返回给调用者
        // .file_descriptor_set_path(out_dir.join("getting_descriptor.bin"))
        .compile(&["proto/route.proto"], &["proto"])
        .unwrap();
    Command::new("cargo").arg("fmt").output().unwrap();

    println!("cargo:rerun-if-changed=proto/route.proto");
    println!("cargo:rerun-if-changed=build.rs");
}

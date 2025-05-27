fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .compile(&["proto/route.proto"], &["proto"])?;
    
    // 确保在 proto 文件改变时重新编译
    println!("cargo:rerun-if-changed=proto/route.proto");
    println!("cargo:rerun-if-changed=build.rs");
    
    Ok(())
} 
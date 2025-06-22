#!/bin/bash

# 跨平台编译脚本
set -e

echo "开始跨平台编译 QProxy..."

# 创建输出目录
mkdir -p dist

# 编译当前平台（macOS ARM64）
echo "编译 macOS ARM64..."
cargo build --release --target aarch64-apple-darwin
cp target/aarch64-apple-darwin/release/qproxy dist/qproxy-macos-aarch64

# 编译 Linux x86_64（需要 Docker）
echo "编译 Linux x86_64..."
if command -v docker &> /dev/null; then
    docker run --rm -v "$(pwd)":/app -w /app rust:latest cargo build --release --target x86_64-unknown-linux-gnu
    cp target/x86_64-unknown-linux-gnu/release/qproxy dist/qproxy-linux-x86_64
else
    echo "警告: Docker 未安装，跳过 Linux 编译"
fi

# 编译 Linux ARM64（需要 Docker）
echo "编译 Linux ARM64..."
if command -v docker &> /dev/null; then
    docker run --rm -v "$(pwd)":/app -w /app rust:latest cargo build --release --target aarch64-unknown-linux-gnu
    cp target/aarch64-unknown-linux-gnu/release/qproxy dist/qproxy-linux-aarch64
else
    echo "警告: Docker 未安装，跳过 Linux ARM64 编译"
fi

# 编译 Windows x86_64（需要 Docker）
echo "编译 Windows x86_64..."
if command -v docker &> /dev/null; then
    docker run --rm -v "$(pwd)":/app -w /app rust:latest cargo build --release --target x86_64-pc-windows-msvc
    cp target/x86_64-pc-windows-msvc/release/qproxy.exe dist/qproxy-windows-x86_64.exe
else
    echo "警告: Docker 未安装，跳过 Windows 编译"
fi

# 创建压缩包
echo "创建发布包..."
cd dist
tar -czf qproxy-release.tar.gz *
zip -r qproxy-release.zip *

echo "编译完成！"
echo "可执行文件位于 dist/ 目录"
ls -la dist/ 
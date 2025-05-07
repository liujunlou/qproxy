#!/bin/bash
set -e

# 安装依赖工具（如果尚未安装）
if ! command -v grcov &> /dev/null; then
    echo "Installing grcov..."
    cargo install grcov
fi

# 创建覆盖率报告输出目录
mkdir -p target/coverage

# 设置Rust环境变量以收集覆盖率数据
export CARGO_INCREMENTAL=0
export RUSTFLAGS="-Cinstrument-coverage"
export LLVM_PROFILE_FILE="target/coverage/cargo-test-%p-%m.profraw"

# 清理并构建项目
cargo clean
cargo build

# 运行测试
echo "Running tests with coverage..."
cargo test

# 生成HTML覆盖率报告
echo "Generating coverage report..."
grcov . --binary-path ./target/debug/ -s . -t html --branch --ignore-not-existing -o target/coverage/html

# 生成LCOV格式覆盖率报告（可用于CI服务集成）
grcov . --binary-path ./target/debug/ -s . -t lcov --branch --ignore-not-existing -o target/coverage/lcov.info

# 打印覆盖率报告摘要
echo "Coverage report generated at target/coverage/html/index.html"

# 在浏览器中打开覆盖率报告（macOS）
if [[ "$OSTYPE" == "darwin"* ]]; then
    open target/coverage/html/index.html
fi 
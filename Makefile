# QProxy Makefile

.PHONY: help build release clean test cross-compile docker-build

# 默认目标
help:
	@echo "QProxy 构建工具"
	@echo ""
	@echo "可用命令:"
	@echo "  build        - 编译调试版本"
	@echo "  release      - 编译发布版本（当前平台）"
	@echo "  clean        - 清理构建文件"
	@echo "  test         - 运行测试"
	@echo "  cross-compile - 跨平台编译（需要 Docker）"
	@echo "  docker-build - 使用 Docker 进行跨平台编译"
	@echo "  install      - 安装到系统"

# 调试版本
build:
	cargo build

# 发布版本（当前平台）
release:
	cargo build --release
	@echo "发布版本已编译到: target/release/qproxy"

# 清理
clean:
	cargo clean
	rm -rf dist/

# 测试
test:
	cargo test

# 跨平台编译（使用脚本）
cross-compile:
	./scripts/cross-compile.sh

# Docker 跨平台编译
docker-build:
	@echo "使用 Docker 进行跨平台编译..."
	@echo "编译 Linux x86_64..."
	docker run --rm -v "$(PWD)":/app -w /app rust:latest cargo build --release --target x86_64-unknown-linux-gnu
	@echo "编译 Linux ARM64..."
	docker run --rm -v "$(PWD)":/app -w /app rust:latest cargo build --release --target aarch64-unknown-linux-gnu
	@echo "编译 Windows x86_64..."
	docker run --rm -v "$(PWD)":/app -w /app rust:latest cargo build --release --target x86_64-pc-windows-msvc
	@echo "Docker 编译完成！"

# 安装到系统
install: release
	cp target/release/qproxy /usr/local/bin/
	@echo "QProxy 已安装到 /usr/local/bin/"

# 创建发布包
package: release
	mkdir -p dist
	cp target/release/qproxy dist/
	cp config*.json dist/ 2>/dev/null || true
	cp README.md dist/ 2>/dev/null || true
	cd dist && tar -czf qproxy-$(shell date +%Y%m%d).tar.gz *
	@echo "发布包已创建: dist/qproxy-$(shell date +%Y%m%d).tar.gz"

# 检查依赖
check-deps:
	@echo "检查 Rust 工具链..."
	rustc --version
	cargo --version
	@echo "检查目标平台..."
	rustup target list --installed
	@echo "检查 Docker（用于跨平台编译）..."
	@if command -v docker &> /dev/null; then echo "Docker 已安装"; else echo "Docker 未安装"; fi 
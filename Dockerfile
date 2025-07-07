# 多阶段构建 - 构建阶段
FROM rust:1.82-slim as builder

# 安装构建依赖
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# 设置工作目录
WORKDIR /app

# 复制所有源代码
COPY . .

# 删除现有的Cargo.lock并重新生成
RUN rm -f Cargo.lock && cargo generate-lockfile

# 构建应用
RUN cargo build --release

# 运行阶段
FROM debian:bookworm-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# 创建非root用户
# RUN groupadd -r qproxy && useradd -r -g qproxy qproxy

# 设置工作目录
WORKDIR /app

# 从构建阶段复制二进制文件
COPY --from=builder /app/target/release/qproxy /app/qproxy

# 复制配置文件
COPY config.json /app/config.json

# 创建日志目录
RUN mkdir -p /app/logs
# && chown -R qproxy:qproxy /app

# 切换到非root用户
# USER qproxy

# 暴露端口，这里通过docker启动指令来暴露端口
# EXPOSE 8080 8082 8084 8085

# 健康检查
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# 启动命令，默认加载config.json配置文件，或通过环境变量CONFIG_PATH指定配置文件
CMD ["/app/qproxy", "--config", "/app/config.json"] 
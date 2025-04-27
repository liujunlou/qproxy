# QProxy

QProxy 是一个支持 HTTP/HTTPS 和 TCP 协议的代理服务器，具有流量录制和回放功能。它可以在一个可用区录制流量，并在另一个可用区回放流量。

## 功能特性

- 支持 HTTP/HTTPS 代理
- 支持 TCP 代理（可选 TLS）
- 流量录制功能
- 流量回放功能
- 跨可用区部署

## 配置说明

配置文件使用 JSON 格式，默认名为 `config.json`。你也可以通过环境变量 `CONFIG_PATH` 指定配置文件路径。

配置文件示例：
```json
{
    "http": {
        "host": "127.0.0.1",
        "port": 8080,
        "downstream": "http://localhost:8081"
    },
    "tcp": {
        "host": "127.0.0.1",
        "port": 8082,
        "downstream": ["localhost:8083"],
        "tls": {
            "tls_cert": "cert.pem",
            "tls_key": "key.pem"
        }
    },
    "mode": "Record",
    "peer": {
        "host": "remote-host",
        "port": 8084,
        "tls": true
    }
}
```

## 部署说明

1. 在录制节点：
   - 设置 `mode` 为 `"Record"`
   - 配置 `peer` 指向回放节点
   - 配置 `http` 和 `tcp` 的下游服务器地址

2. 在回放节点：
   - 设置 `mode` 为 `"Playback"`
   - 不需要配置 `peer`
   - 配置监听地址和端口

## 使用方法

1. 编译项目：
   ```bash
   cargo build --release
   ```

2. 准备配置文件：
   - 复制 `config.json` 到适当位置
   - 根据需要修改配置

3. 运行服务：
   ```bash
   # 使用默认配置文件
   ./target/release/qproxy

   # 使用指定配置文件
   CONFIG_PATH=/path/to/config.json ./target/release/qproxy
   ```

## TLS 配置

如果需要使用 TLS，请准备好证书文件：

1. 生成自签名证书：
   ```bash
   openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes
   ```

2. 在配置文件中指定证书路径：
   ```json
   "tls": {
       "tls_cert": "cert.pem",
       "tls_key": "key.pem"
   }
   ```

## 注意事项

1. 确保防火墙允许相关端口的访问
2. 在生产环境中使用时，建议：
   - 使用正式的 SSL 证书
   - 配置适当的访问控制
   - 监控系统资源使用情况 

## 项目架构

QProxy 采用模块化设计，主要包含以下组件：

- **proxy**: 核心代理模块，处理 HTTP/HTTPS 和 TCP 代理功能
- **playback**: 流量回放服务，负责存储和重放录制的流量
- **sync**: 同步服务，负责在录制节点和回放节点之间同步流量数据
- **api**: HTTP API 接口，用于查询和管理流量记录
- **service_discovery**: 服务发现模块，用于管理下游服务地址

## API 接口

QProxy 提供以下 HTTP API 接口：

### 流量记录查询
```
GET /api/records
查询已录制的流量记录

参数：
- start: 开始时间（ISO 8601 格式）
- end: 结束时间（ISO 8601 格式）
- limit: 返回记录数量限制
```

### 流量记录删除
```
DELETE /api/records/{id}
删除指定的流量记录
```

### 系统状态查询
```
GET /api/status
查询系统运行状态，包括：
- 当前模式（录制/回放）
- 已录制流量数量
- 系统资源使用情况
```

## 性能优化

1. 内存使用
   - 流量数据采用分片存储，避免单个记录过大
   - 使用内存池复用缓冲区，减少内存分配
   - 定期清理过期数据

2. CPU 优化
   - 使用异步 I/O 处理网络请求
   - 采用工作线程池处理计算密集型任务
   - 使用零拷贝技术优化数据传输

3. 网络优化
   - 支持 HTTP/2 以减少连接数
   - 使用连接池复用下游连接
   - 支持流量压缩

## 监控指标

QProxy 提供以下监控指标：

1. 基础指标
   - QPS（每秒请求数）
   - 响应时间分布
   - 错误率

2. 资源指标
   - CPU 使用率
   - 内存使用量
   - 网络 I/O

3. 业务指标
   - 录制流量大小
   - 回放成功率
   - 同步延迟

## 常见问题

1. **连接超时**
   - 检查网络连接是否正常
   - 确认下游服务是否可用
   - 调整超时配置参数

2. **内存使用过高**
   - 调整流量记录保留时间
   - 减少并发连接数限制
   - 开启数据压缩

3. **同步失败**
   - 检查节点间网络连接
   - 确认 TLS 证书配置
   - 查看同步服务日志

## 贡献指南

欢迎提交 Pull Request 或 Issue。在提交代码时请注意：

1. 遵循项目代码风格
2. 添加必要的测试用例
3. 更新相关文档
4. 提供清晰的提交信息

## 开源协议

本项目采用 MIT 协议开源。 
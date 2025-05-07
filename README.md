# QProxy

QProxy 是一个支持 HTTP/HTTPS 和 TCP 协议的代理服务器，具有流量录制和回放功能。它可以在一个可用区录制流量，并在另一个可用区回放流量。

## 功能特性

- 支持 HTTP/HTTPS 代理
- 支持 TCP 代理（可选 TLS）
- 流量录制功能
- 流量回放功能
- 跨可用区部署
- MQTT 客户端支持

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

## MQTT 客户端使用

QProxy 提供了完整的 MQTT 客户端实现，支持发布、订阅和查询功能。以下是使用示例：

```rust
use qproxy::mqtt_client::{
    client::{MqttClient, MqttConfig},
    message::QoS,
};
use bytes::Bytes;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建 MQTT 客户端配置
    let config = MqttConfig {
        client_id: "my-client".to_string(),
        host: "mqtt.example.com".to_string(),
        port: 1883,
        keep_alive: 60,
        app_id: Some("my-app".to_string()),
        token: Some("auth-token".to_string()),
        ..Default::default()
    };
    
    // 创建 MQTT 客户端
    let mut client = MqttClient::new(config);
    
    // 连接到 MQTT 服务器
    client.connect().await?;
    
    // 发布消息
    let topic = "my/topic".to_string();
    let payload = Bytes::from("Hello, MQTT!");
    client.publish(topic, payload, QoS::AtLeastOnce).await?;
    
    // 订阅主题
    client.subscribe("notifications/#".to_string(), QoS::AtMostOnce).await?;
    
    // 查询主题
    let query_topic = "query/status".to_string();
    let query_payload = Bytes::from("query data");
    let response = client.query(query_topic, query_payload).await?;
    
    // 处理响应
    println!("Query response: {:?}", response);
    
    // 断开连接
    client.disconnect().await?;
    
    Ok(())
}
```

MQTT 客户端支持以下主要功能：
- 连接和断开连接
- 消息发布（支持 QoS 0/1/2）
- 主题订阅与取消订阅
- 主题查询
- 自动重连
- 会话保持

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
- **mqtt-client**: MQTT 客户端模块，支持 MQTT 3.1.1 协议，提供消息发布、订阅和查询功能

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

## 测试

QProxy 提供了完整的测试用例，包括单元测试和集成测试。

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行特定测试
cargo test playback_test
cargo test proxy_test
cargo test record_test
cargo test mqtt_test
cargo test mqtt_integration_test
```

### 测试依赖

测试需要以下本地依赖：

- Redis 服务器（用于测试回放和录制功能）
- 确保本地 Redis 服务器在默认端口 6379 运行
- 对于 MQTT 集成测试，需要确保本地网络端口可访问（测试将启动模拟 MQTT 服务器）

### 代码覆盖率

QProxy 支持生成代码覆盖率报告，以帮助识别未被测试覆盖的代码路径。

#### 生成覆盖率报告

我们提供了一个脚本来自动生成覆盖率报告：

```bash
# 确保脚本有执行权限
chmod +x coverage.sh

# 运行覆盖率测试
./coverage.sh
```

这将生成 HTML 格式的覆盖率报告，并自动在浏览器中打开。报告位于 `target/coverage/html/index.html`。

#### 覆盖率报告依赖

生成覆盖率报告需要以下工具：

- grcov（覆盖率报告生成工具，脚本会自动安装）
- LLVM（一般与 Rust 工具链一起安装）

## 贡献指南

欢迎提交 Pull Request 或 Issue。在提交代码时请注意：

1. 遵循项目代码风格
2. 添加必要的测试用例
3. 确保测试覆盖率不下降
4. 更新相关文档
5. 提供清晰的提交信息

## 开源协议

本项目采用 MIT 协议开源。 
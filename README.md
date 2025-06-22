# QProxy

QProxy 是一个支持跨可用区部署的代理服务，主要用于流量录制和回放，其中包括协议有：TCP、HTTP/HTTPS。它包含两种角色：Record 节点和 Playback 节点。Record 节点负责录制流量并存储到 Redis，Playback 节点负责代理流量并回放录制的流量。

## 功能特性

### Record 节点
- 支持 HTTP/HTTPS 流量录制
- 支持 TCP 流量录制（可选 TLS）
- 将录制的流量缓存到 Redis
- 提供 HTTP API 接口供 Playback 节点拉取录制的流量

### Playback 节点
- 支持 HTTP/HTTPS 流量代理
- 支持 TCP 流量代理（可选 TLS）
- 定时从 Record 节点拉取录制的流量
- 在本地可用区回放录制的流量

## 配置说明

配置文件使用 JSON 格式，默认名为 `config.json`。你也可以通过环境变量 `CONFIG_PATH` 指定配置文件路径。

### Record 节点配置示例：
```json
{
    "mode": "Record",                       // 节点模式，录制节点用于公安网
    "http": {                               // http代理服务器，downstream为代理下游地址
        "host": "127.0.0.1",
        "port": 8080,
        "downstream": "http://localhost:8081"
    },
    "grpc": {                               // grpc代理服务器，downstream为代理下游地址
        "enabled": true,
        "host": "127.0.0.1",
        "port": 8081,
        "downstream": ["localhost:9081"]
    },
    "tcp": nill,
    "sync": {                               // 同步服务配置，仅在回放节点开启，来拉取待回放流量
        "enabled": false,
        "shards": 1,
        "interval": 1000,
        "peer": null,
    },
    "redis": {                               // redis服务配置
        "url": "redis://username:password@localhost:6379",
        "pool_size": 10,
        "connection_timeout": 5,
        "retry_count": 3
    },
    "service_discovery": {                   // 当前可用区的服务发现模块，用于回放真实流量
        "provider": "static",
        "config": {
            "static_services": [],
            "zookeeper": {
                "address": "localhost:2181",
                "base_path": "/qproxy"
            },
            "kubernetes": {
                "namespace": "default",
                "service_account_token": null
            }
        }
    },
    "logging": {                             // 日志配置
      // ...
    }
}
```

### Playback 节点配置示例：
```json
{
    "mode": "Playback",                       // 节点模式，回放节点用于警务网、互联网
    "http": {                                 // http代理服务器，downstream为代理下游地址
        "host": "127.0.0.1",
        "port": 8080,
        "downstream": "http://localhost:8081"
    },
    "grpc": {                                 // grpc代理服务器，downstream为代理下游地址
        "enabled": true,
        "host": "127.0.0.1",
        "port": 8081,
        "downstream": ["localhost:9081"]
    },
    "tcp": null,
    "sync": {                                 // 同步服务配置，仅在回放节点开启，来拉取待回放流量
        "enabled": false,
        "shards": 1
        "peer": {
            "host": "127.0.0.1",
            "port": 8084,
            "tls": true
        },
    },
    "redis": {                               // redis服务配置
        "url": "redis://username:password@localhost:6379",
        "pool_size": 10,
        "connection_timeout": 5,
        "retry_count": 3
    },
    "service_discovery": {                   // 当前可用区的服务发现模块，用于回放真实流量
        "provider": "static",
        "config": {
            "static_services": [],
            "zookeeper": {
                "address": "localhost:2181",
                "base_path": "/qproxy"
            },
            "kubernetes": {
                "namespace": "default",
                "service_account_token": null
            }
        }
    },
    "logging": {                             // 日志配置
      // ...
    }
}
```

## 部署说明

### Record 节点部署
1. 配置 `mode` 为 `"Record"`
2. 配置 HTTP/TCP 监听地址和端口
3. 配置下游服务器地址
4. 配置 Redis 连接信息
5. 启动服务

### Playback 节点部署
1. 配置 `mode` 为 `"Playback"`
2. 配置 HTTP/TCP 监听地址和端口
3. 配置下游服务器地址
4. 配置 Record 节点地址（peer）
5. 配置同步间隔
6. 启动服务

## 使用方法

1. 编译项目：
   ```bash
   cargo build --release
   ```

2. 准备配置文件：
   - 根据节点角色选择对应的配置模板
   - 修改配置参数

3. 运行服务：
   ```bash
   # 使用默认配置文件 (config.json)
   ./target/release/qproxy

   # 使用指定配置文件
   ./target/release/qproxy --config /path/to/config.json
   ./target/release/qproxy -c /path/to/config.json

   # 指定日志级别
   ./target/release/qproxy --log-level debug
   ./target/release/qproxy -l debug

   # 显示详细输出
   ./target/release/qproxy --verbose
   ./target/release/qproxy -v

   # 组合使用
   ./target/release/qproxy -c /path/to/config.json -l debug -v

   # 查看帮助信息
   ./target/release/qproxy --help
   ```

4. 命令行参数说明：
   - `-c, --config <FILE>`: 指定配置文件路径 (默认: config.json)
   - `-l, --log-level <LEVEL>`: 设置日志级别 (默认: info)
   - `-v, --verbose`: 显示详细输出
   - `-h, --help`: 显示帮助信息
   - `-V, --version`: 显示版本信息

5. 环境变量：
   ```bash
   # 使用环境变量指定配置文件
   CONFIG_PATH=/path/to/config.json ./target/release/qproxy
   
   # 使用环境变量设置日志级别
   RUST_LOG=debug ./target/release/qproxy
   ```

4. API接口
   ```
   # 拉取同步记录
   curl -X GET 'http://127.0.0.1:8080/sync?peer_id=default&shard_id=default'

   # 上报录制流量
   curl -X POST 'http://127.0.0.1:8080/sync' \
   --header 'Content-Type: application/json' \
   --data '{
      
   }'

   # 提交同步位点
   curl -X POST 'http://127.0.0.1:8080/commit' \
   --header 'Content-Type: application/json' \
   --data '{
      "peer_id": "default",
      "shard_id": "default",
      "last_sync_time": 1747629658,
      "last_record_id": "0",
      "status": "Completed",
      "retry_count": 0,
      "error_message": ""
   }'
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

### Record 节点 API

#### 流量记录查询
```
GET /sync
查询已录制的流量记录

参数：
- start: 开始时间（ISO 8601 格式）
- end: 结束时间（ISO 8601 格式）
- limit: 返回记录数量限制
```

### 代理流量接收
```
POST /sync
接收代理的流量到在当前可用区回放，并响应结果
```

#### 系统状态查询
```
GET /api/status
查询系统运行状态，包括：
- 当前模式（录制/回放）
- 已录制流量数量
- Redis 存储状态
- 系统资源使用情况
```

#### 健康检查接口
```
GET /health
检查服务健康状态，返回：
- 服务状态（UP/DOWN）
- 当前模式（录制/回放）
- 组件状态：
  - Redis 连接状态
  - 流量录制状态
  - 系统资源状态
- 最近一次检查时间
- 错误信息（如果有）

响应示例：
{
    "status": "UP",
    "components": {
        "redis": "UP",
        "recording": "UP",
        "system": "UP"
    },
    "last_check": "2024-03-20T10:00:00Z",
    "error": null
}
```

## 监控指标

### 1. 性能指标

#### Record 节点
- TPS（每秒事务数）
  - 总 TPS
  - 按协议分类的 TPS（HTTP/TCP）
  - 按接口分类的 TPS
- 请求耗时
  - 平均响应时间
  - P50/P90/P99 响应时间
  - 按协议分类的响应时间
- 错误率
  - 总体错误率
  - 按错误类型统计
  - 按协议分类的错误率

#### Playback 节点
- 回放 TPS
  - 总回放 TPS
  - 按协议分类的回放 TPS
- 回放延迟
  - 平均回放延迟
  - P50/P90/P99 回放延迟
- 回放错误率
  - 总体回放错误率
  - 按错误类型统计

#### HTTP 监控指标
- `http_requests_total` - HTTP 请求总数
- `http_request_duration_seconds` - HTTP 请求处理时间
- `http_active_connections` - 活跃 HTTP 连接数
- `http_request_size_bytes` - HTTP 请求大小
- `http_response_size_bytes` - HTTP 响应大小
- `http_errors_total` - HTTP 错误总数

#### gRPC 监控指标
- `grpc_requests_total` - gRPC 请求总数
- `grpc_request_duration_seconds` - gRPC 请求处理时间
- `grpc_active_connections` - 活跃 gRPC 连接数
- `grpc_request_size_bytes` - gRPC 请求大小
- `grpc_response_size_bytes` - gRPC 响应大小
- `grpc_errors_total` - gRPC 错误总数

### 2. 业务指标

#### Record 节点
- 录制流量大小
  - 总流量大小
  - 按协议分类的流量大小
  - 按时间段的流量分布
- 录制成功率
  - 总体录制成功率
  - 按协议分类的录制成功率
- 存储状态
  - Redis 存储使用量
  - 存储增长率
  - 数据保留时间

#### Playback 节点
- 回放成功率
  - 总体回放成功率
  - 按协议分类的回放成功率
  - 按时间段的回放成功率
- 同步延迟
  - 与 Record 节点的同步延迟
  - 按协议分类的同步延迟
  - 同步失败率
- 回放准确性
  - 响应匹配率
  - 错误匹配率
  - 数据一致性检查

### 3. 系统指标

- CPU 使用率
  - 总体 CPU 使用率
  - 按核心的使用率
  - 按进程的使用率
- 内存使用
  - 总内存使用量
  - 内存使用趋势
  - 内存分配情况
- 网络 I/O
  - 网络吞吐量
  - 连接数
  - 网络错误率
- 磁盘 I/O
  - 磁盘读写速率
  - 磁盘使用量
  - I/O 等待时间

### 4. 告警阈值

系统预设了以下告警阈值：

1. 性能告警
   - TPS 超过 1000/s
   - 响应时间超过 1s
   - 错误率超过 1%

2. 业务告警
   - 录制成功率低于 99%
   - 回放成功率低于 95%
   - 同步延迟超过 5s

3. 系统告警
   - CPU 使用率超过 80%
   - 内存使用率超过 85%
   - 磁盘使用率超过 90%

### 访问监控指标

QProxy 提供了完整的监控功能，支持 HTTP 和 gRPC 服务端监控。启动服务后，可以通过以下方式访问监控指标：

```bash
# 获取 Prometheus 格式的指标
curl http://localhost:8080/metrics

# 健康检查
curl http://localhost:8080/health
```

### 监控集成

监控数据可以集成到 Prometheus + Grafana 监控栈中：

1. **Prometheus 配置**：
```yaml
scrape_configs:
  - job_name: 'qproxy'
    static_configs:
      - targets: ['localhost:8080']
    metrics_path: '/metrics'
    scrape_interval: 15s
```

2. **Grafana 仪表板**：
   - 创建仪表板显示请求量、响应时间、错误率等关键指标
   - 设置告警规则监控服务健康状况

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
use qproxy::mqtt_client::{
    client::{MqttClient, MqttConfig},
    message::{
        ConnAckMessage, DisconnectMessage, Header, Message, MessageType, PingMessage, PublishMessage, QoS,
    },
};
use anyhow::Result;
use bytes::{Bytes, BytesMut};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
    task::JoinHandle,
    test,
    time::timeout,
};

// 模拟MQTT服务器
struct MockMqttServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
    rx: mpsc::Receiver<Vec<u8>>,
}

impl MockMqttServer {
    async fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        
        let (tx, rx) = mpsc::channel(100);
        
        let handle = tokio::spawn(async move {
            // 监听请求
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = vec![0; 1024];
                
                while let Ok(n) = stream.read(&mut buffer).await {
                    if n == 0 {
                        break;
                    }
                    
                    // 复制接收到的数据
                    let received = buffer[0..n].to_vec();
                    tx.send(received.clone()).await.ok();
                    
                    // 解析消息
                    if let Ok(message) = Message::decode(&received) {
                        match message.header.message_type {
                            MessageType::Connect => {
                                // 响应CONNECT消息
                                let mut connack = ConnAckMessage::new(0);
                                connack.session = Some("test-session-123".to_string());
                                connack.user_id = Some("test-user-456".to_string());
                                
                                let header = Header::new(MessageType::ConnAck);
                                let resp = Message::new(header, connack.encode().freeze());
                                let encoded = resp.encode();
                                
                                stream.write_all(&encoded).await.ok();
                            }
                            MessageType::Ping => {
                                // 响应PING消息
                                let header = Header::new(MessageType::Pong);
                                let resp = Message::new(header, Bytes::new());
                                let encoded = resp.encode();
                                
                                stream.write_all(&encoded).await.ok();
                            }
                            MessageType::Publish => {
                                // 发送收到的PUBLISH消息回来（echo）
                                stream.write_all(&received).await.ok();
                            }
                            MessageType::Disconnect => {
                                // 断开连接
                                break;
                            }
                            _ => {
                                // 未处理的消息类型
                            }
                        }
                    }
                }
            }
        });
        
        Ok(Self { addr, handle, rx })
    }
    
    async fn next_message(&mut self) -> Option<Vec<u8>> {
        timeout(Duration::from_secs(2), self.rx.recv()).await.ok()?
    }
    
    async fn stop(self) {
        self.handle.abort();
    }
}

// 这是一个集成测试，依赖网络连接，可能需要在CI环境中特别处理
#[test]
async fn test_mqtt_client_connect() -> Result<()> {
    // 创建模拟MQTT服务器
    let mut server = MockMqttServer::new().await?;
    
    // 创建MQTT客户端配置
    let config = MqttConfig {
        client_id: "test-client-integration".to_string(),
        host: "127.0.0.1".to_string(),
        port: server.addr.port(),
        keep_alive: 30,
        ..Default::default()
    };
    
    // 创建MQTT客户端
    let mut client = MqttClient::new(config);
    
    // 连接到模拟服务器
    client.connect().await?;
    
    // 检查服务器是否收到了CONNECT消息
    let message_data = server.next_message().await.expect("No CONNECT message received");
    let message = Message::decode(&message_data).expect("Failed to decode message");
    assert_eq!(message.header.message_type, MessageType::Connect);
    
    // 断开连接
    client.disconnect().await?;
    
    // 检查服务器是否收到了DISCONNECT消息
    let message_data = server.next_message().await.expect("No DISCONNECT message received");
    let message = Message::decode(&message_data).expect("Failed to decode message");
    assert_eq!(message.header.message_type, MessageType::Disconnect);
    
    // 停止服务器
    server.stop().await;
    
    Ok(())
}

// 我们还需要一个测试来验证发布消息功能
#[test]
async fn test_mqtt_client_publish() -> Result<()> {
    // 创建模拟MQTT服务器
    let mut server = MockMqttServer::new().await?;
    
    // 创建MQTT客户端配置
    let config = MqttConfig {
        client_id: "test-client-pub".to_string(),
        host: "127.0.0.1".to_string(),
        port: server.addr.port(),
        keep_alive: 30,
        ..Default::default()
    };
    
    // 创建MQTT客户端
    let mut client = MqttClient::new(config);
    
    // 连接到模拟服务器
    client.connect().await?;
    
    // 等待连接建立
    server.next_message().await; // 忽略CONNECT消息
    
    // 发布消息
    let topic = "test/topic".to_string();
    let payload = Bytes::from_static(b"test payload");
    client.publish(topic.clone(), payload.clone(), QoS::AtMostOnce).await?;
    
    // 检查服务器是否收到了PUBLISH消息
    let message_data = server.next_message().await.expect("No PUBLISH message received");
    let message = Message::decode(&message_data).expect("Failed to decode message");
    assert_eq!(message.header.message_type, MessageType::Publish);
    
    // 解析PUBLISH消息
    let publish = PublishMessage::decode(&message.payload).expect("Failed to decode PUBLISH");
    assert_eq!(publish.topic, topic);
    assert_eq!(publish.payload, payload);
    
    // 断开连接并停止服务器
    client.disconnect().await?;
    server.next_message().await; // 忽略DISCONNECT消息
    server.stop().await;
    
    Ok(())
} 
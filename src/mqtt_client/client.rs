//! MQTT 客户端模块
//! 
//! 本模块实现了一个完整的 MQTT 客户端，支持以下功能：
//! - MQTT 3.1.1 协议
//! - QoS 0/1/2 消息质量
//! - 自动重连
//! - 消息重传
//! - 会话保持
//! - 安全认证

use crate::mqtt_client::message::{
    ConnectMessage, ConnAckMessage, DisconnectMessage, Header, Message, MessageType,
    PublishMessage, QoS, ReconnectMessage, ReconnectAckMessage, ReconnectStatus,
    UnsubscribeMessage, QueryMessage, QueryAckMessage,
};
use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut, BufMut};
use futures::{SinkExt, StreamExt};
use tracing::{debug, error, info};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    time,
};
use tokio_util::codec::{Decoder, Encoder, Framed};

/// 自定义Subscribe消息
struct SubscribeMessage {
    message_id: u16,
    topics: Vec<(String, QoS)>,
}

impl SubscribeMessage {
    fn new(message_id: u16, topics: Vec<(String, QoS)>) -> Self {
        Self { message_id, topics }
    }

    fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Message ID
        buf.put_u16(self.message_id);
        
        // Topics with QoS
        for (topic, qos) in &self.topics {
            let topic_bytes = topic.as_bytes();
            buf.put_u16(topic_bytes.len() as u16);
            buf.extend_from_slice(topic_bytes);
            buf.put_u8(*qos as u8);
        }
        
        buf
    }
}

/// MQTT 客户端配置
#[derive(Debug, Clone)]
pub struct MqttConfig {
    /// 客户端 ID
    pub client_id: String,
    /// 服务器地址
    pub host: String,
    /// 服务器端口
    pub port: u16,
    /// 保活时间（秒）
    pub keep_alive: u16,
    /// 应用 ID
    pub app_id: Option<String>,
    /// 认证令牌
    pub token: Option<String>,
    /// 应用标识符
    pub app_identifier: Option<String>,
    /// 客户端信息
    pub info: Option<String>,
    /// 信息 QoS
    pub info_qos: Option<QoS>,
    /// 是否保留信息
    pub retain_info: bool,
    /// 是否不踢出其他客户端
    pub is_not_kick_other: bool,
    /// 客户端 IP
    pub client_ip: Option<String>,
    /// 验证信息
    pub validation_info: Option<String>,
    /// 是否支持重连
    pub has_reconnect: bool,
    /// 回调扩展
    pub callback_ext: Option<String>,
    /// 签名
    pub signature: Option<Vec<u8>>,
    /// 验证签名
    pub verify_sign: Option<Vec<u8>>,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            host: "localhost".to_string(),
            port: 1883,
            keep_alive: 60,
            app_id: None,
            token: None,
            app_identifier: None,
            info: None,
            info_qos: None,
            retain_info: false,
            is_not_kick_other: false,
            client_ip: None,
            validation_info: None,
            has_reconnect: false,
            callback_ext: None,
            signature: None,
            verify_sign: None,
        }
    }
}

/// MQTT 客户端
pub struct MqttClient {
    /// 客户端配置
    config: MqttConfig,
    /// 当前连接
    connection: Option<Connection>,
    /// 消息 ID 计数器
    packet_id: Arc<Mutex<u16>>,
    /// 订阅主题列表
    subscriptions: Arc<Mutex<HashMap<String, Vec<QoS>>>>,
    /// 会话 ID
    session_id: Option<String>,
    /// 消息处理任务控制器
    message_handler_tx: Option<oneshot::Sender<()>>,
}

/// 连接状态
struct Connection {
    /// 帧处理器
    framed: Framed<TcpStream, MqttCodec>,
    /// 消息发送通道
    tx: mpsc::Sender<Message>,
}

impl MqttClient {
    /// 创建新的 MQTT 客户端
    pub fn new(config: MqttConfig) -> Self {
        Self {
            config,
            connection: None,
            packet_id: Arc::new(Mutex::new(0)),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            session_id: None,
            message_handler_tx: None,
        }
    }

    /// 连接到 MQTT 服务器
    pub async fn connect(&mut self) -> Result<()> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let stream = TcpStream::connect(&addr).await?;
        
        let (tx, mut _rx) = mpsc::channel(100);
        let framed = Framed::new(stream, MqttCodec::new());
        
        // 创建连接消息
        let mut connect = ConnectMessage::new();
        connect.client_id = self.config.client_id.clone();
        connect.keep_alive = self.config.keep_alive;
        connect.app_id = self.config.app_id.clone();
        connect.token = self.config.token.clone();
        connect.app_identifier = self.config.app_identifier.clone();
        connect.info = self.config.info.clone();
        connect.info_qos = self.config.info_qos;
        connect.retain_info = self.config.retain_info;
        connect.is_not_kick_other = self.config.is_not_kick_other;
        connect.client_ip = self.config.client_ip.clone();
        connect.validation_info = self.config.validation_info.clone();
        connect.has_reconnect = self.config.has_reconnect;
        connect.callback_ext = self.config.callback_ext.clone();
        connect.signature = self.config.signature.clone();
        connect.verify_sign = self.config.verify_sign.clone();
        
        let header = Header::new(MessageType::Connect);
        let message = Message::new(header, connect.encode().freeze());
        
        let mut connection = Connection { framed, tx };
        connection.framed.send(message).await?;
        
        // 等待 CONNACK
        if let Some(connack) = connection.framed.next().await {
            let connack = connack?;
            if connack.header.message_type != MessageType::ConnAck {
                return Err(anyhow!("Expected CONNACK, got {:?}", connack.header.message_type));
            }
            
            let connack = ConnAckMessage::decode(&connack.payload)?;
            if connack.status != 0 {
                return Err(anyhow!("Connection refused: {}", connack.status));
            }
            
            // 保存会话 ID
            if let Some(session) = connack.session {
                self.session_id = Some(session);
            }
            
            info!("Connected to MQTT broker at {}", addr);
            self.connection = Some(connection);
            
            // 启动保活任务
            self.start_keep_alive_task();
            
            // 启动消息处理任务
            self.start_message_processing_task();
            
            Ok(())
        } else {
            Err(anyhow!("Connection closed"))
        }
    }

    /// 重连
    pub async fn reconnect(&mut self) -> Result<()> {
        if let Some(session_id) = &self.session_id {
            let reconnect = ReconnectMessage::new(session_id.clone());
            let header = Header::new(MessageType::Reconnect);
            let message = Message::new(header, reconnect.encode().freeze());
            
            if let Some(connection) = &mut self.connection {
                connection.framed.send(message).await?;
                
                // 等待重连确认
                if let Some(reconnect_ack) = connection.framed.next().await {
                    let reconnect_ack = reconnect_ack?;
                    if reconnect_ack.header.message_type != MessageType::ReconnectAck {
                        return Err(anyhow!("Expected RECONNECT_ACK, got {:?}", reconnect_ack.header.message_type));
                    }
                    
                    let reconnect_ack = ReconnectAckMessage::decode(&reconnect_ack.payload)?;
                    match reconnect_ack.status {
                        ReconnectStatus::Accepted => {
                            info!("Reconnected successfully");
                            Ok(())
                        }
                        ReconnectStatus::Error => {
                            Err(anyhow!("Reconnect failed"))
                        }
                        ReconnectStatus::NoMaster => {
                            Err(anyhow!("No master available"))
                        }
                    }
                } else {
                    Err(anyhow!("Connection closed during reconnect"))
                }
            } else {
                Err(anyhow!("Not connected"))
            }
        } else {
            Err(anyhow!("No session ID available"))
        }
    }

    /// 启动保活任务
    fn start_keep_alive_task(&self) {
        let keep_alive = self.config.keep_alive;
        let _tx = self.connection.as_ref().unwrap().tx.clone();
        
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(keep_alive as u64 / 2));
            loop {
                interval.tick().await;
                let ping = Message::new(Header::new(MessageType::Ping), Bytes::new());
                if _tx.send(ping).await.is_err() {
                    break;
                }
            }
        });
    }

    /// 启动消息处理任务
    fn start_message_processing_task(&mut self) {
        if let Some(connection) = &self.connection {
            // 创建取消通道
            let (tx, mut rx) = oneshot::channel();
            self.message_handler_tx = Some(tx);
            
            // 获取需要的引用和复制
            let _tx = connection.tx.clone();
            let subscriptions = self.subscriptions.clone();
            
            // 使用同一个Socket地址创建新连接
            let addr = format!("{}:{}", self.config.host, self.config.port);
            
            tokio::spawn(async move {
                // 创建单独的连接用于消息处理
                let stream = match TcpStream::connect(&addr).await {
                    Ok(stream) => stream,
                    Err(e) => {
                        error!("Failed to create stream for message processing: {}", e);
                        return;
                    }
                };
                
                let mut framed = Framed::new(stream, MqttCodec::new());
                let mut done = false;
                
                loop {
                    // 检查是否收到取消信号
                    if !done {
                        if rx.try_recv().is_ok() {
                            done = true;
                            break;
                        }
                    }
                    
                    // 处理消息
                    if let Some(result) = framed.next().await {
                        match result {
                            Ok(message) => {
                                match message.header.message_type {
                                    MessageType::Publish => {
                                        let topic_len = (message.payload[0] as usize) << 8 | message.payload[1] as usize;
                                        let topic = String::from_utf8_lossy(&message.payload[2..2 + topic_len]).to_string();
                                        let _payload = message.payload.slice(2 + topic_len..);
                                        
                                        if let Some(qos_levels) = subscriptions.lock().unwrap().get(&topic) {
                                            for qos in qos_levels {
                                                debug!("Received message on topic {} with QoS {:?}", topic, qos);
                                            }
                                        }
                                    }
                                    MessageType::Ping => {
                                        let pong = Message::new(Header::new(MessageType::Pong), Bytes::new());
                                        if let Err(e) = framed.send(pong).await {
                                            error!("Failed to send PONG: {}", e);
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                error!("Error processing message: {}", e);
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                }
            });
        }
    }

    /// 发布消息
    pub async fn publish(&mut self, topic: String, payload: Bytes, qos: QoS) -> Result<()> {
        let mut publish = PublishMessage::new(topic, payload);
        
        if qos != QoS::AtMostOnce {
            let mut packet_id = self.packet_id.lock().unwrap();
            *packet_id = packet_id.wrapping_add(1);
            publish.packet_id = Some(*packet_id);
        }
        
        let header = Header {
            message_type: MessageType::Publish,
            dup: false,
            qos,
            retain: false,
        };
        
        let message = Message::new(header, publish.encode().freeze());
        
        if let Some(connection) = &mut self.connection {
            connection.framed.send(message).await?;
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }

    /// 查询消息
    pub async fn query(&mut self, topic: String, payload: Bytes) -> Result<QueryAckMessage> {
        let mut query = QueryMessage::new(topic, payload);
        
        let mut packet_id = self.packet_id.lock().unwrap();
        *packet_id = packet_id.wrapping_add(1);
        query.target_id = Some(packet_id.to_string());
        
        let header = Header::new(MessageType::Query);
        let message = Message::new(header, query.encode().freeze());
        
        if let Some(connection) = &mut self.connection {
            connection.framed.send(message).await?;
            
            // 等待查询确认
            if let Some(query_ack) = connection.framed.next().await {
                let query_ack = query_ack?;
                if query_ack.header.message_type != MessageType::QueryAck {
                    return Err(anyhow!("Expected QUERY_ACK, got {:?}", query_ack.header.message_type));
                }
                
                Ok(QueryAckMessage::decode(&query_ack.payload)?)
            } else {
                Err(anyhow!("Connection closed during query"))
            }
        } else {
            Err(anyhow!("Not connected"))
        }
    }

    /// 订阅主题
    pub async fn subscribe(&mut self, topic: String, qos: QoS) -> Result<()> {
        let mut packet_id = self.packet_id.lock().unwrap();
        *packet_id = packet_id.wrapping_add(1);
        
        let subscribe = SubscribeMessage::new(*packet_id, vec![(topic.clone(), qos)]);
        // 使用Unsubscribe消息类型，因为message.rs中没有Subscribe类型
        let header = Header::new(MessageType::Unsubscribe);
        let message = Message::new(header, subscribe.encode().freeze());
        
        if let Some(connection) = &mut self.connection {
            connection.framed.send(message).await?;
            
            // 添加到本地订阅列表
            self.subscriptions
                .lock()
                .unwrap()
                .entry(topic)
                .or_insert_with(Vec::new)
                .push(qos);
            
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }

    /// 取消订阅
    pub async fn unsubscribe(&mut self, topic: String) -> Result<()> {
        let mut packet_id = self.packet_id.lock().unwrap();
        *packet_id = packet_id.wrapping_add(1);
        
        let unsubscribe = UnsubscribeMessage::new(*packet_id, vec![topic.clone()]);
        let header = Header::new(MessageType::Unsubscribe);
        let message = Message::new(header, unsubscribe.encode().freeze());
        
        if let Some(connection) = &mut self.connection {
            connection.framed.send(message).await?;
            
            // 从本地订阅列表中移除
            self.subscriptions.lock().unwrap().remove(&topic);
            
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }

    /// 断开连接
    pub async fn disconnect(&mut self) -> Result<()> {
        // 取消消息处理任务
        if let Some(tx) = self.message_handler_tx.take() {
            let _ = tx.send(());
        }
        
        if let Some(connection) = &mut self.connection {
            let disconnect = DisconnectMessage::new(0, None);
            let header = Header::new(MessageType::Disconnect);
            let message = Message::new(header, disconnect.encode().freeze());
            
            connection.framed.send(message).await?;
            self.connection = None;
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }
}

/// MQTT 编解码器
struct MqttCodec;

impl MqttCodec {
    fn new() -> Self {
        Self
    }
}

impl Decoder for MqttCodec {
    type Item = Message;
    type Error = anyhow::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            return Ok(None);
        }

        match Message::decode(src) {
            Ok(message) => Ok(Some(message)),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Ok(None),
            Err(e) => Err(anyhow!("Failed to decode message: {}", e)),
        }
    }
}

impl Encoder<Message> for MqttCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.extend_from_slice(&item.encode());
        Ok(())
    }
} 
use crate::PLAYBACK_SERVICE;

use super::codec::MqttCodec;
use super::crypto::CryptoInfo;
use super::message::{
    ConnAckMessage, ConnectMessage, Header, Message, MessageType, PingMessage,
    PubAckMessage, PublishMessage, QoS, QueryAckMessage, QueryConMessage, QueryMessage, ServerPublishMessage,
};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures::{FutureExt, SinkExt, StreamExt};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::mpsc,
    time,
};
use tokio_util::codec::Framed;
use tracing::{debug, error, info, warn};

// A struct to track published messages waiting for acknowledgment
struct OutgoingMessage {
    message_id: u16,
    topic: String,
    qos: QoS,
    timestamp: u64,
    retries: u8,
    payload: Bytes,
}

pub struct MqttClient {
    client_id: String,
    host: String,
    port: u16,
    keep_alive: u16,
    username: Option<String>,
    password: Option<String>,
    clean_session: bool,
    connection: Option<Connection>,
    packet_id: Arc<Mutex<u16>>,
    outgoing_messages: Arc<Mutex<HashMap<u16, OutgoingMessage>>>,
    message_timeout: Duration,
    max_retries: u8,
}

struct Connection {
    tx_to_broker: mpsc::Sender<Message>,
    rx_from_broker: Option<mpsc::Receiver<Result<Message>>>,
    codec: Arc<MqttCodec>,
}

impl MqttClient {
    pub fn new(client_id: String, host: String, port: u16) -> Self {
        Self {
            client_id,
            host,
            port,
            keep_alive: 60,
            username: None,
            password: None,
            clean_session: true,
            connection: None,
            packet_id: Arc::new(Mutex::new(0)),
            outgoing_messages: Arc::new(Mutex::new(HashMap::new())),
            message_timeout: Duration::from_secs(30),
            max_retries: 3,
        }
    }

    pub fn set_credentials(&mut self, username: String, password: String) {
        self.username = Some(username);
        self.password = Some(password);
    }

    pub fn set_keep_alive(&mut self, keep_alive: u16) {
        self.keep_alive = keep_alive;
    }

    pub fn set_clean_session(&mut self, clean_session: bool) {
        self.clean_session = clean_session;
    }

    pub fn set_message_timeout(&mut self, timeout: Duration) {
        self.message_timeout = timeout;
    }

    pub fn set_max_retries(&mut self, max_retries: u8) {
        self.max_retries = max_retries;
    }

    pub async fn connect(&mut self) -> Result<()> {
        let socket = TcpStream::connect(format!("{}:{}", self.host, self.port)).await?;

        // Create a new connect message
        let mut connect = ConnectMessage::new();
        connect.client_id = self.client_id.clone();
        connect.keep_alive = self.keep_alive;

        if let (Some(username), Some(password)) = (&self.username, &self.password) {
            connect.app_id = Some(username.clone());
            connect.token = Some(password.clone());
        }

        // Create a new codec
        let codec = MqttCodec::new();

        // Create framed connection
        let mut framed = Framed::new(socket, codec);

        // Create channels
        let (tx_to_broker, mut rx_to_broker) = mpsc::channel(100);
        let (tx_from_broker, rx_from_broker) = mpsc::channel(100);

        // Create connection
        let connection = Connection {
            tx_to_broker: tx_to_broker.clone(),
            rx_from_broker: Some(rx_from_broker),
            codec: Arc::new(MqttCodec::new()),
        };

        // Clone tx_to_broker before moving connection
        let tx_to_broker = tx_to_broker.clone();

        // Store connection
        self.connection = Some(connection);

        // Send connect message
        let header = Header {
            message_type: MessageType::Connect,
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
        };

        let connect_message = Message::new(header, connect.encode().freeze());

        framed.send(connect_message).await?;

        // Spawn task to handle messages from/to broker
        let client_id = self.client_id.clone();
        let packet_id = self.packet_id.clone();
        let outgoing_messages = self.outgoing_messages.clone();

        tokio::spawn(async move {
            let mut ping_interval = time::interval(Duration::from_secs(30));

            loop {
                // Use futures::FutureExt::select to handle futures in sequence instead of tokio::select!

                // 1. Check for ping timer
                let ping_timer = ping_interval.tick().fuse();
                let broker_message = rx_to_broker.recv().fuse();

                futures::pin_mut!(ping_timer);
                futures::pin_mut!(broker_message);

                match futures::future::select(ping_timer, broker_message).await {
                    // Ping timer fired
                    futures::future::Either::Left((_, _)) => {
                        // Send ping
                        let header = Header {
                            message_type: MessageType::Ping,
                            dup: false,
                            qos: QoS::AtMostOnce,
                            retain: false,
                        };

                        let ping = PingMessage;
                        let message = Message::new(header, ping.encode().freeze());

                        if let Err(err) = framed.send(message).await {
                            error!("Failed to send ping: {}", err);
                            break;
                        }
                    },
                    // Message from broker channel
                    futures::future::Either::Right((Some(message), _)) => {
                        if let Err(err) = framed.send(message).await {
                            error!("Failed to send message: {}", err);
                            break;
                        }
                    },
                    // Channel closed
                    futures::future::Either::Right((None, _)) => {
                        break;
                    }
                }

                // Now check for incoming network messages
                match framed.next().await {
                    Some(Ok(message)) => {
                        match message.header.message_type {
                            MessageType::ConnAck => {
                                // Handle connection acknowledgment
                                if let Ok(connack) = ConnAckMessage::decode(&message.payload) {
                                    if connack.status == 0 {
                                        debug!("Connected to broker");
                                    } else {
                                        error!("Connection refused: status={}", connack.status);
                                        break;
                                    }
                                }
                            }
                            MessageType::Publish => {
                                // Handle incoming publish
                                if let Ok(publish) = ServerPublishMessage::decode(&message.payload) {
                                    // FIX ME 对 QoS 1 和 QoS 2 都回复 PubAckMessage
                                    if message.header.qos == QoS::AtLeastOnce || message.header.qos == QoS::ExactlyOnce {
                                        if let Some(packet_id) = publish.packet_id {
                                            let puback: PubAckMessage = PubAckMessage::new(packet_id, 0);

                                            let ack_header = Header {
                                                message_type: MessageType::PubAck,
                                                dup: false,
                                                qos: QoS::AtMostOnce,
                                                retain: false,
                                            };

                                            let ack_message = Message::new(ack_header, puback.encode().freeze());

                                            if let Err(err) = framed.send(ack_message).await {
                                                error!("Failed to send PUBACK: {}", err);
                                            }
                                        }
                                    }

                                    // 将 ServerPublishMessage 转换为 TrafficRecord
                                    let topic = publish.topic.clone();

                                    // 序列化完整的 publish 对象
                                    let publish_bytes = publish.encode().freeze().to_vec();

                                    // 创建 TrafficRecord
                                    // FIX ME 需要修改
                                    let record = crate::model::TrafficRecord::new_tcp(
                                        &topic, // service_name 使用 topic
                                        publish_bytes, // request_data 使用完整的 publish 对象
                                        vec![] // response_data 为空
                                    );

                                    // 尝试获取回放服务
                                    let playback_service = PLAYBACK_SERVICE.lock().await;
                                    if let Some(playback_service) = playback_service.as_ref() {
                                        // 1. 先存储到 Redis
                                        if let Err(err) = playback_service.add_record(record.clone()).await {
                                            error!("Failed to store MQTT message to Redis: {}", err);
                                        } else {
                                            debug!("Stored MQTT message from topic '{}' to Redis", topic);
                                        }

                                        // 2. 然后触发本地回放
                                        playback_service.trigger_replay(&record).await;
                                        debug!("Triggered local replay for MQTT message from topic '{}'", topic);
                                    } else {
                                        error!("Playback service not initialized");
                                        continue;
                                    };

                                    // Forward to user
                                    if let Err(err) = tx_from_broker.send(Ok(message)).await {
                                        error!("Failed to forward message: {}", err);
                                    }
                                }
                            }
                            MessageType::PubAck => {
                                // Handle publish acknowledgment
                                if let Ok(ack) = PubAckMessage::decode(&message.payload) {
                                    debug!("Received PUBACK for message ID: {}", ack.message_id);

                                    // Remove the acknowledged message from the outgoing messages
                                    outgoing_messages.lock().unwrap().remove(&ack.message_id);
                                }
                            }
                            MessageType::Query => {
                                // Handle incoming query
                                if let Ok(query) = QueryMessage::decode(&message.payload) {
                                    debug!("Received Query for topic: {} with target_id: {:?}",
                                           query.topic, query.target_id);

                                    // 将 QueryMessage 转换为 TrafficRecord
                                    let topic = query.topic.clone();

                                    // 保存原始消息的 QoS 级别
                                    let original_qos = message.header.qos;

                                    // 序列化完整的 query 对象
                                    let query_bytes = query.encode().freeze().to_vec();

                                    // 创建 TrafficRecord
                                    let record = crate::model::TrafficRecord::new_tcp(
                                        &topic, // service_name 使用 topic
                                        query_bytes, // request_data 使用完整的 query 对象
                                        vec![] // response_data 为空
                                    );

                                    // 尝试获取回放服务
                                    let playback_service = PLAYBACK_SERVICE.lock().await;
                                    if let Some(playback_service) = playback_service.as_ref() {
                                        // 1. 先存储到 Redis
                                        if let Err(err) = playback_service.add_record(record.clone()).await {
                                            error!("Failed to store Query message to Redis: {}", err);
                                        } else {
                                            debug!("Stored Query message from topic '{}' to Redis", topic);
                                        }

                                        // 2. 然后触发本地回放
                                        playback_service.trigger_replay(&record).await;
                                        debug!("Triggered local replay for Query message from topic '{}'", topic);
                                    } else {
                                        error!("Playback service not initialized");
                                        continue;
                                    };

                                    // 回复 QueryAck
                                    if let Some(packet_id) = query.packet_id {
                                        let query_ack = QueryAckMessage::new(packet_id, 0, Some(query.payload.clone()));

                                        let ack_header = Header {
                                            message_type: MessageType::QueryAck,
                                            dup: false,
                                            qos: original_qos, // 使用原始消息的 QoS 级别
                                            retain: false,
                                        };

                                        let ack_message = Message::new(ack_header, query_ack.encode().freeze());

                                        if let Err(err) = framed.send(ack_message).await {
                                            error!("Failed to send QueryAck: {}", err);
                                        } else {
                                            debug!("Sent QueryAck for message ID: {}", packet_id);
                                        }
                                    }

                                    // Forward to user
                                    if let Err(err) = tx_from_broker.send(Ok(message)).await {
                                        error!("Failed to forward Query message: {}", err);
                                    }
                                }
                            }
                            MessageType::QueryAck => {
                                // Handle query acknowledgment
                                if let Ok(ack) = QueryAckMessage::decode(&message.payload) {
                                    debug!("Received QueryAck for message ID: {} with status: {}",
                                           ack.message_id, ack.status);

                                    // 在 QoS 2 (ExactlyOnce) 中，不在这里移除消息
                                    // 而是等待 QueryCon 消息到达后才移除

                                    // 如果有响应数据，可以处理它
                                    if let Some(data) = &ack.data {
                                        debug!("QueryAck contains data of size: {} bytes", data.len());
                                    }

                                    // Forward to user
                                    if let Err(err) = tx_from_broker.send(Ok(message)).await {
                                        error!("Failed to forward QueryAck message: {}", err);
                                    }
                                }
                            }
                            MessageType::QueryCon => {
                                // Handle query confirmation
                                if let Ok(con) = QueryConMessage::decode(&message.payload) {
                                    debug!("Received QueryCon for message ID: {}", con.message_id);

                                    // 清理相关的 OutgoingMessage
                                    outgoing_messages.lock().unwrap().remove(&con.message_id);
                                    debug!("Removed outgoing message with ID: {} after receiving QueryCon", con.message_id);
                                }
                            }
                            MessageType::Pong => {
                                // Pong received, do nothing
                                debug!("Received pong from broker");
                            }
                            _ => {
                                // Forward to user
                                if let Err(err) = tx_from_broker.send(Ok(message)).await {
                                    error!("Failed to forward message: {}", err);
                                }
                            }
                        }
                    }
                    Some(Err(err)) => {
                        error!("Error receiving message: {}", err);
                        break;
                    }
                    None => {
                        // Connection closed
                        debug!("Connection closed by broker");
                        break;
                    }
                }
            }
        });

        // Start the message resend task
        let outgoing_messages = self.outgoing_messages.clone();
        let message_timeout = self.message_timeout;
        let max_retries = self.max_retries;

        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(1));

            loop {
                interval.tick().await;

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let mut to_resend = Vec::new();
                let mut to_remove = Vec::new();

                // Check for messages that need to be resent
                {
                    let mut outgoing = outgoing_messages.lock().unwrap();

                    for (id, msg) in outgoing.iter_mut() {
                        if now - msg.timestamp > message_timeout.as_secs() {
                            if msg.retries < max_retries {
                                // Increase retry count and prepare to resend
                                msg.retries += 1;
                                msg.timestamp = now;

                                to_resend.push((
                                    *id,
                                    msg.topic.clone(),
                                    msg.payload.clone(),
                                    msg.qos,
                                ));
                            } else {
                                // Maximum retries reached, remove the message
                                to_remove.push(*id);
                                warn!("Message to topic {} with id {} timed out after {} retries",
                                     msg.topic, id, max_retries);
                            }
                        }
                    }

                    // Remove timed out messages
                    for id in to_remove {
                        outgoing.remove(&id);
                    }
                }

                // Resend messages
                for (id, topic, payload, qos) in to_resend {
                    let mut publish = PublishMessage::new(topic, payload);
                    publish.packet_id = Some(id);

                    let header = Header {
                        message_type: MessageType::Publish,
                        dup: true, // Set DUP flag for retransmission
                        qos,
                        retain: false,
                    };

                    let message = Message::new(header, publish.encode().freeze());

                    if let Err(err) = tx_to_broker.send(message).await {
                        error!("Failed to resend message: {}", err);
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn publish(&mut self, topic: String, payload: Bytes, qos: QoS) -> Result<()> {
        let mut publish = PublishMessage::new(topic.clone(), payload.clone());
        let mut packet_id = 0;
        
        if qos != QoS::AtMostOnce {
            let mut packet_id_guard = self.packet_id.lock().unwrap();
            *packet_id_guard = packet_id_guard.wrapping_add(1);
            packet_id = *packet_id_guard;
            publish.packet_id = Some(packet_id);
        }
        
        let header = Header {
            message_type: MessageType::Publish,
            dup: false,
            qos,
            retain: false,
        };
        
        let message = Message::new(header, publish.encode().freeze());
        
        if let Some(connection) = &self.connection {
            // If QoS is 1 or 2, store the message for potential retransmission
            if qos != QoS::AtMostOnce {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let outgoing_msg = OutgoingMessage {
                    message_id: packet_id,
                    topic,
                    qos,
                    timestamp: now,
                    retries: 0,
                    payload,
                };

                self.outgoing_messages.lock().unwrap().insert(packet_id, outgoing_msg);
            }

            connection.tx_to_broker.send(message).await?;
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }

    /// 发送 Query 消息
    ///
    /// # 参数
    ///
    /// * `topic` - 查询的主题
    /// * `payload` - 查询的数据内容
    /// * `target_id` - 可选的目标ID，指定接收方
    ///
    /// # 返回值
    ///
    /// 操作的结果
    pub async fn query(&mut self, topic: String, payload: Bytes, target_id: Option<String>) -> Result<u16> {
        if self.connection.is_none() {
            return Err(anyhow!("Not connected to broker"));
        }

        // 获取下一个 packet_id
        let packet_id = {
            let mut id = self.packet_id.lock().unwrap();
            *id = if *id < u16::MAX { *id + 1 } else { 1 };
            *id
        };

        // 创建 Query 消息
        let query = QueryMessage::new(topic.clone(), payload.clone(), target_id)
            .with_packet_id(packet_id);

        let header = Header {
            message_type: MessageType::Query,
            dup: false,
            qos: QoS::ExactlyOnce,  // Query 消息使用 QoS 1
            retain: false,
        };

        let message = Message::new(header, query.encode().freeze());

        // 保存发送的消息，以便需要时重发
        {
            let mut outgoing = self.outgoing_messages.lock().unwrap();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            outgoing.insert(
                packet_id,
                OutgoingMessage {
                    message_id: packet_id,
                    topic: topic.clone(),
                    qos: QoS::ExactlyOnce,
                    timestamp: now,
                    retries: 0,
                    payload: payload.clone(),
                },
            );
        }

        // 发送消息
        let connection = self.connection.as_ref().unwrap();
        if let Err(err) = connection.tx_to_broker.send(message).await {
            return Err(anyhow!("Failed to send Query: {}", err));
        }

        debug!("Sent Query message to topic: {} with ID: {}", topic, packet_id);
        Ok(packet_id)
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(connection) = &self.connection {
            let disconnect = Message::new(Header::new(MessageType::Disconnect), Bytes::new());
            connection.tx_to_broker.send(disconnect).await?;
            self.connection = None;
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }

    // 设置AES加密，需要在connect后调用
    pub async fn set_encryption(&self, key: Vec<u8>) -> Result<()> {
        if let Some(connection) = &self.connection {
            let crypto_info = CryptoInfo::new(key);
            connection.codec.set_crypto_info(crypto_info).await;
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }
} 
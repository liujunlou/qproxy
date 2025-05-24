use crate::{
    PLAYBACK_SERVICE,
    options::{Options, ProxyMode},
    proxy::tcp::handle_tls_proxy,
};

use super::codec::{MqttCodec, MqttMessage, QoS, QueryMessage};
use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};
use futures::{FutureExt, SinkExt, StreamExt};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::mpsc,
    time,
};
use tokio_util::codec::Framed;
use tracing::{debug, error, info, warn};
use tokio_rustls::{TlsStream, TlsAcceptor};
use std::io::Cursor;

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
    options: Arc<Options>,
}

struct Connection {
    tx_to_broker: mpsc::Sender<MqttMessage>,
    rx_from_broker: Option<mpsc::Receiver<Result<MqttMessage>>>,
    codec: Arc<MqttCodec>,
}

// 创建一个简单的内存流来模拟 TLS 连接
struct MemoryStream {
    read_buf: Cursor<Vec<u8>>,
    write_buf: Vec<u8>,
}

impl AsyncRead for MemoryStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let n = std::io::Read::read(&mut this.read_buf, buf.initialize_unfilled())?;
        buf.advance(n);
        std::task::Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for MemoryStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        this.write_buf.extend_from_slice(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl MqttClient {
    pub fn new(client_id: String, host: String, port: u16, options: Arc<Options>) -> Self {
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
            options,
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
        let connect = MqttMessage::Connect(super::codec::ConnectMessage {
            device_id: self.client_id.clone(),
            clean_session: self.clean_session,
            keep_alive: self.keep_alive as u32,
            credentials: if let (Some(username), Some(password)) = (&self.username, &self.password) {
                Some(super::codec::Credentials {
                    app_key: username.clone(),
                    token: Some(password.clone()),
                })
            } else {
                None
            },
            will: None,
            protocol_version: 3,
            protocol_name: "RCloud".to_string(),
        });

        // Create a new codec
        let codec = MqttCodec::new(1024 * 1024); // 1MB max frame length

        // Create framed connection
        let mut framed = Framed::new(socket, codec);

        // Create channels
        let (tx_to_broker, mut rx_to_broker) = mpsc::channel(100);
        let (tx_from_broker, rx_from_broker) = mpsc::channel(100);

        // Create connection
        let connection = Connection {
            tx_to_broker: tx_to_broker.clone(),
            rx_from_broker: Some(rx_from_broker),
            codec: Arc::new(MqttCodec::new(1024 * 1024)),
        };

        // Store connection
        self.connection = Some(connection);

        // Send connect message
        framed.send(connect).await?;

        // Spawn task to handle messages from/to broker
        let outgoing_messages = self.outgoing_messages.clone();
        let packet_id = self.packet_id.clone();
        let options = self.options.clone();
        let client_id = self.client_id.clone();
        let mut framed = framed;
        tokio::spawn(async move {
            let mut ping_interval = time::interval(Duration::from_secs(30));

            loop {
                let ping_timer = ping_interval.tick().fuse();
                let broker_message = rx_to_broker.recv().fuse();

                futures::pin_mut!(ping_timer);
                futures::pin_mut!(broker_message);

                match futures::future::select(ping_timer, broker_message).await {
                    futures::future::Either::Left((_, _)) => {
                        // Send ping
                        let ping = MqttMessage::PingReq(super::codec::PingReqMessage {
                            properties: None,
                        });

                        if let Err(err) = framed.send(ping).await {
                            error!("Failed to send ping: {}", err);
                            break;
                        }
                    },
                    futures::future::Either::Right((Some(message), _)) => {
                        if let Err(err) = framed.send(message).await {
                            error!("Failed to send message: {}", err);
                            break;
                        }
                    },
                    futures::future::Either::Right((None, _)) => {
                        break;
                    }
                }

                match framed.next().await {
                    Some(Ok(message)) => {
                        match message {
                            MqttMessage::ConnAck(connack) => {
                                if connack.status == 0 {
                                    debug!("Connected to broker");
                                } else {
                                    error!("Connection refused: status={}", connack.status);
                                    break;
                                }
                            }
                            MqttMessage::Publish(ref publish) => {
                                // Handle incoming publish
                                if publish.qos != QoS::AtMostOnce {
                                    if let Some(packet_id) = publish.message_id {
                                        let puback = MqttMessage::PubAck(super::codec::PubAckMessage {
                                            message_id: packet_id,
                                            reason_code: Some(0),
                                            properties: None,
                                        });

                                        if let Err(err) = framed.send(puback).await {
                                            error!("Failed to send PUBACK: {}", err);
                                        }
                                    }
                                }

                                // Convert to TrafficRecord
                                let topic = publish.topic.clone();
                                let publish_bytes = serde_json::to_vec(&publish).unwrap_or_default();

                                let record = crate::model::TrafficRecord::new_tcp(
                                    &topic,
                                    publish_bytes,
                                    vec![]
                                );

                                match options.mode {
                                    ProxyMode::Record => {
                                        // 1. 先存储到 Redis
                                        let playback_service = PLAYBACK_SERVICE.lock().await;
                                        if let Some(playback_service) = playback_service.as_ref() {
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
                                        }
                                    }
                                    ProxyMode::Playback => {
                                        // 创建内存流
                                        let stream = MemoryStream {
                                            read_buf: Cursor::new(record.request.body.clone()),
                                            write_buf: Vec::new(),
                                        };

                                        // 创建 TLS 流
                                        if let Some(tls) = &options.tcp.tls {
                                            let tls_config = crate::load_tls(&tls.tls_cert, &tls.tls_key);
                                            let acceptor = TlsAcceptor::from(tls_config);
                                            
                                            // 直接调用 handle_tls_proxy
                                            let options = options.clone();
                                            tokio::spawn(async move {
                                                match acceptor.accept(stream).await {
                                                    Ok(tls_stream) => {
                                                        if let Err(e) = handle_tls_proxy(TlsStream::Server(tls_stream), options).await {
                                                            error!("Failed to handle TLS proxy for MQTT message: {:?}", e);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        error!("Failed to establish TLS connection: {:?}", e);
                                                    }
                                                }
                                            });
                                        } else {
                                            error!("TLS configuration not found");
                                        }
                                    }
                                }

                                // Forward to user
                                if let Err(err) = tx_from_broker.send(Ok(message.clone())).await {
                                    error!("Failed to forward message: {}", err);
                                }
                            }
                            MqttMessage::PubAck(ack) => {
                                debug!("Received PUBACK for message ID: {}", ack.message_id);
                                outgoing_messages.lock().unwrap().remove(&ack.message_id);
                            }
                            MqttMessage::Query(ref query) => {
                                let query_message = QueryMessage {
                                    query_type: query.query_type.clone(),
                                    payload: query.payload.clone(),
                                    device_id: query.device_id.clone(),
                                    message_id: query.message_id,
                                    qos: query.qos,
                                    retain: query.retain,
                                    dup: query.dup,
                                };
                                debug!("Received Query for topic: {} with target_id: {:?}",
                                       query_message.query_type, query_message.device_id);

                                // Convert to TrafficRecord
                                let topic = query_message.query_type.clone();
                                let query_bytes = serde_json::to_vec(&query_message).unwrap_or_default();

                                let record = crate::model::TrafficRecord::new_tcp(
                                    &topic,
                                    query_bytes,
                                    vec![]
                                );

                                match options.mode {
                                    ProxyMode::Record => {
                                        // 1. 先存储到 Redis
                                        let playback_service = PLAYBACK_SERVICE.lock().await;
                                        if let Some(playback_service) = playback_service.as_ref() {
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
                                        }
                                    }
                                    ProxyMode::Playback => {
                                        // 创建内存流
                                        let stream = MemoryStream {
                                            read_buf: Cursor::new(record.request.body.clone()),
                                            write_buf: Vec::new(),
                                        };

                                        // 创建 TLS 流
                                        if let Some(tls) = &options.tcp.tls {
                                            let tls_config = crate::load_tls(&tls.tls_cert, &tls.tls_key);
                                            let acceptor = TlsAcceptor::from(tls_config);
                                            
                                            // 直接调用 handle_tls_proxy
                                            let options = options.clone();
                                            tokio::spawn(async move {
                                                match acceptor.accept(stream).await {
                                                    Ok(tls_stream) => {
                                                        if let Err(e) = handle_tls_proxy(TlsStream::Server(tls_stream), options).await {
                                                            error!("Failed to handle TLS proxy for MQTT message: {:?}", e);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        error!("Failed to establish TLS connection: {:?}", e);
                                                    }
                                                }
                                            });
                                        } else {
                                            error!("TLS configuration not found");
                                        }
                                    }
                                }

                                // Reply with QueryAck
                                if let Some(message_id) = query.message_id {
                                    let query_ack = MqttMessage::QueryAck(super::codec::QueryAckMessage {
                                        query_type: query.query_type.clone(),
                                        status: 0,
                                        message_id: Some(message_id),
                                        reason_code: Some(0),
                                        properties: None,
                                    });

                                    if let Err(err) = framed.send(query_ack).await {
                                        error!("Failed to send QueryAck: {}", err);
                                    } else {
                                        debug!("Sent QueryAck for message ID: {}", message_id);
                                    }
                                }

                                // Forward to user
                                if let Err(err) = tx_from_broker.send(Ok(message.clone())).await {
                                    error!("Failed to forward Query message: {}", err);
                                }
                            }
                            MqttMessage::QueryAck(ref ack) => {
                                debug!("Received QueryAck for message ID: {:?} with status: {}",
                                       ack.message_id, ack.status);

                                // Forward to user
                                if let Err(err) = tx_from_broker.send(Ok(message.clone())).await {
                                    error!("Failed to forward QueryAck message: {}", err);
                                }
                            }
                            MqttMessage::QueryConn(con) => {
                                debug!("Received QueryCon for message ID: {:?}", con.message_id);
                                if let Some(message_id) = con.message_id {
                                    outgoing_messages.lock().unwrap().remove(&message_id);
                                    debug!("Removed outgoing message with ID: {} after receiving QueryCon", message_id);
                                }
                            }
                            MqttMessage::PingResp(_) => {
                                debug!("Received pong from broker");
                            }
                            _ => {
                                // Forward to user
                                if let Err(err) = tx_from_broker.send(Ok(message.clone())).await {
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

                {
                    let mut outgoing = outgoing_messages.lock().unwrap();

                    for (id, msg) in outgoing.iter_mut() {
                        if now - msg.timestamp > message_timeout.as_secs() {
                            if msg.retries < max_retries {
                                msg.retries += 1;
                                msg.timestamp = now;

                                to_resend.push((
                                    *id,
                                    msg.topic.clone(),
                                    msg.payload.clone(),
                                    msg.qos,
                                ));
                            } else {
                                to_remove.push(*id);
                                warn!("Message to topic {} with id {} timed out after {} retries",
                                     msg.topic, id, max_retries);
                            }
                        }
                    }

                    for id in to_remove {
                        outgoing.remove(&id);
                    }
                }

                for (id, topic, payload, qos) in to_resend {
                    let publish = MqttMessage::Publish(super::codec::PublishMessage {
                        topic,
                        payload: String::from_utf8_lossy(&payload).to_string(),
                        message_id: Some(id),
                        qos,
                        retain: false,
                        dup: true,
                        properties: None,
                    });

                    if let Err(err) = tx_to_broker.send(publish).await {
                        error!("Failed to resend message: {}", err);
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn publish(&mut self, topic: String, payload: Bytes, qos: QoS) -> Result<()> {
        let mut packet_id = 0;
        
        if qos != QoS::AtMostOnce {
            let mut packet_id_guard = self.packet_id.lock().unwrap();
            *packet_id_guard = packet_id_guard.wrapping_add(1);
            packet_id = *packet_id_guard;
        }
        
        let publish = MqttMessage::Publish(super::codec::PublishMessage {
            topic: topic.clone(),
            payload: String::from_utf8_lossy(&payload).to_string(),
            message_id: if qos != QoS::AtMostOnce { Some(packet_id) } else { None },
            qos,
            retain: false,
            dup: false,
            properties: None,
        });
        
        if let Some(connection) = &self.connection {
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

            connection.tx_to_broker.send(publish).await?;
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }

    pub async fn query(&mut self, topic: String, payload: Bytes, target_id: Option<String>) -> Result<u16> {
        if self.connection.is_none() {
            return Err(anyhow!("Not connected to broker"));
        }

        let packet_id = {
            let mut id = self.packet_id.lock().unwrap();
            *id = if *id < u16::MAX { *id + 1 } else { 1 };
            *id
        };

        let query = MqttMessage::Query(super::codec::QueryMessage {
            query_type: topic.clone(),
            payload: String::from_utf8_lossy(&payload).to_string(),
            device_id: target_id.unwrap_or_default(),
            message_id: Some(packet_id),
            qos: QoS::ExactlyOnce,
            retain: false,
            dup: false,
        });

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

        let connection = self.connection.as_ref().unwrap();
        if let Err(err) = connection.tx_to_broker.send(query).await {
            return Err(anyhow!("Failed to send Query: {}", err));
        }

        debug!("Sent Query message to topic: {} with ID: {}", topic, packet_id);
        Ok(packet_id)
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(connection) = &self.connection {
            let disconnect = MqttMessage::Disconnect(super::codec::DisconnectMessage {
                reason_code: Some(0),
                properties: None,
            });
            connection.tx_to_broker.send(disconnect).await?;
            self.connection = None;
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }

    pub async fn set_encryption(&self, key: Vec<u8>) -> Result<()> {
        if let Some(connection) = &self.connection {
            // Note: Encryption is now handled by the codec
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }
} 
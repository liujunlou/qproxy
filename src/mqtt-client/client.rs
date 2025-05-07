use crate::message::{
    ConnectMessage, ConnAckMessage, DisconnectMessage, Header, Message, MessageType, PingMessage,
    PublishMessage, QoS, SubscribeMessage,
};
use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};
use futures::{Sink, Stream};
use log::{debug, error, info};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::mpsc,
    time,
};
use tokio_util::codec::{Decoder, Encoder, Framed};

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
    subscriptions: Arc<Mutex<HashMap<String, Vec<QoS>>>>,
}

struct Connection {
    framed: Framed<TcpStream, MqttCodec>,
    tx: mpsc::Sender<Message>,
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
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
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

    pub async fn connect(&mut self) -> Result<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let stream = TcpStream::connect(&addr).await?;
        
        let (tx, mut rx) = mpsc::channel(100);
        let framed = Framed::new(stream, MqttCodec::new());
        
        let mut connect = ConnectMessage::new(self.client_id.clone(), self.keep_alive);
        connect.clean_session = self.clean_session;
        connect.username = self.username.clone();
        connect.password = self.password.clone();
        
        let header = Header::new(MessageType::Connect);
        let message = Message::new(header, connect.encode().freeze());
        
        let mut connection = Connection { framed, tx };
        connection.framed.send(message).await?;
        
        // Wait for CONNACK
        if let Some(connack) = connection.framed.next().await {
            let connack = connack?;
            if connack.header.message_type != MessageType::ConnAck {
                return Err(anyhow!("Expected CONNACK, got {:?}", connack.header.message_type));
            }
            
            let connack = ConnAckMessage::decode(&connack.payload)?;
            if connack.return_code != 0 {
                return Err(anyhow!("Connection refused: {}", connack.return_code));
            }
            
            info!("Connected to MQTT broker");
            self.connection = Some(connection);
            
            // Start keep-alive task
            let keep_alive = self.keep_alive;
            let tx = self.connection.as_ref().unwrap().tx.clone();
            tokio::spawn(async move {
                let mut interval = time::interval(Duration::from_secs(keep_alive as u64 / 2));
                loop {
                    interval.tick().await;
                    let ping = Message::new(Header::new(MessageType::Ping), Bytes::new());
                    if tx.send(ping).await.is_err() {
                        break;
                    }
                }
            });
            
            // Start message processing task
            let mut framed = self.connection.as_ref().unwrap().framed.clone();
            let subscriptions = self.subscriptions.clone();
            tokio::spawn(async move {
                while let Some(result) = framed.next().await {
                    match result {
                        Ok(message) => {
                            match message.header.message_type {
                                MessageType::Publish => {
                                    let topic_len = (message.payload[0] as usize) << 8 | message.payload[1] as usize;
                                    let topic = String::from_utf8_lossy(&message.payload[2..2 + topic_len]).to_string();
                                    let payload = message.payload.slice(2 + topic_len..);
                                    
                                    if let Some(qos_levels) = subscriptions.lock().unwrap().get(&topic) {
                                        for qos in qos_levels {
                                            debug!("Received message on topic {} with QoS {:?}", topic, qos);
                                            // Handle message based on QoS
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
                }
            });
            
            Ok(())
        } else {
            Err(anyhow!("Connection closed"))
        }
    }

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

    pub async fn subscribe(&mut self, topic: String, qos: QoS) -> Result<()> {
        let mut packet_id = self.packet_id.lock().unwrap();
        *packet_id = packet_id.wrapping_add(1);
        
        let subscribe = SubscribeMessage::new(*packet_id, vec![(topic.clone(), qos)]);
        let header = Header::new(MessageType::Subscribe);
        let message = Message::new(header, subscribe.encode().freeze());
        
        if let Some(connection) = &mut self.connection {
            connection.framed.send(message).await?;
            
            // Add to local subscriptions
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

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(connection) = &mut self.connection {
            let disconnect = Message::new(Header::new(MessageType::Disconnect), Bytes::new());
            connection.framed.send(disconnect).await?;
            self.connection = None;
            Ok(())
        } else {
            Err(anyhow!("Not connected"))
        }
    }
}

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
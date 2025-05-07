use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::{self, Read, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    NotSupport = -1,
    Reserve1 = 0,
    Connect = 1,
    ConnAck = 2,
    Publish = 3,
    PubAck = 4,
    Query = 5,
    QueryAck = 6,
    QueryCon = 7,
    Reconnect = 8,
    ReconnectAck = 9,
    Unsubscribe = 10,
    UnsubAck = 11,
    Ping = 12,
    Pong = 13,
    Disconnect = 14,
    Reserve2 = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QoS {
    AtMostOnce = 0,
    AtLeastOnce = 1,
    ExactlyOnce = 2,
}

#[derive(Debug, Clone)]
pub struct Header {
    pub message_type: MessageType,
    pub dup: bool,
    pub qos: QoS,
    pub retain: bool,
}

impl Header {
    pub fn new(message_type: MessageType) -> Self {
        Self {
            message_type,
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
        }
    }

    pub fn encode(&self) -> u8 {
        let mut byte = (self.message_type as u8) << 4;
        byte |= (self.dup as u8) << 3;
        byte |= (self.qos as u8) << 1;
        byte |= self.retain as u8;
        byte
    }

    pub fn decode(byte: u8) -> Self {
        let message_type = match (byte >> 4) & 0x0F {
            0 => MessageType::Reserve1,
            1 => MessageType::Connect,
            2 => MessageType::ConnAck,
            3 => MessageType::Publish,
            4 => MessageType::PubAck,
            5 => MessageType::Query,
            6 => MessageType::QueryAck,
            7 => MessageType::QueryCon,
            8 => MessageType::Reconnect,
            9 => MessageType::ReconnectAck,
            10 => MessageType::Unsubscribe,
            11 => MessageType::UnsubAck,
            12 => MessageType::Ping,
            13 => MessageType::Pong,
            14 => MessageType::Disconnect,
            15 => MessageType::Reserve2,
            _ => MessageType::NotSupport,
        };

        let dup = (byte & 0x08) != 0;
        let qos = match (byte >> 1) & 0x03 {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            2 => QoS::ExactlyOnce,
            _ => panic!("Invalid QoS"),
        };
        let retain = (byte & 0x01) != 0;

        Self {
            message_type,
            dup,
            qos,
            retain,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub header: Header,
    pub payload: Bytes,
}

impl Message {
    pub fn new(header: Header, payload: Bytes) -> Self {
        Self { header, payload }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u8(self.header.encode());
        
        let remaining_length = self.payload.len();
        let mut len = remaining_length;
        let mut digit;
        
        loop {
            digit = len % 128;
            len /= 128;
            if len > 0 {
                digit |= 0x80;
            }
            buf.put_u8(digit as u8);
            if len == 0 {
                break;
            }
        }
        
        buf.extend_from_slice(&self.payload);
        buf
    }

    pub fn decode(mut buf: &[u8]) -> io::Result<Self> {
        if buf.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Empty buffer"));
        }

        let header_byte = buf.get_u8();
        let header = Header::decode(header_byte);

        let mut multiplier = 1;
        let mut remaining_length = 0;
        let mut digit;

        loop {
            if buf.is_empty() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Incomplete remaining length"));
            }
            digit = buf.get_u8();
            remaining_length += (digit & 0x7F) as usize * multiplier;
            multiplier *= 128;
            if multiplier > 128 * 128 * 128 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Malformed remaining length"));
            }
            if (digit & 0x80) == 0 {
                break;
            }
        }

        if buf.len() < remaining_length {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Incomplete payload"));
        }

        let payload = Bytes::copy_from_slice(&buf[..remaining_length]);
        Ok(Message { header, payload })
    }
}

#[derive(Debug, Clone)]
pub struct ConnectMessage {
    pub client_id: String,
    pub keep_alive: u16,
    pub clean_session: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl ConnectMessage {
    pub fn new(client_id: String, keep_alive: u16) -> Self {
        Self {
            client_id,
            keep_alive,
            clean_session: true,
            username: None,
            password: None,
        }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Protocol name
        buf.put_u16(4); // Length of "MQTT"
        buf.extend_from_slice(b"MQTT");
        
        // Protocol version
        buf.put_u8(4); // MQTT 3.1.1
        
        // Connect flags
        let mut flags = 0u8;
        if self.clean_session {
            flags |= 0x02;
        }
        if self.username.is_some() {
            flags |= 0x80;
        }
        if self.password.is_some() {
            flags |= 0x40;
        }
        buf.put_u8(flags);
        
        // Keep alive
        buf.put_u16(self.keep_alive);
        
        // Client ID
        let client_id_bytes = self.client_id.as_bytes();
        buf.put_u16(client_id_bytes.len() as u16);
        buf.extend_from_slice(client_id_bytes);
        
        // Username
        if let Some(ref username) = self.username {
            let username_bytes = username.as_bytes();
            buf.put_u16(username_bytes.len() as u16);
            buf.extend_from_slice(username_bytes);
        }
        
        // Password
        if let Some(ref password) = self.password {
            let password_bytes = password.as_bytes();
            buf.put_u16(password_bytes.len() as u16);
            buf.extend_from_slice(password_bytes);
        }
        
        buf
    }
}

#[derive(Debug, Clone)]
pub struct ConnAckMessage {
    pub session_present: bool,
    pub return_code: u8,
}

impl ConnAckMessage {
    pub fn new(session_present: bool, return_code: u8) -> Self {
        Self {
            session_present,
            return_code,
        }
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid CONNACK message"));
        }
        
        let session_present = (buf[0] & 0x01) != 0;
        let return_code = buf[1];
        
        Ok(Self {
            session_present,
            return_code,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PublishMessage {
    pub topic: String,
    pub payload: Bytes,
    pub packet_id: Option<u16>,
}

impl PublishMessage {
    pub fn new(topic: String, payload: Bytes) -> Self {
        Self {
            topic,
            payload,
            packet_id: None,
        }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Topic
        let topic_bytes = self.topic.as_bytes();
        buf.put_u16(topic_bytes.len() as u16);
        buf.extend_from_slice(topic_bytes);
        
        // Packet ID (if QoS > 0)
        if let Some(packet_id) = self.packet_id {
            buf.put_u16(packet_id);
        }
        
        // Payload
        buf.extend_from_slice(&self.payload);
        
        buf
    }
}

#[derive(Debug, Clone)]
pub struct SubscribeMessage {
    pub packet_id: u16,
    pub subscriptions: Vec<(String, QoS)>,
}

impl SubscribeMessage {
    pub fn new(packet_id: u16, subscriptions: Vec<(String, QoS)>) -> Self {
        Self {
            packet_id,
            subscriptions,
        }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Packet ID
        buf.put_u16(self.packet_id);
        
        // Subscriptions
        for (topic, qos) in &self.subscriptions {
            let topic_bytes = topic.as_bytes();
            buf.put_u16(topic_bytes.len() as u16);
            buf.extend_from_slice(topic_bytes);
            buf.put_u8(*qos as u8);
        }
        
        buf
    }
}

#[derive(Debug, Clone)]
pub struct PingMessage;

impl PingMessage {
    pub fn encode(&self) -> BytesMut {
        BytesMut::new()
    }
}

#[derive(Debug, Clone)]
pub struct DisconnectMessage;

impl DisconnectMessage {
    pub fn encode(&self) -> BytesMut {
        BytesMut::new()
    }
} 
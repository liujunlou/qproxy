use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::{self, Read};

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
pub struct RetryableMessage {
    pub message_id: u16,
}

impl RetryableMessage {
    pub fn new(message_id: u16) -> Self {
        Self { message_id }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u16(self.message_id);
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid message ID"));
        }
        let message_id = ((buf[0] as u16) << 8) | (buf[1] as u16);
        Ok(Self { message_id })
    }
}

#[derive(Debug, Clone)]
pub struct ConnectMessage {
    pub protocol_id: String,
    pub protocol_version: u8,
    pub client_id: String,
    pub keep_alive: u16,
    pub app_id: Option<String>,
    pub token: Option<String>,
    pub app_identifier: Option<String>,
    pub info: Option<String>,
    pub info_qos: Option<QoS>,
    pub retain_info: bool,
    pub is_not_kick_other: bool,
    pub client_ip: Option<String>,
    pub validation_info: Option<String>,
    pub has_reconnect: bool,
    pub callback_ext: Option<String>,
    pub signature: Option<Vec<u8>>,
    pub verify_sign: Option<Vec<u8>>,
}

impl ConnectMessage {
    pub fn new() -> Self {
        Self {
            protocol_id: "RCloud".to_string(),
            protocol_version: 3,
            client_id: String::new(),
            keep_alive: 0,
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

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Protocol ID
        let protocol_id_bytes = self.protocol_id.as_bytes();
        buf.put_u16(protocol_id_bytes.len() as u16);
        buf.extend_from_slice(protocol_id_bytes);
        
        // Protocol version
        buf.put_u8(self.protocol_version);
        
        // Connect flags
        let mut flags = 0u8;
        if self.client_ip.is_some() {
            flags |= 0x01;
        }
        if self.is_not_kick_other {
            flags |= 0x02;
        }
        if self.info.is_some() {
            flags |= 0x04;
        }
        if let Some(qos) = self.info_qos {
            flags |= (qos as u8) << 3;
        }
        if self.retain_info {
            flags |= 0x20;
        }
        if self.token.is_some() {
            flags |= 0x40;
        }
        if self.app_id.is_some() {
            flags |= 0x80;
        }
        buf.put_u8(flags);

        // V4 flags if protocol version is 4
        if self.protocol_version == 4 {
            let mut v4_flags = 0u8;
            if self.has_reconnect {
                v4_flags |= 0x01;
            }
            if self.callback_ext.is_some() {
                v4_flags |= 0x02;
            }
            if self.signature.is_some() {
                v4_flags |= 0x80;
            }
            buf.put_u8(v4_flags);
        }
        
        // Keep alive
        buf.put_u16(self.keep_alive);
        
        // Client ID
        let client_id_bytes = self.client_id.as_bytes();
        buf.put_u16(client_id_bytes.len() as u16);
        buf.extend_from_slice(client_id_bytes);

        // App identifier and info if present
        if let Some(ref app_identifier) = self.app_identifier {
            let app_identifier_bytes = app_identifier.as_bytes();
            buf.put_u16(app_identifier_bytes.len() as u16);
            buf.extend_from_slice(app_identifier_bytes);
        }
        if let Some(ref info) = self.info {
            let info_bytes = info.as_bytes();
            buf.put_u16(info_bytes.len() as u16);
            buf.extend_from_slice(info_bytes);
        }
        
        // App ID if present
        if let Some(ref app_id) = self.app_id {
            let app_id_bytes = app_id.as_bytes();
            buf.put_u16(app_id_bytes.len() as u16);
            buf.extend_from_slice(app_id_bytes);
        }
        
        // Token if present
        if let Some(ref token) = self.token {
            let token_bytes = token.as_bytes();
            buf.put_u16(token_bytes.len() as u16);
            buf.extend_from_slice(token_bytes);
        }

        // Client IP if present
        if let Some(ref client_ip) = self.client_ip {
            let client_ip_bytes = client_ip.as_bytes();
            buf.put_u16(client_ip_bytes.len() as u16);
            buf.extend_from_slice(client_ip_bytes);
        }

        // Validation info if present
        if let Some(ref validation_info) = self.validation_info {
            let validation_info_bytes = validation_info.as_bytes();
            buf.put_u16(validation_info_bytes.len() as u16);
            buf.extend_from_slice(validation_info_bytes);
        }

        // V4 specific fields
        if self.protocol_version == 4 {
            // Callback extension if present
            if let Some(ref callback_ext) = self.callback_ext {
                let callback_ext_bytes = callback_ext.as_bytes();
                buf.put_u16(callback_ext_bytes.len() as u16);
                buf.extend_from_slice(callback_ext_bytes);
            }

            // Signature if present
            if let Some(ref signature) = self.signature {
                buf.extend_from_slice(signature);
            }
        }
        
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        let mut cursor = std::io::Cursor::new(buf);
        
        // Protocol ID
        let protocol_id_len = cursor.get_u16() as usize;
        let mut protocol_id_bytes = vec![0u8; protocol_id_len];
        cursor.read_exact(&mut protocol_id_bytes)?;
        let protocol_id = String::from_utf8(protocol_id_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        
        // Protocol version
        let protocol_version = cursor.get_u8();
        
        // Connect flags
        let flags = cursor.get_u8();
        let has_client_ip = (flags & 0x01) != 0;
        let is_not_kick_other = (flags & 0x02) != 0;
        let has_info = (flags & 0x04) != 0;
        let info_qos = match (flags >> 3) & 0x03 {
            0 => None,
            1 => Some(QoS::AtLeastOnce),
            2 => Some(QoS::ExactlyOnce),
            _ => None,
        };
        let retain_info = (flags & 0x20) != 0;
        let has_token = (flags & 0x40) != 0;
        let has_app_id = (flags & 0x80) != 0;

        // V4 flags if protocol version is 4
        let mut has_reconnect = false;
        let mut has_callback_ext = false;
        let mut has_signature = false;
        if protocol_version == 4 {
            let v4_flags = cursor.get_u8();
            has_reconnect = (v4_flags & 0x01) != 0;
            has_callback_ext = (v4_flags & 0x02) != 0;
            has_signature = (v4_flags & 0x80) != 0;
        }
        
        // Keep alive
        let keep_alive = cursor.get_u16();
        
        // Client ID
        let client_id_len = cursor.get_u16() as usize;
        let mut client_id_bytes = vec![0u8; client_id_len];
        cursor.read_exact(&mut client_id_bytes)?;
        let client_id = String::from_utf8(client_id_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // App identifier and info if present
        let mut app_identifier = None;
        let mut info = None;
        if has_info {
            let app_identifier_len = cursor.get_u16() as usize;
            let mut app_identifier_bytes = vec![0u8; app_identifier_len];
            cursor.read_exact(&mut app_identifier_bytes)?;
            app_identifier = Some(String::from_utf8(app_identifier_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);

            let info_len = cursor.get_u16() as usize;
            let mut info_bytes = vec![0u8; info_len];
            cursor.read_exact(&mut info_bytes)?;
            info = Some(String::from_utf8(info_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
        }

        // App ID if present
        let mut app_id = None;
        if has_app_id {
            let app_id_len = cursor.get_u16() as usize;
            let mut app_id_bytes = vec![0u8; app_id_len];
            cursor.read_exact(&mut app_id_bytes)?;
            app_id = Some(String::from_utf8(app_id_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
        }

        // Token if present
        let mut token = None;
        if has_token {
            let token_len = cursor.get_u16() as usize;
            let mut token_bytes = vec![0u8; token_len];
            cursor.read_exact(&mut token_bytes)?;
            token = Some(String::from_utf8(token_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
        }

        // Client IP if present
        let mut client_ip = None;
        if has_client_ip {
            let client_ip_len = cursor.get_u16() as usize;
            let mut client_ip_bytes = vec![0u8; client_ip_len];
            cursor.read_exact(&mut client_ip_bytes)?;
            client_ip = Some(String::from_utf8(client_ip_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
        }

        // Validation info if present
        let mut validation_info = None;
        if retain_info {
            let validation_info_len = cursor.get_u16() as usize;
            let mut validation_info_bytes = vec![0u8; validation_info_len];
            cursor.read_exact(&mut validation_info_bytes)?;
            validation_info = Some(String::from_utf8(validation_info_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
        }

        // V4 specific fields
        let mut callback_ext = None;
        let mut signature = None;
        let mut verify_sign = None;
        if protocol_version == 4 {
            // Callback extension if present
            if has_callback_ext {
                let callback_ext_len = cursor.get_u16() as usize;
                let mut callback_ext_bytes = vec![0u8; callback_ext_len];
                cursor.read_exact(&mut callback_ext_bytes)?;
                callback_ext = Some(String::from_utf8(callback_ext_bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
            }

            // Signature if present
            if has_signature {
                let mut signature_bytes = vec![0u8; 8];
                cursor.read_exact(&mut signature_bytes)?;
                signature = Some(signature_bytes);

                // Calculate verify sign from payload
                let payload = &buf[..cursor.position() as usize - 8];
                verify_sign = Some(digest_util::init_signature(payload));
            }
        }

        Ok(Self {
            protocol_id,
            protocol_version,
            client_id,
            keep_alive,
            app_id,
            token,
            app_identifier,
            info,
            info_qos,
            retain_info,
            is_not_kick_other,
            client_ip,
            validation_info,
            has_reconnect,
            callback_ext,
            signature,
            verify_sign,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ConnAckMessage {
    pub status: u8,
    pub user_id: Option<String>,
    pub session: Option<String>,
    pub timestamp: Option<u64>,
    pub message_id: Option<u16>,
    pub online_client_info: Option<String>,
    pub decryption_type: u8,
    pub secret_key: Option<String>,
}

impl ConnAckMessage {
    pub fn new(status: u8) -> Self {
        Self {
            status,
            user_id: None,
            session: None,
            timestamp: None,
            message_id: None,
            online_client_info: None,
            decryption_type: 0,
            secret_key: None,
        }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Status
        buf.put_u8(0); // Reserved byte
        buf.put_u8(self.status);

        if self.status == 0 {
            // User ID
            if let Some(ref user_id) = self.user_id {
                let user_id_bytes = user_id.as_bytes();
                buf.put_u16(user_id_bytes.len() as u16);
                buf.extend_from_slice(user_id_bytes);
            }

            // Session
            if let Some(ref session) = self.session {
                let session_bytes = session.as_bytes();
                buf.put_u16(session_bytes.len() as u16);
                buf.extend_from_slice(session_bytes);
            }

            // Timestamp
            if let Some(timestamp) = self.timestamp {
                buf.put_u64(timestamp);
            }

            // Message ID
            if let Some(message_id) = self.message_id {
                buf.put_u16(message_id);
            }

            // Online client info
            if let Some(ref online_client_info) = self.online_client_info {
                let info_bytes = online_client_info.as_bytes();
                buf.put_u16(info_bytes.len() as u16);
                buf.extend_from_slice(info_bytes);
            }

            // Decryption type and secret key
            buf.put_u8(self.decryption_type);
            if self.decryption_type > 0 {
                if let Some(ref secret_key) = self.secret_key {
                    let key_bytes = secret_key.as_bytes();
                    buf.put_u16(key_bytes.len() as u16);
                    buf.extend_from_slice(key_bytes);
                }
            }
        }
        
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        let mut cursor = std::io::Cursor::new(buf);
        
        // Skip reserved byte
        cursor.get_u8();
        
        // Status
        let status = cursor.get_u8();
        
        let mut user_id = None;
        let mut session = None;
        let mut timestamp = None;
        let mut message_id = None;
        let mut online_client_info = None;
        let mut decryption_type = 0u8;
        let mut secret_key = None;

        if status == 0 {
            // User ID
            let user_id_len = cursor.get_u16() as usize;
            let mut user_id_bytes = vec![0u8; user_id_len];
            cursor.read_exact(&mut user_id_bytes)?;
            user_id = Some(String::from_utf8(user_id_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);

            // Session
            let session_len = cursor.get_u16() as usize;
            let mut session_bytes = vec![0u8; session_len];
            cursor.read_exact(&mut session_bytes)?;
            session = Some(String::from_utf8(session_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);

            // Timestamp
            timestamp = Some(cursor.get_u64());

            // Message ID
            message_id = Some(cursor.get_u16());

            // Online client info
            let info_len = cursor.get_u16() as usize;
            let mut info_bytes = vec![0u8; info_len];
            cursor.read_exact(&mut info_bytes)?;
            online_client_info = Some(String::from_utf8(info_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);

            // Decryption type and secret key
            decryption_type = cursor.get_u8();
            if decryption_type > 0 {
                let key_len = cursor.get_u16() as usize;
                let mut key_bytes = vec![0u8; key_len];
                cursor.read_exact(&mut key_bytes)?;
                secret_key = Some(String::from_utf8(key_bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
            }
        }

        Ok(Self {
            status,
            user_id,
            session,
            timestamp,
            message_id,
            online_client_info,
            decryption_type,
            secret_key,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PublishMessage {
    pub topic: String,
    pub payload: Bytes,
    pub packet_id: Option<u16>,
    pub target_id: Option<String>,
    pub signature: Option<Vec<u8>>,
    pub verify_sign: Option<Vec<u8>>,
}

impl PublishMessage {
    pub fn new(topic: String, payload: Bytes) -> Self {
        Self {
            topic,
            payload,
            packet_id: None,
            target_id: None,
            signature: None,
            verify_sign: None,
        }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Topic
        let topic_bytes = self.topic.as_bytes();
        buf.put_u16(topic_bytes.len() as u16);
        buf.extend_from_slice(topic_bytes);
        
        // Target ID if present
        if let Some(ref target_id) = self.target_id {
            let target_id_bytes = target_id.as_bytes();
            buf.put_u16(target_id_bytes.len() as u16);
            buf.extend_from_slice(target_id_bytes);
        }
        
        // Packet ID if present
        if let Some(packet_id) = self.packet_id {
            buf.put_u16(packet_id);
        }
        
        // Payload
        buf.extend_from_slice(&self.payload);
        
        // Signature if present
        if let Some(ref signature) = self.signature {
            buf.extend_from_slice(signature);
        }
        
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        let mut cursor = std::io::Cursor::new(buf);
        
        // Topic
        let topic_len = cursor.get_u16() as usize;
        let mut topic_bytes = vec![0u8; topic_len];
        cursor.read_exact(&mut topic_bytes)?;
        let topic = String::from_utf8(topic_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        
        // Target ID
        let target_id_len = cursor.get_u16() as usize;
        let mut target_id_bytes = vec![0u8; target_id_len];
        cursor.read_exact(&mut target_id_bytes)?;
        let target_id = Some(String::from_utf8(target_id_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
        
        // Packet ID
        let packet_id = Some(cursor.get_u16());
        
        // Payload
        let payload_len = buf.len() - cursor.position() as usize;
        let mut payload_bytes = vec![0u8; payload_len];
        cursor.read_exact(&mut payload_bytes)?;
        let payload = Bytes::from(payload_bytes);
        
        // Calculate verify sign from payload
        let verify_sign = Some(digest_util::init_signature(&payload));
        
        Ok(Self {
            topic,
            payload,
            packet_id,
            target_id,
            signature: None,
            verify_sign,
        })
    }
}

#[derive(Debug, Clone)]
pub struct QueryMessage {
    pub topic: String,
    pub payload: Bytes,
    pub target_id: Option<String>,
    pub signature: Option<Vec<u8>>,
    pub verify_sign: Option<Vec<u8>>,
}

impl QueryMessage {
    pub fn new(topic: String, payload: Bytes) -> Self {
        Self {
            topic,
            payload,
            target_id: None,
            signature: None,
            verify_sign: None,
        }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Topic
        let topic_bytes = self.topic.as_bytes();
        buf.put_u16(topic_bytes.len() as u16);
        buf.extend_from_slice(topic_bytes);
        
        // Target ID if present
        if let Some(ref target_id) = self.target_id {
            let target_id_bytes = target_id.as_bytes();
            buf.put_u16(target_id_bytes.len() as u16);
            buf.extend_from_slice(target_id_bytes);
        }
        
        // Payload
        buf.extend_from_slice(&self.payload);
        
        // Signature if present
        if let Some(ref signature) = self.signature {
            buf.extend_from_slice(signature);
        }
        
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        let mut cursor = std::io::Cursor::new(buf);
        
        // Topic
        let topic_len = cursor.get_u16() as usize;
        let mut topic_bytes = vec![0u8; topic_len];
        cursor.read_exact(&mut topic_bytes)?;
        let topic = String::from_utf8(topic_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        
        // Target ID
        let target_id_len = cursor.get_u16() as usize;
        let mut target_id_bytes = vec![0u8; target_id_len];
        cursor.read_exact(&mut target_id_bytes)?;
        let target_id = Some(String::from_utf8(target_id_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
        
        // Payload
        let payload_len = buf.len() - cursor.position() as usize;
        let mut payload_bytes = vec![0u8; payload_len];
        cursor.read_exact(&mut payload_bytes)?;
        let payload = Bytes::from(payload_bytes);
        
        // Calculate verify sign from payload
        let verify_sign = Some(digest_util::init_signature(&payload));
        
        Ok(Self {
            topic,
            payload,
            target_id,
            signature: None,
            verify_sign,
        })
    }
}

#[derive(Debug, Clone)]
pub struct QueryAckMessage {
    pub message_id: u16,
    pub status: u16,
    pub data: Option<Bytes>,
    pub date: u32,
}

impl QueryAckMessage {
    pub fn new(message_id: u16, status: u16, data: Option<Bytes>) -> Self {
        Self {
            message_id,
            status,
            data,
            date: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32),
        }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Message ID
        buf.put_u16(self.message_id);
        
        // Date
        buf.put_u32(self.date);
        
        // Status
        buf.put_u16(self.status);
        
        // Data if present
        if let Some(ref data) = self.data {
            buf.extend_from_slice(data);
        }
        
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        let mut cursor = std::io::Cursor::new(buf);
        
        // Message ID
        let message_id = cursor.get_u16();
        
        // Date
        let date = cursor.get_u32();
        
        // Status
        let status = cursor.get_u16();
        
        // Data if present
        let mut data = None;
        if cursor.position() < buf.len() as u64 {
            let data_len = buf.len() - cursor.position() as usize;
            let mut data_bytes = vec![0u8; data_len];
            cursor.read_exact(&mut data_bytes)?;
            data = Some(Bytes::from(data_bytes));
        }
        
        Ok(Self {
            message_id,
            status,
            data,
            date,
        })
    }
}

#[derive(Debug, Clone)]
pub struct QueryConMessage {
    pub message_id: u16,
}

impl QueryConMessage {
    pub fn new(message_id: u16) -> Self {
        Self { message_id }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u16(self.message_id);
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid message ID"));
        }
        let message_id = ((buf[0] as u16) << 8) | (buf[1] as u16);
        Ok(Self { message_id })
    }
}

#[derive(Debug, Clone)]
pub struct ReconnectMessage {
    pub session: String,
}

impl ReconnectMessage {
    pub fn new(session: String) -> Self {
        Self { session }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        let session_bytes = self.session.as_bytes();
        buf.put_u16(session_bytes.len() as u16);
        buf.extend_from_slice(session_bytes);
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        let mut cursor = std::io::Cursor::new(buf);
        let session_len = cursor.get_u16() as usize;
        let mut session_bytes = vec![0u8; session_len];
        cursor.read_exact(&mut session_bytes)?;
        let session = String::from_utf8(session_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Self { session })
    }
}

#[derive(Debug, Clone)]
pub struct ReconnectAckMessage {
    pub status: ReconnectStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectStatus {
    Accepted = 0,
    Error = 1,
    NoMaster = 2,
}

impl ReconnectAckMessage {
    pub fn new(status: ReconnectStatus) -> Self {
        Self { status }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u8(0); // Reserved byte
        buf.put_u8(self.status as u8);
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid message length"));
        }
        let status = match buf[1] {
            0 => ReconnectStatus::Accepted,
            1 => ReconnectStatus::Error,
            2 => ReconnectStatus::NoMaster,
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid status code")),
        };
        Ok(Self { status })
    }
}

#[derive(Debug, Clone)]
pub struct UnsubscribeMessage {
    pub message_id: u16,
    pub topics: Vec<String>,
}

impl UnsubscribeMessage {
    pub fn new(message_id: u16, topics: Vec<String>) -> Self {
        Self { message_id, topics }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Message ID
        buf.put_u16(self.message_id);
        
        // Topics
        for topic in &self.topics {
            let topic_bytes = topic.as_bytes();
            buf.put_u16(topic_bytes.len() as u16);
            buf.extend_from_slice(topic_bytes);
        }
        
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        let mut cursor = std::io::Cursor::new(buf);
        
        // Message ID
        let message_id = cursor.get_u16();
        
        // Topics
        let mut topics = Vec::new();
        while cursor.position() < buf.len() as u64 {
            let topic_len = cursor.get_u16() as usize;
            let mut topic_bytes = vec![0u8; topic_len];
            cursor.read_exact(&mut topic_bytes)?;
            let topic = String::from_utf8(topic_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            topics.push(topic);
        }
        
        Ok(Self { message_id, topics })
    }
}

#[derive(Debug, Clone)]
pub struct UnsubAckMessage {
    pub message_id: u16,
}

impl UnsubAckMessage {
    pub fn new(message_id: u16) -> Self {
        Self { message_id }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u16(self.message_id);
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid message ID"));
        }
        let message_id = ((buf[0] as u16) << 8) | (buf[1] as u16);
        Ok(Self { message_id })
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
pub struct PongMessage;

impl PongMessage {
    pub fn encode(&self) -> BytesMut {
        BytesMut::new()
    }
}

#[derive(Debug, Clone)]
pub struct DisconnectMessage {
    pub status: u8,
    pub kicked_client_info: Option<String>,
}

impl DisconnectMessage {
    pub fn new(status: u8, kicked_client_info: Option<String>) -> Self {
        Self {
            status,
            kicked_client_info,
        }
    }

    pub fn encode(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        
        // Reserved byte and status
        buf.put_u8(0);
        buf.put_u8(self.status);
        
        // Kicked client info if present
        if let Some(ref info) = self.kicked_client_info {
            let info_bytes = info.as_bytes();
            buf.put_u16(info_bytes.len() as u16);
            buf.extend_from_slice(info_bytes);
        }
        
        buf
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        let mut cursor = std::io::Cursor::new(buf);
        
        // Skip reserved byte
        cursor.get_u8();
        
        // Status
        let status = cursor.get_u8();
        
        // Kicked client info if present
        let mut kicked_client_info = None;
        if cursor.position() < buf.len() as u64 {
            let info_len = cursor.get_u16() as usize;
            let mut info_bytes = vec![0u8; info_len];
            cursor.read_exact(&mut info_bytes)?;
            kicked_client_info = Some(String::from_utf8(info_bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
        }
        
        Ok(Self {
            status,
            kicked_client_info,
        })
    }
}

// Add DigestUtil for signature calculation
mod digest_util {
    const SIGNATURE_MAX_LEN: usize = 8;
    
    pub fn init_signature(data: &[u8]) -> Vec<u8> {
        // Simple implementation without external dependencies
        let mut result = vec![0u8; SIGNATURE_MAX_LEN];
        for (i, &byte) in data.iter().enumerate().take(data.len()) {
            result[i % SIGNATURE_MAX_LEN] ^= byte;
        }
        result
    }
} 
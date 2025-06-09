use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use std::io;
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MqttMessage {
    Connect(ConnectMessage),
    ConnAck(ConnAckMessage),
    Query(QueryMessage),
    QueryAck(QueryAckMessage),
    QueryConn(QueryConnMessage),
    Publish(PublishMessage),
    PubAck(PubAckMessage),
    Subscribe(SubscribeMessage),
    SubAck(SubAckMessage),
    Unsubscribe(UnsubscribeMessage),
    UnsubAck(UnsubAckMessage),
    PingReq(PingReqMessage),
    PingResp(PingRespMessage),
    Disconnect(DisconnectMessage),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QoS {
    AtMostOnce = 0,
    AtLeastOnce = 1,
    ExactlyOnce = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectMessage {
    pub device_id: String,
    pub clean_session: bool,
    pub keep_alive: u32,
    pub credentials: Option<Credentials>,
    pub will: Option<WillMessage>,
    pub protocol_version: u8,
    pub protocol_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnAckMessage {
    pub status: u8,
    pub session_present: bool,
    pub return_code: u8,
    pub properties: Option<ConnAckProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnAckProperties {
    pub session_expiry_interval: Option<u32>,
    pub receive_maximum: Option<u16>,
    pub maximum_qos: Option<u8>,
    pub retain_available: Option<bool>,
    pub maximum_packet_size: Option<u32>,
    pub assigned_client_identifier: Option<String>,
    pub topic_alias_maximum: Option<u16>,
    pub reason_string: Option<String>,
    pub wildcard_subscription_available: Option<bool>,
    pub subscription_identifiers_available: Option<bool>,
    pub shared_subscription_available: Option<bool>,
    pub server_keep_alive: Option<u16>,
    pub response_information: Option<String>,
    pub server_reference: Option<String>,
    pub authentication_method: Option<String>,
    pub authentication_data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryMessage {
    #[serde(rename = "type")]
    pub query_type: String,
    pub payload: String,
    pub device_id: String,
    pub message_id: Option<u16>,
    pub qos: QoS,
    pub retain: bool,
    pub dup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryAckMessage {
    #[serde(rename = "type")]
    pub query_type: String,
    pub status: u8,
    pub message_id: Option<u16>,
    pub reason_code: Option<u8>,
    pub properties: Option<QueryAckProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryAckProperties {
    pub reason_string: Option<String>,
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConnMessage {
    #[serde(rename = "type")]
    pub query_type: String,
    pub payload: String,
    pub device_id: String,
    pub message_id: Option<u16>,
    pub qos: QoS,
    pub retain: bool,
    pub dup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishMessage {
    pub topic: String,
    pub payload: String,
    pub message_id: Option<u16>,
    pub qos: QoS,
    pub retain: bool,
    pub dup: bool,
    pub properties: Option<PublishProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishProperties {
    pub payload_format_indicator: Option<u8>,
    pub message_expiry_interval: Option<u32>,
    pub topic_alias: Option<u16>,
    pub response_topic: Option<String>,
    pub correlation_data: Option<Vec<u8>>,
    pub user_property: Option<Vec<(String, String)>>,
    pub subscription_identifier: Option<u32>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PubAckMessage {
    pub message_id: u16,
    pub reason_code: Option<u8>,
    pub properties: Option<PubAckProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PubAckProperties {
    pub reason_string: Option<String>,
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeMessage {
    pub message_id: u16,
    pub subscriptions: Vec<Subscription>,
    pub properties: Option<SubscribeProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub topic_filter: String,
    pub qos: QoS,
    pub no_local: bool,
    pub retain_as_published: bool,
    pub retain_handling: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeProperties {
    pub subscription_identifier: Option<u32>,
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAckMessage {
    pub message_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Option<SubAckProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAckProperties {
    pub reason_string: Option<String>,
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeMessage {
    pub message_id: u16,
    pub topic_filters: Vec<String>,
    pub properties: Option<UnsubscribeProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeProperties {
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubAckMessage {
    pub message_id: u16,
    pub reason_codes: Vec<u8>,
    pub properties: Option<UnsubAckProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubAckProperties {
    pub reason_string: Option<String>,
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingReqMessage {
    pub properties: Option<PingReqProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingReqProperties {
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingRespMessage {
    pub properties: Option<PingRespProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingRespProperties {
    pub reason_string: Option<String>,
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisconnectMessage {
    pub reason_code: Option<u8>,
    pub properties: Option<DisconnectProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisconnectProperties {
    pub session_expiry_interval: Option<u32>,
    pub reason_string: Option<String>,
    pub server_reference: Option<String>,
    pub user_property: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub app_key: String,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WillMessage {
    pub topic: String,
    pub payload: String,
    pub qos: QoS,
    pub retain: bool,
    pub properties: Option<WillProperties>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WillProperties {
    pub payload_format_indicator: Option<u8>,
    pub message_expiry_interval: Option<u32>,
    pub content_type: Option<String>,
    pub response_topic: Option<String>,
    pub correlation_data: Option<Vec<u8>>,
    pub user_property: Option<Vec<(String, String)>>,
    pub will_delay_interval: Option<u32>,
}

pub struct MqttCodec {
    max_frame_length: usize,
}

impl MqttCodec {
    pub fn new(max_frame_length: usize) -> Self {
        Self { max_frame_length }
    }
}

impl Decoder for MqttCodec {
    type Item = MqttMessage;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }

        // Read message length (first 4 bytes)
        let mut length_bytes = [0u8; 4];
        src.copy_to_slice(&mut length_bytes);
        let length = u32::from_be_bytes(length_bytes) as usize;

        if length > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Frame of length {} is too large", length),
            ));
        }

        if src.len() < length + 4 {
            return Ok(None);
        }

        // Read message type (1 byte after length)
        let msg_type = src[4];

        // Read the actual message payload
        let payload = src[5..5 + length].to_vec();
        src.advance(length + 5);

        // Deserialize based on message type
        let message = match msg_type {
            1 => {
                let connect: ConnectMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::Connect(connect)
            }
            2 => {
                let conn_ack: ConnAckMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::ConnAck(conn_ack)
            }
            3 => {
                let query: QueryMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::Query(query)
            }
            4 => {
                let query_ack: QueryAckMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::QueryAck(query_ack)
            }
            5 => {
                let query_conn: QueryConnMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::QueryConn(query_conn)
            }
            6 => {
                let publish: PublishMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::Publish(publish)
            }
            7 => {
                let pub_ack: PubAckMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::PubAck(pub_ack)
            }
            8 => {
                let subscribe: SubscribeMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::Subscribe(subscribe)
            }
            9 => {
                let sub_ack: SubAckMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::SubAck(sub_ack)
            }
            10 => {
                let unsubscribe: UnsubscribeMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::Unsubscribe(unsubscribe)
            }
            11 => {
                let unsub_ack: UnsubAckMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::UnsubAck(unsub_ack)
            }
            12 => {
                let ping_req: PingReqMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::PingReq(ping_req)
            }
            13 => {
                let ping_resp: PingRespMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::PingResp(ping_resp)
            }
            14 => {
                let disconnect: DisconnectMessage = serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                MqttMessage::Disconnect(disconnect)
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unknown message type: {}", msg_type),
                ))
            }
        };

        Ok(Some(message))
    }
}

impl Encoder<MqttMessage> for MqttCodec {
    type Error = io::Error;

    fn encode(&mut self, item: MqttMessage, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let (msg_type, payload) = match item {
            MqttMessage::Connect(msg) => (1, serde_json::to_vec(&msg)?),
            MqttMessage::ConnAck(msg) => (2, serde_json::to_vec(&msg)?),
            MqttMessage::Query(msg) => (3, serde_json::to_vec(&msg)?),
            MqttMessage::QueryAck(msg) => (4, serde_json::to_vec(&msg)?),
            MqttMessage::QueryConn(msg) => (5, serde_json::to_vec(&msg)?),
            MqttMessage::Publish(msg) => (6, serde_json::to_vec(&msg)?),
            MqttMessage::PubAck(msg) => (7, serde_json::to_vec(&msg)?),
            MqttMessage::Subscribe(msg) => (8, serde_json::to_vec(&msg)?),
            MqttMessage::SubAck(msg) => (9, serde_json::to_vec(&msg)?),
            MqttMessage::Unsubscribe(msg) => (10, serde_json::to_vec(&msg)?),
            MqttMessage::UnsubAck(msg) => (11, serde_json::to_vec(&msg)?),
            MqttMessage::PingReq(msg) => (12, serde_json::to_vec(&msg)?),
            MqttMessage::PingResp(msg) => (13, serde_json::to_vec(&msg)?),
            MqttMessage::Disconnect(msg) => (14, serde_json::to_vec(&msg)?),
        };

        let length = payload.len() as u32;

        // Write message length (4 bytes)
        dst.put_u32(length);

        // Write message type (1 byte)
        dst.put_u8(msg_type);

        // Write payload
        dst.extend_from_slice(&payload);

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    Connect = 1,
    ConnAck = 2,
    Publish = 3,
    PubAck = 4,
    Subscribe = 8,
    SubAck = 9,
    Unsubscribe = 10,
    UnsubAck = 11,
    PingReq = 12,
    PingResp = 13,
    Disconnect = 14,
    Query = 15,
    QueryAck = 16,
    QueryConn = 17,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub header: MessageHeader,
    #[serde(with = "bytes_serde")]
    pub payload: Bytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageHeader {
    pub message_type: MessageType,
    pub qos: QoS,
    pub retain: bool,
    pub dup: bool,
}

impl PublishMessage {
    pub fn decode(payload: &[u8]) -> Result<Self, io::Error> {
        serde_json::from_slice(payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn encode(&self) -> Bytes {
        Bytes::from(serde_json::to_vec(self).unwrap())
    }

    pub fn remove(&mut self, field: &str) {
        match field {
            "topic" => self.topic = String::new(),
            "payload" => self.payload = String::new(),
            "message_id" => self.message_id = None,
            "qos" => self.qos = QoS::AtMostOnce,
            "retain" => self.retain = false,
            "dup" => self.dup = false,
            "properties" => self.properties = None,
            _ => {}
        }
    }

    pub fn field_names() -> Vec<&'static str> {
        vec![
            "topic",
            "payload",
            "message_id",
            "qos",
            "retain",
            "dup",
            "properties",
        ]
    }
}

impl QueryMessage {
    pub fn decode(payload: &[u8]) -> Result<Self, io::Error> {
        serde_json::from_slice(payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn encode(&self) -> Bytes {
        Bytes::from(serde_json::to_vec(self).unwrap())
    }

    pub fn remove(&mut self, field: &str) {
        match field {
            "type" => self.query_type = String::new(),
            "payload" => self.payload = String::new(),
            "device_id" => self.device_id = String::new(),
            "message_id" => self.message_id = None,
            "qos" => self.qos = QoS::AtMostOnce,
            "retain" => self.retain = false,
            "dup" => self.dup = false,
            _ => {}
        }
    }

    pub fn field_names() -> Vec<&'static str> {
        vec![
            "type",
            "payload",
            "device_id",
            "message_id",
            "qos",
            "retain",
            "dup",
        ]
    }
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::new();
        // Write message type (1 byte)
        buf.put_u8(self.header.message_type as u8);
        // Write QoS (1 byte)
        buf.put_u8(self.header.qos as u8);
        // Write flags (1 byte)
        let flags = (self.header.retain as u8) | ((self.header.dup as u8) << 1);
        buf.put_u8(flags);
        // Write payload length (4 bytes)
        buf.put_u32(self.payload.len() as u32);
        // Write payload
        buf.extend_from_slice(&self.payload);
        buf.to_vec()
    }
}

// Add serde serialization support for Bytes
mod bytes_serde {
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(bytes: &Bytes, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        Ok(Bytes::from(bytes))
    }
}

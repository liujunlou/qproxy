use qproxy::mqtt_client::{
    message::{
        ConnectMessage, ConnAckMessage, Header, Message, MessageType, PublishMessage, QoS,
        QueryMessage, QueryAckMessage, ReconnectMessage, ReconnectStatus, ReconnectAckMessage,
        DisconnectMessage, PingMessage,
    },
    client::MqttConfig,
};
use bytes::{Bytes, BytesMut};
use std::io::Result;

#[test]
fn test_mqtt_message_header() {
    // 测试消息头编解码
    let header = Header::new(MessageType::Publish);
    let encoded = header.encode();
    let decoded = Header::decode(encoded);
    
    assert_eq!(decoded.message_type, MessageType::Publish);
    assert_eq!(decoded.qos, QoS::AtMostOnce);
    assert_eq!(decoded.dup, false);
    assert_eq!(decoded.retain, false);
    
    // 测试带QoS的消息头
    let mut header = Header::new(MessageType::Publish);
    header.qos = QoS::AtLeastOnce;
    header.dup = true;
    header.retain = true;
    
    let encoded = header.encode();
    let decoded = Header::decode(encoded);
    
    assert_eq!(decoded.message_type, MessageType::Publish);
    assert_eq!(decoded.qos, QoS::AtLeastOnce);
    assert_eq!(decoded.dup, true);
    assert_eq!(decoded.retain, true);
}

#[test]
fn test_mqtt_message_encode_decode() {
    // 创建一个简单的消息
    let header = Header::new(MessageType::Publish);
    let payload = Bytes::from_static(b"test payload");
    let message = Message::new(header, payload.clone());
    
    // 编码消息
    let encoded = message.encode();
    
    // 解码消息
    let decoded = Message::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.header.message_type, MessageType::Publish);
    assert_eq!(decoded.payload, payload);
}

#[test]
fn test_connect_message() {
    // 创建连接消息
    let mut connect = ConnectMessage::new();
    connect.client_id = "test-client".to_string();
    connect.keep_alive = 60;
    connect.app_id = Some("test-app".to_string());
    connect.info = Some("test-info".to_string());
    connect.info_qos = Some(QoS::AtLeastOnce);
    
    // 编码消息
    let encoded = connect.encode();
    
    // 解码消息
    let decoded = ConnectMessage::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.client_id, "test-client");
    assert_eq!(decoded.keep_alive, 60);
    assert_eq!(decoded.app_id, Some("test-app".to_string()));
    assert_eq!(decoded.info, Some("test-info".to_string()));
    assert_eq!(decoded.info_qos, Some(QoS::AtLeastOnce));
}

#[test]
fn test_connack_message() {
    // 创建连接确认消息
    let mut connack = ConnAckMessage::new(0);
    connack.user_id = Some("user-1".to_string());
    connack.session = Some("session-1234".to_string());
    connack.timestamp = Some(1634567890);
    
    // 编码消息
    let encoded = connack.encode();
    
    // 解码消息
    let decoded = ConnAckMessage::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.status, 0);
    assert_eq!(decoded.user_id, Some("user-1".to_string()));
    assert_eq!(decoded.session, Some("session-1234".to_string()));
    assert_eq!(decoded.timestamp, Some(1634567890));
}

#[test]
fn test_publish_message() {
    // 创建发布消息
    let topic = "test/topic".to_string();
    let payload = Bytes::from_static(b"test message");
    let mut publish = PublishMessage::new(topic.clone(), payload.clone());
    publish.packet_id = Some(1234);
    
    // 编码消息
    let encoded = publish.encode();
    
    // 解码消息
    let decoded = PublishMessage::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.topic, topic);
    assert_eq!(decoded.payload, payload);
    assert_eq!(decoded.packet_id, Some(1234));
}

#[test]
fn test_query_message() {
    // 创建查询消息
    let topic = "query/topic".to_string();
    let payload = Bytes::from_static(b"query data");
    let mut query = QueryMessage::new(topic.clone(), payload.clone());
    query.target_id = Some("target-1".to_string());
    
    // 编码消息
    let encoded = query.encode();
    
    // 解码消息
    let decoded = QueryMessage::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.topic, topic);
    assert_eq!(decoded.payload, payload);
    assert_eq!(decoded.target_id, Some("target-1".to_string()));
}

#[test]
fn test_query_ack_message() {
    // 创建查询确认消息
    let message_id = 5678;
    let status = 0;
    let data = Some(Bytes::from_static(b"response data"));
    let query_ack = QueryAckMessage::new(message_id, status, data.clone());
    
    // 编码消息
    let encoded = query_ack.encode();
    
    // 解码消息
    let decoded = QueryAckMessage::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.message_id, message_id);
    assert_eq!(decoded.status, status);
    assert_eq!(decoded.data, data);
}

#[test]
fn test_reconnect_message() {
    // 创建重连消息
    let session = "session-id-1234".to_string();
    let reconnect = ReconnectMessage::new(session.clone());
    
    // 编码消息
    let encoded = reconnect.encode();
    
    // 解码消息
    let decoded = ReconnectMessage::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.session, session);
}

#[test]
fn test_reconnect_ack_message() {
    // 创建重连确认消息
    let recon_ack = ReconnectAckMessage::new(ReconnectStatus::Accepted);
    
    // 编码消息
    let encoded = recon_ack.encode();
    
    // 解码消息
    let decoded = ReconnectAckMessage::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.status as u8, ReconnectStatus::Accepted as u8);
    
    // 测试错误状态
    let recon_ack_error = ReconnectAckMessage::new(ReconnectStatus::Error);
    let encoded_error = recon_ack_error.encode();
    let decoded_error = ReconnectAckMessage::decode(&encoded_error).unwrap();
    assert_eq!(decoded_error.status as u8, ReconnectStatus::Error as u8);
}

#[test]
fn test_disconnect_message() {
    // 创建断开连接消息
    let status = 1;
    let kicked_client_info = Some("client-kicked-by-admin".to_string());
    let disconnect = DisconnectMessage::new(status, kicked_client_info.clone());
    
    // 编码消息
    let encoded = disconnect.encode();
    
    // 解码消息
    let decoded = DisconnectMessage::decode(&encoded).unwrap();
    
    // 验证解码结果
    assert_eq!(decoded.status, status);
    assert_eq!(decoded.kicked_client_info, kicked_client_info);
}

#[test]
fn test_mqtt_config() {
    // 测试MQTT配置
    let config = MqttConfig {
        client_id: "test-client".to_string(),
        host: "mqtt.example.com".to_string(),
        port: 1883,
        keep_alive: 120,
        app_id: Some("test-app".to_string()),
        token: Some("auth-token".to_string()),
        ..Default::default()
    };
    
    assert_eq!(config.client_id, "test-client");
    assert_eq!(config.host, "mqtt.example.com");
    assert_eq!(config.port, 1883);
    assert_eq!(config.keep_alive, 120);
    assert_eq!(config.app_id, Some("test-app".to_string()));
    assert_eq!(config.token, Some("auth-token".to_string()));
}

#[test]
fn test_ping_message() {
    // 创建Ping消息
    let ping = PingMessage;
    
    // 编码消息
    let encoded = ping.encode();
    
    // 验证编码结果（Ping消息没有负载）
    assert_eq!(encoded.len(), 0);
} 
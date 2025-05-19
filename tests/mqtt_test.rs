use qproxy::mqtt_client::{
    message::{
        ConnectMessage, ConnAckMessage, DisconnectMessage, Header, Message, MessageType, PingMessage,
        PublishMessage, QoS, PubAckMessage,
    },
    client::MqttClient
};
use bytes::{Bytes, BytesMut};
use std::time::Duration;
use qproxy::options::Options;
use std::sync::Arc;
use tokio::time::sleep;

#[tokio::test]
async fn test_mqtt_message_construction() {
    // Test creating a connect message
    let mut connect = ConnectMessage::new();
    connect.client_id = "test_client".to_string();
    connect.keep_alive = 60;
    connect.app_id = Some("test_app".to_string());
    connect.token = Some("test_token".to_string());
    
    let encoded = connect.encode();
    assert!(!encoded.is_empty());
    
    // Test creating a publish message
    let payload = Bytes::from("Hello MQTT");
    let topic = "test/topic".to_string();
    let mut publish = PublishMessage::new(topic, payload);
    publish.packet_id = Some(1);
    
    let encoded = publish.encode();
    assert!(!encoded.is_empty());
    
    // Test creating a message with header
    let header = Header {
        message_type: MessageType::Publish,
        dup: false,
        qos: QoS::AtLeastOnce,
        retain: false,
    };
    
    let message = Message::new(header, publish.encode().freeze());
    assert_eq!(message.header.message_type, MessageType::Publish);
    assert_eq!(message.header.qos, QoS::AtLeastOnce);
}

#[tokio::test]
async fn test_mqtt_client_initialization() {
    let client_id = format!("test_client_{}", uuid::Uuid::new_v4());
    let options = Arc::new(Options::default());
    let mut client = MqttClient::new(client_id, "localhost".to_string(), 1883, options);
    client.set_credentials("test_user".to_string(), "test_password".to_string());
    client.set_keep_alive(120);
    client.set_clean_session(true);
    client.set_message_timeout(Duration::from_secs(60));
    client.set_max_retries(5);
    // Since we're not actually connecting to a broker, we just verify the client was initialized
    // with the correct settings
    // No assertions needed - we just make sure the code runs without panicking
} 
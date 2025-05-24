use qproxy::mqtt_client::codec::{
    ConnectMessage, PublishMessage, Message, MessageHeader, MessageType, QoS
};
use bytes::Bytes;
use std::time::Duration;
use qproxy::options::Options;
use std::sync::Arc;
use tokio::time::sleep;

#[tokio::test]
async fn test_mqtt_message_construction() {
    // Test creating a connect message
    let connect = ConnectMessage {
        device_id: "test_client".to_string(),
        clean_session: true,
        keep_alive: 60,
        credentials: None,
        will: None,
        protocol_version: 3,
        protocol_name: "RCloud".to_string(),
    };
    let encoded = serde_json::to_vec(&connect).unwrap();
    assert!(!encoded.is_empty());

    // Test creating a publish message
    let payload = "Hello MQTT".to_string();
    let topic = "test/topic".to_string();
    let mut publish = PublishMessage {
        topic: topic.clone(),
        payload: payload.clone(),
        message_id: Some(1),
        qos: QoS::AtLeastOnce,
        retain: false,
        dup: false,
        properties: None,
    };
    let encoded = serde_json::to_vec(&publish).unwrap();
    assert!(!encoded.is_empty());

    // Test creating a message with header
    let header = MessageHeader {
        message_type: MessageType::Publish,
        dup: false,
        qos: QoS::AtLeastOnce,
        retain: false,
    };
    let message = Message {
        header,
        payload: Bytes::from(encoded.clone()),
    };
    assert_eq!(message.header.message_type, MessageType::Publish);
    assert_eq!(message.header.qos, QoS::AtLeastOnce);
}

#[tokio::test]
async fn test_mqtt_client_initialization() {
    let client_id = format!("test_client_{}", uuid::Uuid::new_v4());
    let options = Arc::new(Options::default());
    let mut client = qproxy::mqtt_client::client::MqttClient::new(client_id, "localhost".to_string(), 1883, options);
    client.set_credentials("test_user".to_string(), "test_password".to_string());
    client.set_keep_alive(120);
    client.set_clean_session(true);
    client.set_message_timeout(Duration::from_secs(60));
    client.set_max_retries(5);
    // Since we're not actually connecting to a broker, we just verify the client was initialized
    // with the correct settings
    // No assertions needed - we just make sure the code runs without panicking
} 
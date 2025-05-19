use qproxy::mqtt_client::{
    message::{QoS, Message},
    client::MqttClient
};
use bytes::Bytes;
use std::time::Duration;
use tokio::time::sleep;
use qproxy::options::Options;
use std::sync::Arc;

// 这是一个集成测试，需要连接到真实的 MQTT 代理
// 默认情况下会被跳过，除非明确指定环境变量 MQTT_BROKER_HOST
#[tokio::test]
#[ignore]
async fn test_mqtt_connect_publish() {
    // 检查是否设置了 MQTT 代理主机环境变量
    let broker_host = match std::env::var("MQTT_BROKER_HOST") {
        Ok(host) => host,
        Err(_) => {
            println!("Skipping MQTT integration test. Set MQTT_BROKER_HOST to run it.");
            return;
        }
    };
    
    let broker_port = std::env::var("MQTT_BROKER_PORT")
        .unwrap_or_else(|_| "1883".to_string())
        .parse::<u16>()
        .unwrap_or(1883);
    
    // 创建 MQTT 客户端
    let client_id = format!("test_client_{}", uuid::Uuid::new_v4());
    let options = Arc::new(Options::default());
    let mut client = MqttClient::new(client_id, broker_host, broker_port, options);
    
    // 可选：设置凭据
    if let (Ok(username), Ok(password)) = (std::env::var("MQTT_USERNAME"), std::env::var("MQTT_PASSWORD")) {
        client.set_credentials(username, password);
    }
    
    // 连接到代理
    if let Err(err) = client.connect().await {
        panic!("Failed to connect to MQTT broker: {}", err);
    }
    
    // 等待连接建立
    sleep(Duration::from_secs(1)).await;
    
    // 发布消息
    let test_topic = "test/mqtt_client";
    let test_payload = Bytes::from("Hello from MQTT integration test");
    
    if let Err(err) = client.publish(test_topic.to_string(), test_payload, QoS::AtMostOnce).await {
        panic!("Failed to publish message: {}", err);
    }
    
    // 等待消息发送
    sleep(Duration::from_secs(1)).await;
    
    // 断开连接
    if let Err(err) = client.disconnect().await {
        panic!("Failed to disconnect from MQTT broker: {}", err);
    }
    
    println!("MQTT integration test completed successfully");
} 
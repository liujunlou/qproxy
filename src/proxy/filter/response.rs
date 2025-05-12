use std::sync::{Arc, RwLock};

use crate::{errors::Error, model::{Protocol, TrafficRecord}, mqtt_client::{codec::MqttCodec, message::{Message, MessageType, PublishMessage, QueryMessage}}};

use super::Filter;

pub struct ResponseFilter {
    mqtt_codec: Arc<RwLock<MqttCodec>>,
    fields: Option<Vec<String>>,
}

impl ResponseFilter {
    pub fn new(fields: Option<Vec<String>>) -> Self {
        Self {
            mqtt_codec: Arc::new(RwLock::new(MqttCodec::new())),
            fields,
        }
    }
}

impl Filter for ResponseFilter {
    // 对定时拉取的流量进行过滤
    fn filter(&self, record: &TrafficRecord) -> Result<TrafficRecord, Error> {
        let mut filtered = record.clone();
        // 在TCP的情况下，解析拉取的body为具体的消息对象，并过滤其中的字段
        if record.protocol == Protocol::TCP {
            // 解析拉取的body为具体的消息对象
            let msg = self.mqtt_codec.write()
                .map_err(|e| Error::MqttDecodeError(e.to_string()))?
                .decode_bytes(&filtered.response.body)
                .map_err(|e| Error::MqttDecodeError(e.to_string()))?;
            
            // 根据消息类型处理
            let updated_payload = match msg.header.message_type {
                MessageType::Publish => {
                    let mut publish_message = PublishMessage::decode(&msg.payload)
                        .map_err(|e| Error::MqttDecodeError(e.to_string()))?;
                    if let Some(fields) = &self.fields {
                        for field in fields {
                            if PublishMessage::field_names().contains(&field.as_str()) {
                                publish_message.remove(field);
                            }
                        }
                    }
                    publish_message.encode().freeze()
                }
                MessageType::Query => {
                    let mut query_message = QueryMessage::decode(&msg.payload)
                        .map_err(|e| Error::MqttDecodeError(e.to_string()))?;
                    if let Some(fields) = &self.fields {
                        for field in fields {
                            if QueryMessage::field_names().contains(&field.as_str()) {
                                query_message.remove(field);
                            }
                        }
                    }
                    query_message.encode().freeze()
                }
                _ => msg.payload.clone(),
            };

            // 创建新的消息并更新 body
            let updated_msg = Message::new(msg.header, updated_payload);
            filtered.response.body = updated_msg.encode().to_vec();
        }
        
        Ok(filtered)
    }
}
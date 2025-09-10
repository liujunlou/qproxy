use bytes::BytesMut;
use prost::Message;
use std::sync::{Arc, RwLock};
use tokio_util::codec::{Decoder, Encoder};

use crate::{
    errors::Error,
    model::{Protocol, TrafficRecord},
    mqtt_client::codec::{
        MqttCodec, MqttMessage, PublishMessage, QueryMessage,
    }, playback::route::RouteMessage,
};

use super::Filter;

pub struct ResponseFilter {
    mqtt_codec: Arc<RwLock<MqttCodec>>,
    fields: Option<Vec<String>>,
    skip_topics: Option<Vec<String>>,
}

impl ResponseFilter {
    pub fn new(fields: Option<Vec<String>>, skip_topics: Option<Vec<String>>) -> Self {
        Self {
            mqtt_codec: Arc::new(RwLock::new(MqttCodec::new(1024 * 1024))),
            fields,
            skip_topics
        }
    }

    pub fn set_fields(&mut self, fields: Vec<String>) {
        self.fields = Some(fields);
    }
}

impl Filter for ResponseFilter {
    // 对定时拉取的流量进行过滤
    fn filter(&self, record: &TrafficRecord) -> Result<TrafficRecord, Error> {
        let mut filtered = record.clone();
        // 在TCP的情况下，解析拉取的body为具体的消息对象，并过滤其中的字段
        if record.protocol == Protocol::TCP {
            // 将 Vec<u8> 转换为 BytesMut
            let mut buf = BytesMut::from(&filtered.response.body[..]);

            // 解析拉取的body为具体的消息对象
            let msg = self
                .mqtt_codec
                .write()
                .map_err(|e| Error::MqttDecodeError(e.to_string()))?
                .decode_eof(&mut buf)
                .map_err(|e| Error::MqttDecodeError(e.to_string()))?
                .ok_or_else(|| Error::MqttDecodeError("Failed to decode message".to_string()))?;

            // 根据消息类型处理
            let updated_msg = match msg {
                MqttMessage::Publish(publish_msg) => {
                    let mut publish_message = publish_msg;
                    if let Some(fields) = &self.fields {
                        for field in fields {
                            if PublishMessage::field_names().contains(&field.as_str()) {
                                publish_message.remove(field);
                            }
                        }
                    }
                    MqttMessage::Publish(publish_message)
                }
                MqttMessage::Query(query_msg) => {
                    let mut query_message = query_msg;
                    if let Some(fields) = &self.fields {
                        for field in fields {
                            if QueryMessage::field_names().contains(&field.as_str()) {
                                query_message.remove(field);
                            }
                        }
                    }
                    MqttMessage::Query(query_message)
                }
                _ => msg,
            };

            // 更新 body
            let mut buf = BytesMut::new();
            self.mqtt_codec
                .write()
                .map_err(|e| Error::MqttDecodeError(e.to_string()))?
                .encode(updated_msg, &mut buf)
                .map_err(|e| Error::MqttDecodeError(e.to_string()))?;
            filtered.response.body = buf.to_vec();
        } else if record.protocol == Protocol::GRPC {
            let mut route_message = RouteMessage::decode(&mut &record.request.body[..])
                .map_err(|_| {
                    Error::GrpcStatus(format!(
                        "Failed to decode RouteMessage {:?}",
                        record.request.body
                    ))
                })?;
            // 过滤路由消息中的字段
            if let Some(skip_topics) = &self.skip_topics {
                if let Some(method) = route_message.method.as_ref() {
                    if skip_topics.contains(method) {
                        return Ok(filtered);
                    }
                }
            }
            if let Some(fields) = &self.fields {
                for field in fields {
                    if RouteMessage::field_names().contains(&field.as_str()) {
                        route_message.remove(field);
                    }
                }
            }
            // 重新编码过滤后的消息
            let mut buf = BytesMut::new();
            route_message.encode(&mut buf).map_err(|e| Error::GrpcStatus(e.to_string()))?;
            filtered.response.body = buf.to_vec();
        } else { // http 协议不过滤
            return Ok(filtered);
        }

        Ok(filtered)
    }
}

impl RouteMessage {
    pub fn field_names() -> Vec<&'static str> {
        vec![
            "message_id",
            "method", 
            "target_id",
            "app_id",
            "log_id",
            "from_gateway",
            "payload",
            "metadata",
        ]
    }

    pub fn remove(&mut self, field: &str) {
        match field {
            "message_id" => self.message_id = None,
            "method" => self.method = None,
            "target_id" => self.target_id = None,
            "app_id" => self.app_id = None,
            "log_id" => self.log_id = None,
            "from_gateway" => self.from_gateway = None,
            "payload" => self.payload = None,
            "metadata" => self.metadata.clear(),
            _ => {}
        }
    }
}

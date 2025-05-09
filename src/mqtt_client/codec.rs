use super::crypto::{self, CryptoInfo};
use super::message::{Message, MessageType};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::{self, Cursor};
use std::sync::{Arc, Mutex};
use tokio_util::codec::{Decoder, Encoder};

// 类似于Java中的MqttMessageEncoder/Decoder
pub struct MqttCodec {
    secret_key: Arc<Mutex<Option<[u8; crypto::KEY_LEN]>>>,
    crypto_info: Arc<Mutex<Option<CryptoInfo>>>,
}

impl MqttCodec {
    pub fn new() -> Self {
        Self {
            secret_key: Arc::new(Mutex::new(None)),
            crypto_info: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_secret_key(&self, key: [u8; crypto::KEY_LEN]) {
        if let Ok(mut secret_key) = self.secret_key.lock() {
            *secret_key = Some(key);
        }
    }

    pub async fn set_crypto_info(&self, info: CryptoInfo) {
        if let Ok(mut crypto_info) = self.crypto_info.lock() {
            *crypto_info = Some(info);
        }
    }
}

impl Decoder for MqttCodec {
    type Item = Message;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 2 {
            // 需要至少两个字节来解析类型和长度
            return Ok(None);
        }

        // 标记当前位置以便需要时重置
        let mut cursor = Cursor::new(&src[..]);
        
        // 读取第一个字节，它包含消息类型和标志
        let first_byte = cursor.get_u8();
        
        // 计算消息长度
        let mut multiplier = 1;
        let mut remaining_length = 0;
        let mut length_size = 0;
        
        loop {
            if cursor.position() as usize >= src.len() {
                // 没有足够的字节来确定长度
                return Ok(None);
            }
            
            length_size += 1;
            let digit = cursor.get_u8();
            remaining_length += (digit & 0x7F) as usize * multiplier;
            multiplier *= 128;
            
            if multiplier > 128 * 128 * 128 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Malformed remaining length"));
            }
            
            if (digit & 0x80) == 0 {
                break;
            }
        }
        
        let total_length = 1 + length_size + remaining_length;
        
        if src.len() < total_length {
            // 消息不完整，等待更多数据
            return Ok(None);
        }
        
        // 提取完整消息
        let mut data = src.split_to(total_length).freeze();
        
        // 确定起始偏移量，与Java代码一致
        let start_offset = 1 + length_size;
        
        // 获取消息类型
        let msg_type = (first_byte >> 4) & 0x0F;
        let message_type = match msg_type {
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
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid message type")),
        };
        
        // 创建可变数据副本以进行解密
        let mut data_vec = data.to_vec();
        
        // 根据消息类型应用不同的解密策略
        match message_type {
            MessageType::Ping | MessageType::Pong => {
                // 为Ping/Pong消息应用XOR混淆
                if let Ok(secret_key_guard) = self.secret_key.lock() {
                    if let Some(key) = &*secret_key_guard {
                        crypto::obfuscation(&mut data_vec, key, start_offset);
                    }
                }
            }
            MessageType::Connect => {
                // 从Connect消息中提取密钥
                let secret_key = crypto::get_secret_key(&data_vec, start_offset);
                if let Ok(mut secret_key_guard) = self.secret_key.lock() {
                    *secret_key_guard = Some(secret_key);
                }
            }
            MessageType::Reserve1 | MessageType::Reserve2 => {
                // 不做任何处理
            }
            _ => {
                // 对其他所有消息类型
                let mut has_crypto = false;
                if let Ok(crypto_info_guard) = self.crypto_info.lock() {
                    has_crypto = crypto_info_guard.is_some();
                }
                
                if has_crypto {
                    // 使用AES解密
                    let crypto_key = if let Ok(crypto_info_guard) = self.crypto_info.lock() {
                        crypto_info_guard.as_ref().map(|info| info.key.clone())
                    } else {
                        None
                    };
                    
                    if let Some(key) = crypto_key {
                        let mut payload = Vec::new();
                        payload.extend_from_slice(&data_vec[start_offset..]);
                        
                        match crypto::aes_decode(&key, &payload) {
                            Ok(decoded) => {
                                // 替换原始数据中的有效载荷
                                let mut new_data = Vec::with_capacity(start_offset + decoded.len());
                                new_data.extend_from_slice(&data_vec[..start_offset]);
                                new_data.extend_from_slice(&decoded);
                                data_vec = new_data;
                            }
                            Err(e) => {
                                return Err(io::Error::new(io::ErrorKind::InvalidData, 
                                    format!("AES decryption error: {}", e)));
                            }
                        }
                    }
                } else {
                    // 使用XOR混淆
                    if let Ok(secret_key_guard) = self.secret_key.lock() {
                        if let Some(key) = &*secret_key_guard {
                            crypto::obfuscation(&mut data_vec, key, start_offset);
                        }
                    }
                }
            }
        }
        
        // 将修改后的数据转换回Bytes
        data = Bytes::from(data_vec);
        
        // 尝试解析消息
        match Message::decode(&data) {
            Ok(message) => Ok(Some(message)),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, 
                format!("Failed to decode message: {}", e))),
        }
    }
}

impl Encoder<Message> for MqttCodec {
    type Error = io::Error;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        // 首先编码消息
        let mut data = item.encode();
        let data_vec = data.to_vec();
        
        // 确定起始偏移量
        let header_len = 1; // 固定头部1字节
        let len = item.payload.len();
        let mut length_size = 0;
        
        // 计算长度字段大小
        let mut temp = len;
        loop {
            length_size += 1;
            temp /= 128;
            if temp == 0 {
                break;
            }
        }
        
        let start_offset = header_len + length_size;
        
        // 根据消息类型应用不同的加密策略
        match item.header.message_type {
            MessageType::Ping | MessageType::Pong => {
                // 为Ping/Pong消息应用XOR混淆
                if let Ok(secret_key_guard) = self.secret_key.lock() {
                    if let Some(key) = &*secret_key_guard {
                        let mut data_mut = data.to_vec();
                        crypto::obfuscation(&mut data_mut, key, start_offset);
                        data = BytesMut::from(&data_mut[..]);
                    }
                }
            }
            MessageType::Reserve1 | MessageType::Reserve2 => {
                // 不做任何处理
            }
            _ => {
                // 对其他所有消息类型
                let mut has_crypto = false;
                if let Ok(crypto_info_guard) = self.crypto_info.lock() {
                    has_crypto = crypto_info_guard.is_some();
                }
                
                if has_crypto {
                    // 使用AES加密
                    let crypto_key = if let Ok(crypto_info_guard) = self.crypto_info.lock() {
                        crypto_info_guard.as_ref().map(|info| info.key.clone())
                    } else {
                        None
                    };
                    
                    if let Some(key) = crypto_key {
                        let payload = &data_vec[start_offset..];
                        
                        match crypto::aes_encode(&key, payload) {
                            Ok(encoded) => {
                                // 重建消息结构
                                let mut output = BytesMut::new();
                                
                                // 写入原始类型字节
                                let code = data_vec[0];
                                output.put_u8(code);
                                
                                // 初始化校验码占位符
                                output.put_u8(0);
                                
                                // 写入加密后数据的长度
                                let mut len = encoded.len();
                                let mut digest = 0;
                                
                                loop {
                                    let b = (len & 0x7F) as u8;
                                    len >>= 7;
                                    
                                    digest ^= b;
                                    
                                    if len > 0 {
                                        output.put_u8(b | 0x80);
                                    } else {
                                        output.put_u8(b);
                                        break;
                                    }
                                }
                                
                                // 写入加密后的数据
                                output.extend_from_slice(&encoded);
                                
                                // 更新校验码
                                let bytes = output.to_vec();
                                data = BytesMut::from(&bytes[..]);
                                data[1] = digest as u8;
                            }
                            Err(e) => {
                                return Err(io::Error::new(io::ErrorKind::InvalidData, 
                                    format!("AES encryption error: {}", e)));
                            }
                        }
                    }
                } else {
                    // 使用XOR混淆
                    if let Ok(secret_key_guard) = self.secret_key.lock() {
                        if let Some(key) = &*secret_key_guard {
                            let mut data_mut = data.to_vec();
                            crypto::obfuscation(&mut data_mut, key, start_offset);
                            data = BytesMut::from(&data_mut[..]);
                        }
                    }
                }
            }
        }
        
        // 写入最终数据
        dst.extend_from_slice(&data);
        
        Ok(())
    }
} 
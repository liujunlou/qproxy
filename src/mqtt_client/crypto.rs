use aes_gcm::{
    aead::{generic_array::GenericArray, Aead, KeyInit},
    Aes128Gcm, Nonce
};

// 定义与Java相同的常量
pub const RCLOUD_KEY: [u8; 8] = [0, 6, 82, 67, 108, 111, 117, 100]; // "RCloud"
pub const KEY_LEN: usize = 8;

// 实现与Java FormatUtil.obfuscation相同的混淆函数
pub fn obfuscation(data: &mut [u8], keys: &[u8], start: usize) {
    let data_len = data.len();
    
    for i in (start..data_len).step_by(KEY_LEN) {
        for (j, b) in (i..std::cmp::min(i + KEY_LEN, data_len)).enumerate() {
            data[b] = data[b] ^ keys[j % keys.len()];
        }
    }
}

// 实现与Java FormatUtil.getSecretKey相同的函数，用于从协议ID获取密钥
pub fn get_secret_key(data: &[u8], payload_start: usize) -> [u8; KEY_LEN] {
    let mut result = [0u8; KEY_LEN];
    
    for (i, j) in (payload_start..(payload_start + KEY_LEN)).zip(0..KEY_LEN) {
        if i < data.len() {
            result[j] = data[i] ^ RCLOUD_KEY[j];
        }
    }
    
    result
}

// 实现与Java AESUtils.encode相同的AES加密
pub fn aes_encode(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    // 确保密钥长度为16字节 (128位)
    let key_bytes = if key.len() >= 16 {
        &key[0..16]
    } else {
        return Err("Key must be at least 16 bytes".to_string());
    };
    
    // 创建密钥和随机数
    let key = GenericArray::from_slice(key_bytes);
    let cipher = Aes128Gcm::new(key);
    
    // 使用全零随机数 (不安全，仅为兼容性)
    let nonce = Nonce::from_slice(&[0u8; 12]);
    
    // 加密数据
    cipher.encrypt(nonce, data)
        .map_err(|e| format!("AES encryption error: {}", e))
}

// 实现与Java AESUtils.decode相同的AES解密
pub fn aes_decode(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    // 确保密钥长度为16字节 (128位)
    let key_bytes = if key.len() >= 16 {
        &key[0..16]
    } else {
        return Err("Key must be at least 16 bytes".to_string());
    };
    
    // 创建密钥和随机数
    let key = GenericArray::from_slice(key_bytes);
    let cipher = Aes128Gcm::new(key);
    
    // 使用全零随机数 (不安全，仅为兼容性)
    let nonce = Nonce::from_slice(&[0u8; 12]);
    
    // 解密数据
    cipher.decrypt(nonce, data)
        .map_err(|e| format!("AES decryption error: {}", e))
}

// 定义一个加密信息结构体，对应Java中的CryptoInfo
#[derive(Debug, Clone)]
pub struct CryptoInfo {
    pub key: Vec<u8>,
    pub algorithm: String,
    pub mode: String,
    pub padding: String,
}

impl CryptoInfo {
    pub fn new(key: Vec<u8>) -> Self {
        Self {
            key,
            algorithm: "AES".to_string(),
            mode: "ECB".to_string(),
            padding: "PKCS5".to_string(),
        }
    }
} 
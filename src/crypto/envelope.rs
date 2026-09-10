//! NetSecKit 信封加密（iOS 7.3.40 请求链）。
//!
//! - keyDataOne = "com.wanhang.school" 末 12 字符
//! - keyDataTwo = "5K0E8400-E29B-11D4-A716-4G6RW65G544F" 前 12 字符
//! - keyDataThree = "DE6AED50-2B3A-5327-ACED-597DEA1AFB49" 末 12 字符
//! - keyDataFour = format!("{:.6}", now_ms_f64) 的末 12 字符（native %f 语义，
//!   singleton 首次生成后缓存——同一会话内所有请求复用同一个 Four）
//! - inner container 固定键序 data,timeStamp,platform,keyDataOne..Four
//! - d=AES-128-CBC/PKCS7/IV=0(reqKey)；h=MD5(container)；k=RSA-1024 PKCS1v15(reqKey)
//! - outer 键序：observed=[k,p,d,h,t]（headerSign 用）；insert=[d,h,k,p,t]（body 用）

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cbc::cipher::{block_padding::Pkcs7, BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use md5::{Digest, Md5};
use rand::Rng;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

pub const KEYDATA_ONE: &str = "nhang.school";
pub const KEYDATA_TWO: &str = "5K0E8400-E29";
pub const KEYDATA_THREE: &str = "597DEA1AFB49";

/// native reqKey 白名单字符池。
pub const SALT_ALPHABET: &str = "+kot8A*B45jF6CD@a!UVWubcdKLZ{efgMpNOxyz01PQ}Rn)Tvw23XYh(iG7rsEqJHI9+Slm/";

/// 客户端内置 RSA-1024 公钥（SPKI PEM）。
pub const RSA_PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC5pqlTzsGNZk1RxhH4O4x3JNTD\n\
V7FbVH66mPfW5v1tnIy4ty7xv8DGMG4Zn/TvstwlJWeYOADHdi8uF21lJaBvzvPt\n\
VEhifHXZq825fI9hGYtDoaVQmCN/Nfs2dKmt89XDrhtl3SZxO6TumOCTQt+5oqjF\n\
2Jo3o1YtkAyGzjaJnwIDAQAB\n\
-----END PUBLIC KEY-----";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OuterOrder {
    /// [k,p,d,h,t] —— 真实 headerSign 样本顺序
    Observed,
    /// [d,h,k,p,t] —— body 用插入序
    Insert,
}

/// 缓存 RSA 公钥（进程内只解析一次）。
pub fn rsa_public_key() -> rsa::RsaPublicKey {
    use rsa::pkcs8::DecodePublicKey;
    rsa::RsaPublicKey::from_public_key_pem(RSA_PUB_PEM).expect("内置 RSA 公钥解析失败")
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn md5_hex(data: &[u8]) -> String {
    let mut h = Md5::new();
    h.update(data);
    let out = h.finalize();
    let mut s = String::with_capacity(32);
    for b in out {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

pub fn b64_encode(data: &[u8]) -> String {
    B64.encode(data)
}

pub fn b64_decode(data: &str) -> Result<Vec<u8>, String> {
    B64.decode(data).map_err(|e| format!("base64 解码失败: {e}"))
}

/// AES-128-CBC / PKCS7 / IV=0 加密（请求 d 与响应两层解密同参数）。
pub fn aes128_cbc_encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let key: &[u8; 16] = key
        .try_into()
        .map_err(|_| format!("AES key 必须是 16B，得到 {}", key.len()))?;
    let iv = [0u8; 16];
    Ok(Aes128CbcEnc::new(key.into(), &iv.into()).encrypt_padded_vec::<Pkcs7>(plaintext))
}

/// AES-128-CBC / PKCS7 / IV=0 解密。
pub fn aes128_cbc_decrypt(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let key: &[u8; 16] = key
        .try_into()
        .map_err(|_| format!("AES key 必须是 16B，得到 {}", key.len()))?;
    let iv = [0u8; 16];
    Aes128CbcDec::new(key.into(), &iv.into())
        .decrypt_padded_vec::<Pkcs7>(ciphertext)
        .map_err(|e| format!("AES 解密/PKCS7 失败: {e}"))
}

/// AES-128-CBC / PKCS7 / 自定义 IV 加密（GT4 get_w 用 ASCII "0000"x4 IV）。
pub fn aes128_cbc_encrypt_iv(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let key: &[u8; 16] = key
        .try_into()
        .map_err(|_| format!("AES key 必须是 16B，得到 {}", key.len()))?;
    let iv: &[u8; 16] = iv
        .try_into()
        .map_err(|_| format!("AES IV 必须是 16B，得到 {}", iv.len()))?;
    Ok(Aes128CbcEnc::new(key.into(), iv.into()).encrypt_padded_vec::<Pkcs7>(plaintext))
}

/// 从 SALT_ALPHABET 独立抽取 n 个字符（native reqKey 语义）。
pub fn random_fragment(n: usize) -> String {
    let alphabet = SALT_ALPHABET.as_bytes();
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..n)
        .map(|_| alphabet[rng.gen_range(0..alphabet.len())])
        .collect();
    String::from_utf8(bytes).unwrap_or_default()
}

pub fn req_key() -> String {
    random_fragment(16)
}

/// normalize_raw_key_data：≤7B 补随机字母数字到 12；≥13B 取 UTF-8 末 12；8-12 原样。
pub fn normalize_raw_key_data(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() <= 7 {
        let missing = 12 - bytes.len();
        let mut out = value.to_string();
        out.push_str(&random_fragment(missing));
        out
    } else if bytes.len() >= 13 {
        String::from_utf8_lossy(&bytes[bytes.len() - 12..]).into_owned()
    } else {
        value.to_string()
    }
}

/// keyDataFour：native `stringWithFormat:"%f"`（毫秒 double → 6 位小数）取末 12 字符。
pub fn key_data_four_from_ms(now_ms_f64: f64) -> String {
    let formatted = format!("{:.6}", now_ms_f64);
    normalize_raw_key_data(&formatted)
}

/// NTESSecurityKit singleton 会话：Four 首次生成后整个会话复用。
pub struct EnvelopeSession {
    four: Option<String>,
}

impl Default for EnvelopeSession {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvelopeSession {
    pub fn new() -> Self {
        Self { four: None }
    }

        pub fn key_data(&mut self) -> [String; 4] {
        if self.four.is_none() {
            self.four = Some(key_data_four_from_ms(now_ms() as f64));
        }
        [
            KEYDATA_ONE.to_string(),
            KEYDATA_TWO.to_string(),
            KEYDATA_THREE.to_string(),
            self.four.clone().unwrap(),
        ]
    }

    /// 测试注入固定 Four。
    #[allow(dead_code)]
    pub fn with_four(four: &str) -> Self {
        Self { four: Some(four.to_string()) }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BuiltEnvelope {
    /// 外层信封 JSON（按指定键序）。
    pub json: String,
    /// inner container 原文字节（h 的输入）。
    pub container: Vec<u8>,
    pub req_key: String,
    pub ts_ms: i64,
    pub key_data: [String; 4],
}

pub fn build_envelope(session: &mut EnvelopeSession, plain: &str, order: OuterOrder) -> BuiltEnvelope {
    build_envelope_ts(session, plain, order, now_ms())
}

pub fn build_envelope_ts(
    session: &mut EnvelopeSession,
    plain: &str,
    order: OuterOrder,
    ts_ms: i64,
) -> BuiltEnvelope {
    let key_data = session.key_data();
    let container_data = B64.encode(plain.as_bytes());
    let container = serialize_container(&container_data, ts_ms, &key_data);
    let rk = req_key();
    let d = B64.encode(aes128_cbc_encrypt(rk.as_bytes(), &container).unwrap_or_default());
    let h = md5_hex(&container);
    let k = {
        let pk = rsa_public_key();
        let mut rng = rand::thread_rng();
        let ct = pk
            .encrypt(&mut rng, rsa::Pkcs1v15Encrypt, rk.as_bytes())
            .unwrap_or_default();
        B64.encode(ct)
    };
    let json = serialize_envelope_fields(&d, &h, &k, order);
    BuiltEnvelope { json, container, req_key: rk, ts_ms, key_data }
}

/// inner container：固定键序紧凑 JSON（UTF-8 字节）。
pub fn serialize_container(data_b64: &str, ts_ms: i64, key_data: &[String; 4]) -> Vec<u8> {
    let mut m = serde_json::Map::new();
    m.insert("data".into(), serde_json::Value::String(data_b64.to_string()));
    m.insert("timeStamp".into(), serde_json::Value::from(ts_ms));
    m.insert("platform".into(), serde_json::Value::from(1));
    m.insert("keyDataOne".into(), serde_json::Value::String(key_data[0].clone()));
    m.insert("keyDataTwo".into(), serde_json::Value::String(key_data[1].clone()));
    m.insert("keyDataThree".into(), serde_json::Value::String(key_data[2].clone()));
    m.insert("keyDataFour".into(), serde_json::Value::String(key_data[3].clone()));
    serde_json::to_vec(&serde_json::Value::Object(m)).unwrap_or_default()
}

/// 外层信封序列化（手工拼接保证键序，字符串值均为无需转义的 base64/hex）。
pub fn serialize_envelope_fields(d: &str, h: &str, k: &str, order: OuterOrder) -> String {
    match order {
        // observed: k,p,d,h,t（真实 headerSign 样本序）
        OuterOrder::Observed => format!("{{\"k\":\"{k}\",\"p\":101,\"d\":\"{d}\",\"h\":\"{h}\",\"t\":0}}"),
        // insert: d,h,k,p,t（body 插入序）
        OuterOrder::Insert => format!("{{\"d\":\"{d}\",\"h\":\"{h}\",\"k\":\"{k}\",\"p\":101,\"t\":0}}"),
    }
}

/// 本地验证一个信封（不需要 RSA 私钥；生产未用，测试/诊断用）：
/// AES 回解 == container、h == MD5(container)、k 长度 128B。
#[allow(dead_code)]
pub fn validate_local_envelope(env: &BuiltEnvelope) -> Result<(), String> {
    if env.req_key.len() != 16 {
        return Err("reqKey UTF-8 长度不是 16B".into());
    }
    let v: serde_json::Value =
        serde_json::from_str(&env.json).map_err(|e| format!("信封 JSON 解析失败: {e}"))?;
    let d = v["d"].as_str().ok_or("信封缺 d")?;
    let h = v["h"].as_str().ok_or("信封缺 h")?;
    let k = v["k"].as_str().ok_or("信封缺 k")?;
    let plain = aes128_cbc_decrypt(env.req_key.as_bytes(), &b64_decode(d)?)?;
    if plain != env.container {
        return Err("d AES 解密结果与 inner container 不一致".into());
    }
    if h != md5_hex(&env.container) {
        return Err("h != MD5(inner container)".into());
    }
    if b64_decode(k)?.len() != 128 {
        return Err("k 解码长度 != 128B (RSA-1024)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 实测向量。
    #[test]
    fn test_md5_vector() {
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    /// 实测向量（AES-128-CBC/IV=0/PKCS7）。
    #[test]
    fn test_aes_cbc_vector() {
        let key = b"0123456789abcdef";
        let pt = b"hello world, this is a test!!";
        let ct = aes128_cbc_encrypt(key, pt).unwrap();
        assert_eq!(
            ct.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "c4cf3b785dd429f0d80254ad853d92e1295bc6969a5f029dd83b939da9566c78"
        );
        let back = aes128_cbc_decrypt(key, &ct).unwrap();
        assert_eq!(back, pt);
    }

    /// keyDataFour：native %f 语义（毫秒 double 6 位小数取末 12 字符）。
    #[test]
    fn test_key_data_four() {
        assert_eq!(key_data_four_from_ms(1_788_958_186_123.456_8), "86123.456787");
        // 长度规则
        let four = key_data_four_from_ms(1_788_958_186_123.0);
        assert_eq!(four.len(), 12);
        assert!(four.ends_with(".000000"));
    }

    /// normalize_raw_key_data 长度规则。
    #[test]
    fn test_normalize_raw_key_data() {
        assert_eq!(normalize_raw_key_data("nhang.school"), "nhang.school");
        assert_eq!(normalize_raw_key_data("5K0E8400-E29B-11D4"), "00-E29B-11D4");
        let short = normalize_raw_key_data("abc");
        assert_eq!(short.len(), 12);
        assert!(short.starts_with("abc"));
    }

    /// 完整信封构造→反解 roundtrip（本地验证，无需网络/私钥）。
    #[test]
    fn test_envelope_roundtrip() {
        let mut session = EnvelopeSession::with_four("86123.456789");
        let plain = r#"{"runMode":1,"ruleUpdateTime":0}"#;
        let env = build_envelope(&mut session, plain, OuterOrder::Insert);
        validate_local_envelope(&env).expect("本地信封校验失败");
        assert_eq!(env.key_data[0], KEYDATA_ONE);
        assert_eq!(env.key_data[1], KEYDATA_TWO);
        assert_eq!(env.key_data[2], KEYDATA_THREE);
        // observed 键序
        let env2 = build_envelope(&mut session, plain, OuterOrder::Observed);
        assert!(env2.json.starts_with("{\"k\":\""));
        assert!(env2.json.ends_with(",\"p\":101") || env2.json.contains("\",\"p\":101,\"d\":\""));
        // insert 键序
        assert!(env.json.starts_with("{\"d\":\""));
        // 同一会话 Four 复用
        assert_eq!(env.key_data[3], env2.key_data[3]);
    }

    /// reqKey 白名单与长度。
    #[test]
    fn test_req_key() {
        for _ in 0..20 {
            let rk = req_key();
            assert_eq!(rk.len(), 16);
            assert!(rk.chars().all(|c| SALT_ALPHABET.contains(c)));
        }
    }

    /// RSA 公钥解析 + 加密长度 = 128B（RSA-1024 块）。
    #[test]
    fn test_rsa_encrypt_len() {
        let pk = rsa_public_key();
        let mut rng = rand::thread_rng();
        let ct = pk.encrypt(&mut rng, rsa::Pkcs1v15Encrypt, b"0123456789abcdef").unwrap();
        assert_eq!(ct.len(), 128);
    }
}

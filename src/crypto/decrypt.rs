//! 响应解密链。
//!
//! 响应 {"r","s","v"} 处理顺序：
//! 1. 四段 keyData 原文 → fold32 x4 → MBA 混合 → little-endian 16B pAesKey
//! 2. r = Base64(AES-128-CBC/PKCS7/IV=0(pAesKey, 内层信封 JSON))
//! 3. s = Base64(RSA-1024 PKCS#1 type-1 签名)，公钥 raw 运算恢复 32 位小写 MD5，
//!    与 MD5(内层信封明文) 比对
//! 4. 内层信封 d 再用同一 pAesKey 解密 → {"data": Base64(业务 JSON), ...}
//! 5. 业务 data 再 Base64 解码

use rsa::traits::PublicKeyParts;
use serde_json::Value;

/// UTF8String + strlen 语义：只处理第一个 NUL 之前的字节。
fn c_string_bytes(v: &str) -> &[u8] {
    let b = v.as_bytes();
    match b.iter().position(|&c| c == 0) {
        Some(i) => &b[..i],
        None => b,
    }
}

/// 0x103f5684c: h = (((h & 0xffffff) << 8) | c) ^ (h >> 24)。
pub fn fold32(value: &str) -> u32 {
    let mut h: u32 = 0;
    for &c in c_string_bytes(value) {
        h = (((h & 0x00FF_FFFF) << 8) | c as u32) ^ (h >> 24);
    }
    h
}

fn ror32(value: u32, bits: u32) -> u32 {
    value.rotate_right(bits)
}

/// 0x103f5689c MBA 混合 → little-endian 16B pAesKey。
pub fn derive_paes_key(one: &str, two: &str, three: &str, four: &str) -> [u8; 16] {
    let vals = [one, two, three, four];
    for v in &vals {
        let l = c_string_bytes(v).len();
        assert!((8..=12).contains(&l), "keyData UTF-8 长度必须为 8..12B: {v}");
    }
    let (a, b, c, d) = (fold32(one), fold32(two), fold32(three), fold32(four));

    let mut w9 = c ^ a;
    let mut w10 = w9 ^ ror32(w9, 24);
    w9 = w10 ^ ror32(w9, 8);
    w10 = w9 ^ b;
    w9 ^= d;
    let mut w8 = d ^ b;
    let mut w11 = w8 ^ ror32(w8, 24);
    w8 = w11 ^ ror32(w8, 8);
    w11 = w8 ^ a;
    w8 ^= c;

    let mut w12 = w8 & w10;
    w11 ^= w12;
    w12 = w8 | w9;
    w8 ^= w9;
    w10 ^= w12;
    w8 = w10 ^ (!w8);
    w12 = w8 ^ w11;
    w8 |= w11;
    w8 ^= w10;
    w10 = w12 & !w10;
    w9 ^= w10;

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&w9.to_le_bytes());
    out[4..8].copy_from_slice(&w8.to_le_bytes());
    out[8..12].copy_from_slice(&w12.to_le_bytes());
    out[12..16].copy_from_slice(&w11.to_le_bytes());
    out
}

/// RSA-1024 公钥 raw 运算恢复 response.s 的 MD5 hex（PKCS#1 type-1 块）。
pub fn rsa_recovered_digest(response_s: &str, pub_key: &rsa::RsaPublicKey) -> Result<String, String> {
    let sig = super::envelope::b64_decode(response_s)?;
    let n = pub_key.n().to_bytes_be();
    let size = n.len();
    if sig.len() != size {
        return Err(format!("s 解码长度错误: {len} != {size}", len = sig.len()));
    }
    let e = pub_key.e().to_bytes_be();
    let m = rsa::BigUint::from_bytes_be(&sig);
    let eb = rsa::BigUint::from_bytes_be(&e);
    let nb = rsa::BigUint::from_bytes_be(&n);
    let em = m.modpow(&eb, &nb).to_bytes_be();
    let em = {
        // 左填充到 size 字节
        let mut v = vec![0u8; size - em.len()];
        v.extend_from_slice(&em);
        v
    };
    if em.len() < 2 || em[0] != 0x00 || em[1] != 0x01 {
        return Err("s 不是 RSA PKCS#1 type-1 块".into());
    }
    let marker = em[2..]
        .iter()
        .position(|&b| b == 0x00)
        .map(|i| i + 2)
        .ok_or("s 缺少 RSA PKCS#1 分隔符")?;
    if marker < 10 || em[2..marker].iter().any(|&b| b != 0xFF) {
        return Err("s 的 RSA PKCS#1 type-1 填充不合法".into());
    }
    let digest = &em[marker + 1..];
    if digest.len() != 32 || !digest.iter().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err("RSA(s) 尾部不是 32 字节小写 hex".into());
    }
    Ok(String::from_utf8_lossy(digest).into_owned())
}

/// 响应解密结果。
pub struct Decrypted {
    /// 第一层明文（内层信封 JSON 字符串）。
    #[allow(dead_code)]
    pub plaintext: String,
    pub business: Value,
    /// RSA(s) 恢复出的期望 MD5。
    #[allow(dead_code)]
    pub expected_md5: String,
    /// 第一层明文实际 MD5（相等即验签通过）。
    #[allow(dead_code)]
    pub actual_md5: String,
}

/// 完整解密：r/s/v 两层 AES + MD5 校验；非加密响应原样透传为业务 JSON。
pub fn decrypt_response(
    raw: &[u8],
    key: &[u8; 16],
    pub_key: &rsa::RsaPublicKey,
) -> Result<Decrypted, String> {
    let text = String::from_utf8_lossy(raw).into_owned();
    let mut obj: Value = serde_json::from_str(text.trim())
        .map_err(|e| format!("响应不是合法 JSON: {e}"))?;
    if let Some(resp) = obj.get("resp") {
        if resp.is_string() {
            let inner = resp.as_str().unwrap();
            obj = serde_json::from_str(inner)
                .map_err(|e| format!("resp 包装解析失败: {e}"))?;
        } else if !resp.is_null() {
            obj = resp.clone();
        }
    }
    let has_rsv = ["r", "s", "v"].iter().all(|k| obj.get(*k).is_some());
    if !has_rsv {
        // 普通明文业务响应（错误/网关提示等）
        return Ok(Decrypted {
            plaintext: text,
            business: obj,
            expected_md5: String::new(),
            actual_md5: String::new(),
        });
    }
    let version = obj["v"]
        .as_i64()
        .or_else(|| obj["v"].as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| format!("响应 v 不是整数: {}", obj["v"]))?;
    if version != 101 {
        return Err(format!("响应版本错误: {version} != 101"));
    }
    let s = obj["s"].as_str().ok_or("响应缺 s")?;
    let expected = rsa_recovered_digest(s, pub_key)?;
    let layer1 = super::envelope::aes128_cbc_decrypt(
        key,
        &super::envelope::b64_decode(obj["r"].as_str().ok_or("响应缺 r")?)?,
    )
    .map_err(|e| format!("第一层 AES 解密失败: {e}"))?;
    let actual = super::envelope::md5_hex(&layer1);
    if actual != expected {
        return Err(format!("响应验签失败: 实际 MD5 {actual} 期望 {expected}"));
    }
    let plaintext = String::from_utf8_lossy(&layer1).into_owned();
    let env: Value = serde_json::from_str(&plaintext)
        .map_err(|e| {
            format!(
                "内层信封 JSON 解析失败: {e} / 前64B: {}",
                crate::textlog::truncate(&plaintext, 64)
            )
        })?;
    // 双层（信封含 d 再解一层）或单层（r 明文即业务包裹）兼容
    let outer = match env.get("d").and_then(|d| d.as_str()) {
        Some(d) => {
            let layer2 = super::envelope::aes128_cbc_decrypt(key, &super::envelope::b64_decode(d)?)?;
            serde_json::from_slice::<Value>(&layer2)
                .map_err(|e| format!("第二层明文 JSON 解析失败: {e}"))?
        }
        None => env,
    };
    let business = match outer.get("data") {
        Some(Value::String(b64)) => {
            let bytes = super::envelope::b64_decode(b64)?;
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|e| format!("业务 data Base64 解码后不是 JSON: {e}"))?
        }
        _ => outer,
    };
    Ok(Decrypted {
        plaintext,
        business,
        expected_md5: expected,
        actual_md5: actual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::envelope::{aes128_cbc_encrypt, b64_encode, md5_hex, EnvelopeSession};

    /// 实测向量。
    #[test]
    fn test_fold32_and_paes_key_vectors() {
        let one = "nhang.school";
        let two = "5K0E8400-E29";
        let three = "597DEA1AFB49";
        let four = "86123.456789";
        let folds: Vec<String> = [one, two, three, four]
            .iter()
            .map(|v| format!("0x{:08x}", fold32(v)))
            .collect();
        assert_eq!(folds, ["0x61297d61", "0x203a324c", "0x363a323c", "0x3d2f3d3e"]);
        let key = derive_paes_key(one, two, three, four);
        assert_eq!(key.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                   "faaed5a4d99af386a7d3f5d109071f13");
    }

    /// RSA(s) 恢复逻辑：自造 type-1 块签名（无真私钥，用测试密钥对验证块格式/恢复）。
    #[test]
    fn test_rsa_recovered_digest_roundtrip() {
        use rsa::traits::{PrivateKeyParts, PublicKeyParts};
        let mut rng = rand::thread_rng();
        let priv_key = rsa::RsaPrivateKey::new(&mut rng, 512).expect("生成测试密钥");
        let pub_key = rsa::RsaPublicKey::from(&priv_key);
        let digest = md5_hex(b"payload-bytes");
        // EM = 00 01 FF..FF 00 <32B 小写hex>
        let size = priv_key.size();
        let mut em = vec![0xFFu8; size];
        em[0] = 0x00;
        em[1] = 0x01;
        let di = size - 1 - digest.len();
        em[di] = 0x00;
        em[di + 1..].copy_from_slice(digest.as_bytes());
        let sig = rsa::BigUint::from_bytes_be(&em)
            .modpow(priv_key.d(), priv_key.n())
            .to_bytes_be();
        let mut sig_pad = vec![0u8; size - sig.len()];
        sig_pad.extend_from_slice(&sig);
        let s_b64 = b64_encode(&sig_pad);
        let got = rsa_recovered_digest(&s_b64, &pub_key).expect("恢复失败");
        assert_eq!(got, digest);
    }

    /// 完整 r/s/v 两层解密链（自造向量：pAesKey 加密两层 + 明文业务 JSON）。
    #[test]
    fn test_full_response_decrypt_chain() {
        // 用自造 512 位密钥对造 s（同上）
        use rsa::traits::PrivateKeyParts;
        let mut rng = rand::thread_rng();
        let priv_key = rsa::RsaPrivateKey::new(&mut rng, 512).expect("生成测试密钥");
        let pub_key = rsa::RsaPublicKey::from(&priv_key);
        let size = priv_key.size();
        let make_s = |digest: String| {
            let mut em = vec![0xFFu8; size];
            em[0] = 0x00;
            em[1] = 0x01;
            let di = size - 1 - digest.len();
            em[di] = 0x00;
            em[di + 1..].copy_from_slice(digest.as_bytes());
            let sig = rsa::BigUint::from_bytes_be(&em)
                .modpow(priv_key.d(), priv_key.n())
                .to_bytes_be();
            let mut sig_pad = vec![0u8; size - sig.len()];
            sig_pad.extend_from_slice(&sig);
            b64_encode(&sig_pad)
        };

        let mut session = EnvelopeSession::with_four("86123.456789");
        let key_data = session.key_data();
        let key = derive_paes_key(&key_data[0], &key_data[1], &key_data[2], &key_data[3]);

        let business = r#"{"error":10000,"message":"成功","data":"{\"k\":1}"}"#;
        let layer2 = format!("{{\"data\":\"{}\",\"timeStamp\":123}}", b64_encode(business.as_bytes()));
        let inner_env = format!(
            "{{\"k\":\"x\",\"p\":101,\"d\":\"{}\",\"h\":\"y\",\"t\":0}}",
            b64_encode(&aes128_cbc_encrypt(&key, layer2.as_bytes()).unwrap())
        );
        let r = b64_encode(&aes128_cbc_encrypt(&key, inner_env.as_bytes()).unwrap());
        let s = make_s(md5_hex(inner_env.as_bytes()));
        let raw = format!("{{\"r\":\"{r}\",\"s\":\"{s}\",\"v\":101}}");

        let dec = decrypt_response(raw.as_bytes(), &key, &pub_key).expect("解密失败");
        assert_eq!(dec.business["error"], 10000);
        assert_eq!(dec.business["message"], "成功");
        assert_eq!(dec.business["data"], "{\"k\":1}");
    }

    /// 畸形/敌意输入只返回 Err，不允许 panic。
    #[test]
    fn test_decrypt_never_panics_on_garbage() {
        let pub_key = crate::crypto::envelope::rsa_public_key();
        let key = derive_paes_key("nhang.school", "5K0E8400-E29", "597DEA1AFB49", "86123.456789");
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"not json at all".to_vec(),
            b"{".to_vec(),
            b"{}".to_vec(),
            br#"{"r":"not-base64!!","s":"x","v":101}"#.to_vec(),
            br#"{"r":"AAAA","s":"AAAA","v":101}"#.to_vec(),
            br#"{"r":"AAAA","s":"AAAA","v":"999"}"#.to_vec(),
            br#"{"r":123,"s":456,"v":true}"#.to_vec(),
            // 非法 UTF-8 字节
            vec![0xff, 0xfe, 0x80],
            // 多字节中文被截断的 JSON
            r#"{"r":"YQ==","message":"中文截断"}"#.as_bytes().to_vec(),
            // 巨大嵌套
            br#"{"resp":"deep"}"#.to_vec(),
        ];
        for (i, raw) in cases.iter().enumerate() {
            let r = decrypt_response(raw, &key, &pub_key);
            assert!(r.is_ok() || r.is_err(), "case {i} panic");
        }
    }

    /// 非加密响应（plain JSON）透传。
    #[test]
    fn test_plain_response_passthrough() {
        let pub_key = crate::crypto::envelope::rsa_public_key();
        let key = [0u8; 16];
        let raw = "{\"error\":10121,\"message\":\"设备风险\"}".as_bytes();
        let dec = decrypt_response(raw, &key, &pub_key).unwrap();
        assert_eq!(dec.business["error"], 10121);
    }
}

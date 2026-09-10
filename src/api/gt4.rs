//! GT4 滑块验证（api/）。
//!
//! 流程：GET /load（bg/slice/lot_number/payload/process_token/pow_detail）
//!   → 下载两图 → 缺口识别（gt4image）→ PoW（sha256 前导 00）
//!   → get_w（AES-CBC 明文 + RSA 加密 str16 拼接）→ GET /verify → 四凭证。

use rand::Rng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::client::{make_agent, ureq_err};
use super::gt4image::get_distance_original;

const GEE_HOST: &str = "https://gcaptcha4.geetest.com";
const STATIC_HOST: &str = "https://static.geetest.com/";
const UA_WEB: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/// jsbn 28bit limb 数组 → RSA 公钥（e=65537）。
fn gee_rsa_public_key() -> rsa::RsaPublicKey {
    const LIMBS: [u32; 37] = [
        134982529, 254232810, 164556709, 234907349, 134685994, 35463984, 258277946, 12518857,
        44638621, 93783641, 212253739, 62792472, 186688352, 109500232, 182488077, 261196188,
        26354094, 103248217, 106891695, 165771045, 41530993, 263704736, 111785174, 12753611,
        232116673, 155524985, 218291229, 122452343, 248250238, 118739550, 251169095, 129059733,
        149835464, 5498868, 71719731, 154456417, 49635,
    ];
    let dv = rsa::BigUint::from(1u8) << 28usize;
    let mut n = rsa::BigUint::from(0u8);
    for limb in LIMBS.iter().rev() {
        n = n * &dv + rsa::BigUint::from(*limb);
    }
    rsa::RsaPublicKey::new(n, rsa::BigUint::from(65537u32)).expect("GT4 RSA 公钥构造失败")
}

/// 4 个随机 hex 字符。
fn four_random_chart(rng: &mut impl Rng) -> String {
    let v = (65536.0 * (1.0 + rng.r#gen::<f64>())) as u32;
    let s = format!("{v:x}");
    s[1..5].to_string()
}

/// 16 个随机 hex 字符。
fn get_str_16(rng: &mut impl Rng) -> String {
    (0..4).map(|_| four_random_chart(rng)).collect()
}

/// 找 nonce（16 个随机 hex 字符）使 sha256(base+nonce) 以 "00" 开头。
fn get_pow_msg_str_16(pow_msg_base: &str, log: &mut dyn FnMut(&str)) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut rng = rand::thread_rng();
    let mut attempts = 0u64;
    loop {
        let nonce: String = (0..16)
            .map(|_| HEX[rng.gen_range(0..16)] as char)
            .collect();
        let mut hasher = Sha256::new();
        hasher.update(pow_msg_base.as_bytes());
        hasher.update(nonce.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        attempts += 1;
        if hash.starts_with("00") {
            log(&format!("[gt4] PoW 找到 nonce（尝试 {attempts} 次）"));
            return nonce;
        }
    }
}

/// pow_msg = version|bits|hashfunc|datetime|captchaId|lot||nonce。
fn get_pow_msg(
    pow_detail: &Value,
    captcha_id: &str,
    lot_number: &str,
    log: &mut dyn FnMut(&str),
) -> String {
    let base = format!(
        "{}|{}|{}|{}|{}|{}||",
        pow_detail["version"].as_str().unwrap_or(""),
        pow_detail["bits"],
        pow_detail["hashfunc"].as_str().unwrap_or(""),
        pow_detail["datetime"].as_str().unwrap_or(""),
        captcha_id,
        lot_number
    );
    let nonce = get_pow_msg_str_16(&base, log);
    format!("{base}{nonce}")
}

/// pow_sign = sha256(pow_msg) hex。
fn get_pow_sign(pow_msg: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pow_msg.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// AES-128-CBC(key=str16, IV 为 ASCII "0"x16, PKCS7) → hex。
fn aes_o(plaintext: &str, str_16: &str) -> String {
    let ct = crate::crypto::envelope::aes128_cbc_encrypt_iv(
        str_16.as_bytes(),
        b"0000000000000000",
        plaintext.as_bytes(),
    )
    .expect("GT4 AES 加密失败");
    bytes_to_hex(&ct)
}

fn bytes_to_hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// RSA PKCS1v15 加密 → hex。
fn encrypt_data(pub_key: &rsa::RsaPublicKey, plaintext: &str) -> String {
    let mut rng = rand::thread_rng();
    let ct = pub_key
        .encrypt(&mut rng, rsa::Pkcs1v15Encrypt, plaintext.as_bytes())
        .expect("GT4 RSA 加密失败");
    bytes_to_hex(&ct)
}

/// w = hex(AES(plaintext)) + hex(RSA(str16))。
fn get_w(set_left: usize, lot_number: &str, pow_msg: &str, pow_sign: &str, str16: &str) -> String {
    let pk = gee_rsa_public_key();
    let userresponse = set_left as f64 / 1.005_946_666_666_666_5 + 2.0;
    let lf = |a: usize, b: usize| lot_number[a..b].to_string();
    let plaintext = format!(
        "{{\"setLeft\":{},\"passtime\":1887,\"userresponse\":{},\"device_id\":\"\",\"lot_number\":\"{}\",\"pow_msg\":\"{}\",\"pow_sign\":\"{}\",\"geetest\":\"captcha\",\"lang\":\"zh\",\"ep\":\"123\",\"biht\":\"1426265548\",\"gee_guard\":{{\"roe\":{{\"aup\":\"3\",\"sep\":\"3\",\"egp\":\"3\",\"auh\":\"3\",\"rew\":\"3\",\"snh\":\"3\",\"res\":\"3\",\"cdc\":\"3\"}}}},\"YciC\":\"P3Vn\",\"{}\":\"{}\",\"em\":{{\"ph\":0,\"cp\":0,\"ek\":\"11\",\"wd\":1,\"nt\":0,\"si\":0,\"sc\":0}}}}",
        set_left,
        userresponse,
        lot_number,
        pow_msg,
        pow_sign,
        lf(26, 30) + &lf(7, 11),
        lf(6, 14),
    );
    let r = encrypt_data(&pk, str16);
    let i = aes_o(&plaintext, str16);
    i + &r
}

fn jsonp_value(text: &str) -> Result<Value, String> {
    let start = text.find('(').ok_or("JSONP 缺左括号")?;
    let end = text.rfind(')').ok_or("JSONP 缺右括号")?;
    if end <= start {
        return Err("JSONP 格式错误".into());
    }
    serde_json::from_str(&text[start + 1..end]).map_err(|e| format!("JSONP JSON 解析失败: {e}"))
}

/// 完整 GT4 求解（带重试）。返回 {lotNumber, captchaOutput, passToken, genTime}。
pub fn solve_gt4(
    captcha_id: &str,
    _client: &mut super::client::ApiClient,
    log: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=3 {
        match solve_once(captcha_id, _client, log) {
            Ok(v) => return Ok(v),
            Err(e) => {
                log(&format!("× [gt4] 第 {attempt} 次失败: {e}"));
                last_err = e;
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }
    Err(format!("GT4 连续 3 次失败: {last_err}"))
}

fn solve_once(
    captcha_id: &str,
    _client: &mut super::client::ApiClient,
    log: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    let agent = make_agent();
    let challenge = uuid::Uuid::new_v4().to_string().replace('-', "");
    let callback = format!("geetest_{}", crate::crypto::envelope::now_ms());
    let load_url = format!(
        "{GEE_HOST}/load?callback={cb}&captcha_id={cid}&challenge={ch}&client_type=web&risk_type=slide&lang=zh",
        cb = callback,
        cid = captcha_id,
        ch = challenge
    );
    let resp = agent.get(&load_url).set("User-Agent", UA_WEB).call().map_err(ureq_err)?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let data = jsonp_value(&text)?["data"]
        .as_object()
        .ok_or("GT4 /load 缺 data")?
        .clone();
    let bg = data["bg"].as_str().ok_or("GT4 /load 缺 bg")?.to_string();
    let slice = data["slice"].as_str().ok_or("GT4 /load 缺 slice")?.to_string();
    let lot_number = data["lot_number"].as_str().ok_or("缺 lot_number")?.to_string();
    let payload = data["payload"].as_str().ok_or("缺 payload")?.to_string();
    let process_token = data["process_token"].as_str().ok_or("缺 process_token")?.to_string();
    let pow_detail = data["pow_detail"].clone();

    let bg_png = download(&agent, &format!("{STATIC_HOST}{bg}"))?;
    let slice_png = download(&agent, &format!("{STATIC_HOST}{slice}"))?;

    let dist = get_distance_original(&bg_png, &slice_png)?;
    log(&format!("[gt4] 缺口识别 distance={dist}"));

    let pow_msg = get_pow_msg(&pow_detail, captcha_id, &lot_number, log);
    let pow_sign = get_pow_sign(&pow_msg);
    let str16 = get_str_16(&mut rand::thread_rng());
    let w = get_w(dist, &lot_number, &pow_msg, &pow_sign, &str16);

    // verify
    let cb = format!("geetest_{}", crate::crypto::envelope::now_ms());
    let verify_url = format!(
        "{GEE_HOST}/verify?callback={cb}&captcha_id={cid}&client_type=web&lot_number={lot}&risk_type=slide&payload={pl}&process_token={pt}&payload_protocol=1&pt=1&w={w}",
        cid = captcha_id,
        lot = urlencode(&lot_number),
        pl = urlencode(&payload),
        pt = urlencode(&process_token),
        w = urlencode(&w),
    );
    let resp = agent.get(&verify_url).set("User-Agent", UA_WEB).call().map_err(ureq_err)?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let root = jsonp_value(&text)?;
    if root["status"].as_str() != Some("success") {
        return Err(format!("GT4 /verify 失败: {}", truncate(&text, 200)));
    }
    let seccode = &root["data"]["seccode"];
    Ok(json!({
        "lotNumber": seccode["lot_number"],
        "captchaOutput": seccode["captcha_output"],
        "passToken": seccode["pass_token"],
        "genTime": seccode["gen_time"],
    }))
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let resp = agent.get(url).set("User-Agent", UA_WEB).call().map_err(ureq_err)?;
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    crate::textlog::truncate(s, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::traits::PublicKeyParts;

    /// web 端 key：t=37 limbs ≈ 1036bit。
    #[test]
    fn test_gee_rsa_key_shape() {
        let pk = gee_rsa_public_key();
        assert_eq!(pk.e().to_bytes_be(), vec![1, 0, 1]); // 65537
        assert!(pk.size() >= 128, "GT4 key size={}", pk.size());
        // get_str_16 长度
        let s = get_str_16(&mut rand::thread_rng());
        assert_eq!(s.len(), 16);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// get_pow_msg 拼接 + pow_sign。
    #[test]
    fn test_pow_msg_and_sign() {
        let detail = json!({
            "version": "v1", "bits": 5, "hashfunc": "sha256",
            "datetime": "2026-09-09T00:00:00"
        });
        let mut logf = |_s: &str| {};
        let msg = get_pow_msg(&detail, "cid123", "lot456", &mut logf);
        assert!(msg.starts_with("v1|5|sha256|2026-09-09T00:00:00|cid123|lot456||"));
        assert_eq!(msg.len() - msg.rfind('|').unwrap() - 1, 16);
        let sign = get_pow_sign(&msg);
        assert_eq!(sign.len(), 64);
        // 自洽：sign == sha256(msg)
        let mut h = Sha256::new();
        h.update(msg.as_bytes());
        assert_eq!(format!("{:x}", h.finalize()), sign);
    }

    /// w 参数：格式 = aes_hex + rsa_hex，长度 = 32 字节块 + key 块。
    #[test]
    fn test_get_w_shape() {
        let lot = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6";
        let w = get_w(123, lot, "pow|msg||nonce1234567890ab", "sign", "0123456789abcdef");
        // AES 输出 ≥32 hex + RSA 130 字节 = 260 hex（key≈1036bit）
        assert!(w.len() > 260, "w len={}", w.len());
        assert!(w.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

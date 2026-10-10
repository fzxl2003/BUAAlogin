//! Srun uses JavaScript UTF-16 code units for xencode, UTF-8 for HMAC/SHA1.
use hmac::{Hmac, Mac};
use md5::Md5;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::sync::LazyLock;
use zeroize::Zeroizing;

pub fn parse_jsonp(text: &str) -> Result<Value, String> {
    let text = text.trim();
    let body = if text.starts_with('{') {
        text
    } else {
        static RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?s)^[\w.$]+\s*\((.*)\)\s*;?$").unwrap());
        let re = &*RE;
        re.captures(text)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str())
            .ok_or("网关返回了无法识别的 JSON/JSONP")?
    };
    let value: Value = serde_json::from_str(body).map_err(|_| "网关 JSON 格式错误")?;
    if !value.is_object() {
        return Err("网关返回值不是对象".into());
    }
    Ok(value)
}
fn words(text: &str, length: bool) -> Vec<u32> {
    let units = Zeroizing::new(text.encode_utf16().collect::<Vec<u16>>());
    let mut out = vec![0; units.len().div_ceil(4)];
    for (i, unit) in units.iter().enumerate() {
        out[i / 4] |= (*unit as u32) << ((i % 4) * 8);
    }
    if length {
        out.push(units.len() as u32);
    }
    out
}
pub fn xencode(text: &str, token: &str) -> Vec<u8> {
    if text.is_empty() {
        return vec![];
    }
    let mut v = words(text, true);
    let mut key = words(token, false);
    key.resize(key.len().max(4), 0);
    let n = v.len() - 1;
    let mut z = v[n];
    let mut sum = 0u32;
    for _ in 0..(6 + 52 / v.len()) {
        sum = sum.wrapping_add(0x9e3779b9);
        let e = (sum >> 2) & 3;
        for p in 0..=n {
            let y = v[if p == n { 0 } else { p + 1 }];
            let m = ((z >> 5) ^ (y << 2))
                .wrapping_add(((y >> 3) ^ (z << 4)) ^ (sum ^ y))
                .wrapping_add(key[(p & 3) ^ e as usize] ^ z);
            v[p] = v[p].wrapping_add(m);
            z = v[p];
        }
    }
    v.into_iter().flat_map(u32::to_le_bytes).collect()
}
pub fn srun_base64(bytes: &[u8]) -> String {
    const ALPHA: &[u8] = b"LVoJPiCN2R8G90yg+hmFHuacZ1OWMnrsSTXkYpUq/3dlbfKwv6xztjI7DeBE45QA";
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let b = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            output.push(if i > chunk.len() {
                '='
            } else {
                ALPHA[((b >> shift) & 63) as usize] as char
            });
        }
    }
    output
}
#[derive(Serialize)]
struct Info<'a> {
    username: &'a str,
    password: &'a str,
    ip: &'a str,
    acid: &'a str,
    enc_ver: &'a str,
}
pub struct LoginFields {
    pub password: String,
    pub info: String,
    pub checksum: String,
}
pub fn login_fields(
    username: &str,
    password: &str,
    ip: &str,
    acid: &str,
    token: &str,
) -> LoginFields {
    let info = Zeroizing::new(
        serde_json::to_string(&Info {
            username,
            password,
            ip,
            acid,
            enc_ver: "srun_bx1",
        })
        .unwrap(),
    );
    let encoded = Zeroizing::new(xencode(&info, token));
    let info = format!("{{SRBX1}}{}", srun_base64(&encoded));
    let mut hmac = Hmac::<Md5>::new_from_slice(token.as_bytes()).unwrap();
    hmac.update(password.as_bytes());
    let md5 = format!("{:x}", hmac.finalize().into_bytes());
    let check = [username, &md5, acid, ip, "200", "1", &info]
        .iter()
        .map(|v| format!("{token}{v}"))
        .collect::<String>();
    let checksum = format!("{:x}", Sha1::digest(check.as_bytes()));
    LoginFields {
        password: format!("{{MD5}}{md5}"),
        info,
        checksum,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jsonp() {
        assert_eq!(
            parse_jsonp("cb( {\"error\":\"ok\"} );").unwrap()["error"],
            "ok"
        );
        assert!(parse_jsonp("cb([])").is_err());
        assert!(parse_jsonp("<html>error</html>").is_err());
    }
    #[test]
    fn base64_padding() {
        assert_eq!(srun_base64(&[]), "");
        assert_eq!(srun_base64(&[0]), "LL==");
        assert_eq!(srun_base64(&[0, 0]), "LLL=");
        assert_eq!(srun_base64(&[0, 0, 0]), "LLLL");
    }
}

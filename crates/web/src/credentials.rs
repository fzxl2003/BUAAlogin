const MAGIC: &[u8] = b"BUAACRED2";
const SECRET: &[u8] = b"BUAALogin/native/local/credential/v2";

fn hash(bytes: &[u8]) -> u64 {
    SECRET
        .iter()
        .chain(bytes)
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn transform(bytes: &[u8], nonce: &[u8]) -> Vec<u8> {
    let mut state = hash(nonce) | 1;
    bytes
        .iter()
        .map(|byte| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            byte ^ state as u8
        })
        .collect()
}

pub fn encrypt(bytes: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; 16];
    if getrandom::fill(&mut nonce).is_err() {
        let fallback = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            ^ u128::from(std::process::id());
        nonce.copy_from_slice(&fallback.to_le_bytes());
    }
    let mut output = Vec::with_capacity(MAGIC.len() + nonce.len() + 8 + bytes.len());
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&hash(bytes).to_le_bytes());
    output.extend_from_slice(&transform(bytes, &nonce));
    output
}

pub fn decrypt(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let header = MAGIC.len() + 16 + 8;
    if bytes.len() < header || !bytes.starts_with(MAGIC) {
        return Err("应用内凭据格式无效".into());
    }
    let nonce = &bytes[MAGIC.len()..MAGIC.len() + 16];
    let tag = u64::from_le_bytes(
        bytes[MAGIC.len() + 16..header]
            .try_into()
            .map_err(|_| "应用内凭据格式无效")?,
    );
    let plain = transform(&bytes[header..], nonce);
    if hash(&plain) != tag {
        return Err("应用内凭据校验失败".into());
    }
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_integrity() {
        let plain = br#"{"username":"test","password":"secret"}"#;
        let encrypted = encrypt(plain);
        assert_ne!(&encrypted[encrypted.len() - plain.len()..], plain);
        assert_eq!(decrypt(&encrypted).unwrap(), plain);
        let mut damaged = encrypted;
        *damaged.last_mut().unwrap() ^= 1;
        assert!(decrypt(&damaged).is_err());
    }
}

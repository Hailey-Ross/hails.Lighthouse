//! Random secrets for shared keys.

/// Random hex string for shared secrets (the ingest secret, once mods can send status).
#[allow(dead_code)]
pub fn random_secret(bytes: usize) -> anyhow::Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow::anyhow!("no system randomness: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_hex_of_right_length() {
        let s = random_secret(32).unwrap();
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

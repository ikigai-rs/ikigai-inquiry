//! Sender keys: keyed BLAKE3 over a sender's address or domain, so a block names a sender
//! without holding the address.
//!
//! - an **address key** is the keyed hash of the address, trimmed and lowercased;
//! - a **domain key** is the keyed hash of `domain:<the domain, trimmed and lowercased>`, so no
//!   address hashes to a domain's key (an address always holds an `@`, and the `domain:` form
//!   never does).
//!
//! Nothing provider-specific is folded: `a.b@example.com` and `ab@example.com` are two senders,
//! and so are `a+x@example.com` and `a@example.com`. The 32-byte key is the host's, read from a
//! file it keeps; without it, a key says nothing about the address behind it.

use std::fmt;

/// The length of a key file's secret, in bytes.
pub const SECRET_BYTES: usize = 32;

/// The longest address (or domain) a key is computed for, in bytes: RFC 5321's path limit.
pub const MAX_ADDRESS_BYTES: usize = 254;

/// The context the inquiry-id key is derived under, so an id and a sender key never share a
/// hash even over the same bytes.
const ID_CONTEXT: &str = "ikigai-inquiry 2026-11-14 inquiry id";

/// A sender key: 64 lowercase hex digits, the keyed BLAKE3 hash of an address or a domain.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(String);

impl Key {
    /// A key from its text, refused unless it is 64 lowercase hex digits.
    ///
    /// ```
    /// use ikigai_inquiry::Key;
    ///
    /// assert!(Key::parse(&"0a".repeat(32)).is_ok());
    /// assert!(Key::parse(&"0A".repeat(32)).is_err());
    /// assert!(Key::parse("0a").is_err());
    /// ```
    pub fn parse(text: &str) -> Result<Key, String> {
        if text.len() == 2 * blake3::OUT_LEN
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Key(text.to_string()))
        } else {
            Err(format!(
                "`{text}` is not a sender key: {} lowercase hex digits",
                2 * blake3::OUT_LEN
            ))
        }
    }

    /// The key's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a key names: one address, or every address at a domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// One address.
    Address,
    /// A whole domain.
    Domain,
}

impl Scope {
    /// `address` or `domain`.
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Address => "address",
            Scope::Domain => "domain",
        }
    }

    /// The scope named `text`.
    pub fn parse(text: &str) -> Option<Scope> {
        match text {
            "address" => Some(Scope::Address),
            "domain" => Some(Scope::Domain),
            _ => None,
        }
    }
}

/// The host's sender-key secret.
#[derive(Clone)]
pub struct SenderKeys {
    secret: [u8; SECRET_BYTES],
    id_secret: [u8; SECRET_BYTES],
}

impl fmt::Debug for SenderKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SenderKeys(..)")
    }
}

impl SenderKeys {
    /// Keys under a 32-byte secret.
    pub fn new(secret: [u8; SECRET_BYTES]) -> Self {
        SenderKeys {
            secret,
            id_secret: blake3::derive_key(ID_CONTEXT, &secret),
        }
    }

    /// Keys under the secret in a file: exactly 32 bytes, or 64 hex digits and an optional
    /// trailing newline. On Unix the file is refused when its group or others may read or
    /// write it.
    #[cfg(not(target_family = "wasm"))]
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let shown = path.display();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path)
                .map_err(|e| format!("sender-key file {shown}: {e}"))?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(format!(
                    "sender-key file {shown} is open to its group or others (mode {:o}): \
                     make it 0600",
                    mode & 0o777
                ));
            }
        }
        let bytes = std::fs::read(path).map_err(|e| format!("sender-key file {shown}: {e}"))?;
        Self::from_key_bytes(&bytes).map_err(|e| format!("sender-key file {shown}: {e}"))
    }

    /// Keys under a key file's contents (see [`from_file`](Self::from_file)).
    pub fn from_key_bytes(bytes: &[u8]) -> Result<Self, String> {
        if let Ok(secret) = <[u8; SECRET_BYTES]>::try_from(bytes) {
            return Ok(Self::new(secret));
        }
        let hex = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        if hex.len() == 2 * SECRET_BYTES {
            let mut secret = [0u8; SECRET_BYTES];
            let digits = std::str::from_utf8(hex).ok();
            let parsed = digits.is_some_and(|digits| {
                secret.iter_mut().enumerate().all(|(i, byte)| {
                    u8::from_str_radix(&digits[2 * i..2 * i + 2], 16)
                        .map(|b| *byte = b)
                        .is_ok()
                })
            });
            if parsed {
                return Ok(Self::new(secret));
            }
        }
        Err(format!(
            "{} bytes: a key is exactly {SECRET_BYTES} bytes, or {} hex digits",
            bytes.len(),
            2 * SECRET_BYTES
        ))
    }

    /// The key of an address: trimmed and lowercased, nothing else folded.
    ///
    /// ```
    /// use ikigai_inquiry::SenderKeys;
    ///
    /// let keys = SenderKeys::new([7; 32]);
    /// let key = keys.address(" Ana@Example.COM ").unwrap();
    /// assert_eq!(key, keys.address("ana@example.com").unwrap());
    /// assert_ne!(key, keys.address("a.na@example.com").unwrap());
    /// assert_ne!(key, keys.domain("example.com").unwrap());
    /// assert!(keys.address("no at sign").is_err());
    /// ```
    pub fn address(&self, address: &str) -> Result<Key, String> {
        let address = normalize(address, "address")?;
        let (local, domain) = address
            .rsplit_once('@')
            .ok_or("an address holds an `@`: refused")?;
        if local.is_empty() || domain.is_empty() {
            return Err("an address has a name before its `@` and a domain after it".into());
        }
        Ok(self.hash(address.as_bytes()))
    }

    /// The key of a domain: `domain:` and the domain, trimmed and lowercased.
    pub fn domain(&self, domain: &str) -> Result<Key, String> {
        let domain = normalize(domain, "domain")?;
        if domain.contains('@') {
            return Err("a domain holds no `@`: refused".into());
        }
        Ok(self.hash(format!("domain:{domain}").as_bytes()))
    }

    /// The key of an address's domain (what follows its last `@`).
    pub fn domain_of(&self, address: &str) -> Result<Key, String> {
        let (_, domain) = address
            .trim()
            .rsplit_once('@')
            .ok_or("an address holds an `@`: refused")?;
        self.domain(domain)
    }

    fn hash(&self, bytes: &[u8]) -> Key {
        Key(blake3::keyed_hash(&self.secret, bytes).to_hex().to_string())
    }

    /// An inquiry's stable id: 32 hex digits of a keyed hash (under a key derived from the
    /// secret) of every part of the submission, each length-prefixed. The same submission at
    /// the same instant is the same inquiry; without the secret, the id says nothing of its
    /// text.
    pub(crate) fn inquiry_id(&self, parts: &[&str]) -> String {
        let mut hasher = blake3::Hasher::new_keyed(&self.id_secret);
        for part in parts {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        hasher.finalize().to_hex()[..32].to_string()
    }
}

fn normalize(text: &str, what: &str) -> Result<String, String> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return Err(format!("an empty {what}: refused"));
    }
    if text.len() > MAX_ADDRESS_BYTES {
        return Err(format!(
            "{} bytes, past the bound of {MAX_ADDRESS_BYTES} for an {what}: refused",
            text.len()
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(format!("a control character in the {what}: refused"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_key_is_keyed_blake3_of_the_trimmed_lowercased_address() {
        let secret = [42u8; 32];
        let keys = SenderKeys::new(secret);
        let expected = blake3::keyed_hash(&secret, b"ana@example.com").to_hex();
        assert_eq!(
            keys.address("  ANA@example.com\t").unwrap().as_str(),
            expected.as_str()
        );
    }

    #[test]
    fn the_domain_key_is_keyed_blake3_of_the_domain_form() {
        let secret = [42u8; 32];
        let keys = SenderKeys::new(secret);
        let expected = blake3::keyed_hash(&secret, b"domain:example.com").to_hex();
        assert_eq!(
            keys.domain_of("ana@Example.com").unwrap().as_str(),
            expected.as_str()
        );
        assert_eq!(
            keys.domain(" EXAMPLE.com").unwrap().as_str(),
            expected.as_str()
        );
    }

    #[test]
    fn another_secret_gives_other_keys() {
        let a = SenderKeys::new([1; 32]).address("ana@example.com").unwrap();
        let b = SenderKeys::new([2; 32]).address("ana@example.com").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn no_provider_specific_folding() {
        let keys = SenderKeys::new([3; 32]);
        let plain = keys.address("ana@gmail.com").unwrap();
        assert_ne!(plain, keys.address("a.na@gmail.com").unwrap());
        assert_ne!(plain, keys.address("ana+x@gmail.com").unwrap());
        assert_ne!(plain, keys.address("ana@googlemail.com").unwrap());
    }

    #[test]
    fn a_key_file_is_32_bytes_or_64_hex_digits() {
        let raw = SenderKeys::from_key_bytes(&[9; 32]).unwrap();
        let hex = SenderKeys::from_key_bytes(format!("{}\n", "09".repeat(32)).as_bytes()).unwrap();
        assert_eq!(raw.address("a@b").unwrap(), hex.address("a@b").unwrap());
        assert!(SenderKeys::from_key_bytes(&[9; 31]).is_err());
        assert!(SenderKeys::from_key_bytes("zz".repeat(32).as_bytes()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_others_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ikigai-inquiry-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sender.key");
        std::fs::write(&path, [5u8; 32]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(SenderKeys::from_file(&path).unwrap_err().contains("0600"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let keys = SenderKeys::from_file(&path).unwrap();
        assert_eq!(
            keys.address("a@b").unwrap(),
            SenderKeys::new([5; 32]).address("a@b").unwrap()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

//! Local vault: secrets encrypted at rest, unlocked with a master passphrase.
//!
//! * File: `<workspace>/vault.json`. Secret *names* are stored in clear (so they can be listed
//!   and referenced without unlocking); every *value* is sealed on its own.
//! * Key derivation: Argon2id (parameters and a random salt are stored in the file).
//! * Encryption: XChaCha20-Poly1305 with a fresh random 192-bit nonce per value. The secret's
//!   name is bound as associated data, so a sealed value cannot be moved to another name.
//! * Use: `{{vault:name}}` anywhere a variable can be used. Vault values are resolved only
//!   when a request is prepared for sending; they are not visible to scripts and are scrubbed
//!   from the request snapshot, the history and run reports.
//!
//! The passphrase is never stored. Losing it means losing the vault's contents.

use crate::model::VarMap;
use crate::store::Workspace;
use anyhow::{anyhow, bail, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use indexmap::IndexMap;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroizing;

pub const REFERENCE_PREFIX: &str = "vault:";
pub const MIN_PASSPHRASE_CHARS: usize = 8;
const CHECK_NAME: &str = "\u{0}check";
const CHECK_VALUE: &[u8] = b"rspostman vault";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Kdf {
    pub algorithm: String,
    /// Memory cost in KiB.
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    /// Base64.
    pub salt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sealed {
    /// Base64, 24 bytes.
    pub nonce: String,
    /// Base64 ciphertext with authentication tag.
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VaultFile {
    pub version: u32,
    pub cipher: String,
    pub kdf: Kdf,
    /// A sealed constant used to tell a wrong passphrase from a damaged file.
    pub check: Sealed,
    #[serde(default)]
    pub secrets: IndexMap<String, Sealed>,
}

/// An unlocked vault. The key is wiped from memory when this is dropped.
pub struct Vault {
    path: PathBuf,
    file: VaultFile,
    key: Zeroizing<[u8; 32]>,
}

// Never prints the key or any value.
impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault").field("path", &self.path).field("secrets", &self.file.secrets.len()).finish_non_exhaustive()
    }
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

pub fn path(ws: &Workspace) -> PathBuf {
    ws.root.join("vault.json")
}

pub fn exists(ws: &Workspace) -> bool {
    path(ws).is_file()
}

fn read(ws: &Workspace) -> Result<VaultFile> {
    let p = path(ws);
    let text = std::fs::read_to_string(&p).with_context(|| format!("cannot read {}", p.display()))?;
    let file: VaultFile = serde_json::from_str(&text).with_context(|| format!("{} is not a valid vault file", p.display()))?;
    if file.version != 1 || file.cipher != "xchacha20poly1305" || file.kdf.algorithm != "argon2id" {
        bail!("{} uses an unsupported vault format (version {}, {}, {})", p.display(), file.version, file.cipher, file.kdf.algorithm);
    }
    Ok(file)
}

/// Names of the stored secrets; works without the passphrase.
pub fn names(ws: &Workspace) -> Result<Vec<String>> {
    if !exists(ws) {
        return Ok(vec![]);
    }
    Ok(read(ws)?.secrets.keys().cloned().collect())
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn check_passphrase(passphrase: &str) -> Result<()> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        bail!("the vault passphrase must have at least {MIN_PASSPHRASE_CHARS} characters");
    }
    Ok(())
}

fn derive(passphrase: &str, kdf: &Kdf) -> Result<Zeroizing<[u8; 32]>> {
    let salt = b64().decode(&kdf.salt).map_err(|_| anyhow!("vault file is damaged (salt)"))?;
    // refuse absurd parameters from a tampered file instead of exhausting memory
    if kdf.memory_kib > 4 * 1024 * 1024 || kdf.iterations > 64 || kdf.parallelism > 16 {
        bail!("vault file asks for unreasonable key-derivation parameters");
    }
    let params = Params::new(kdf.memory_kib, kdf.iterations, kdf.parallelism, Some(32)).map_err(|e| anyhow!("invalid key-derivation parameters: {e}"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), &salt, key.as_mut())
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

fn aad(name: &str) -> Vec<u8> {
    format!("rspostman-vault:v1:{name}").into_bytes()
}

fn seal(key: &[u8; 32], name: &str, value: &[u8]) -> Result<Sealed> {
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let data = XChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: value, aad: &aad(name) })
        .map_err(|_| anyhow!("encryption failed"))?;
    Ok(Sealed { nonce: b64().encode(nonce), data: b64().encode(data) })
}

fn open(key: &[u8; 32], name: &str, sealed: &Sealed) -> Result<Zeroizing<Vec<u8>>> {
    let nonce = b64().decode(&sealed.nonce).ok().filter(|n| n.len() == 24).ok_or_else(|| anyhow!("vault entry is damaged (nonce)"))?;
    let data = b64().decode(&sealed.data).map_err(|_| anyhow!("vault entry is damaged (data)"))?;
    XChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(XNonce::from_slice(&nonce), Payload { msg: &data, aad: &aad(name) })
        .map(Zeroizing::new)
        .map_err(|_| anyhow!("cannot decrypt (wrong passphrase or modified vault file)"))
}

fn new_kdf() -> Kdf {
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    // OWASP-recommended Argon2id setting (19 MiB, 2 passes)
    Kdf { algorithm: "argon2id".into(), memory_kib: 19 * 1024, iterations: 2, parallelism: 1, salt: b64().encode(salt) }
}

impl Vault {
    /// Create an empty vault. Fails when one already exists.
    pub fn create(ws: &Workspace, passphrase: &str) -> Result<Vault> {
        if exists(ws) {
            bail!("a vault already exists in this workspace");
        }
        check_passphrase(passphrase)?;
        let kdf = new_kdf();
        let key = derive(passphrase, &kdf)?;
        let file = VaultFile { version: 1, cipher: "xchacha20poly1305".into(), check: seal(&key, CHECK_NAME, CHECK_VALUE)?, kdf, secrets: IndexMap::new() };
        let v = Vault { path: path(ws), file, key };
        v.save()?;
        Ok(v)
    }

    /// Unlock the vault of a workspace.
    pub fn unlock(ws: &Workspace, passphrase: &str) -> Result<Vault> {
        if !exists(ws) {
            bail!("this workspace has no vault yet");
        }
        let file = read(ws)?;
        let key = derive(passphrase, &file.kdf)?;
        match open(&key, CHECK_NAME, &file.check) {
            Ok(v) if v.as_slice() == CHECK_VALUE => {}
            _ => bail!("wrong vault passphrase"),
        }
        Ok(Vault { path: path(ws), file, key })
    }

    fn save(&self) -> Result<()> {
        crate::store::write_private(&self.path, serde_json::to_string_pretty(&self.file)?.as_bytes())
    }

    pub fn names(&self) -> Vec<String> {
        self.file.secrets.keys().cloned().collect()
    }

    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        if !valid_name(name) {
            bail!("invalid secret name `{name}` (use letters, digits, `_`, `-` and `.`)");
        }
        self.file.secrets.insert(name.to_string(), seal(&self.key, name, value.as_bytes())?);
        self.save()
    }

    pub fn get(&self, name: &str) -> Result<Option<String>> {
        match self.file.secrets.get(name) {
            None => Ok(None),
            Some(s) => {
                let bytes = open(&self.key, name, s).with_context(|| format!("vault secret `{name}`"))?;
                Ok(Some(String::from_utf8(bytes.to_vec()).map_err(|_| anyhow!("vault secret `{name}` is not valid UTF-8"))?))
            }
        }
    }

    pub fn remove(&mut self, name: &str) -> Result<bool> {
        let existed = self.file.secrets.shift_remove(name).is_some();
        if existed {
            self.save()?;
        }
        Ok(existed)
    }

    /// Every secret, decrypted. Used to resolve `{{vault:name}}` for one session.
    pub fn all(&self) -> Result<VarMap> {
        let mut out = VarMap::new();
        for name in self.file.secrets.keys() {
            if let Some(v) = self.get(name)? {
                out.insert(name.clone(), v);
            }
        }
        Ok(out)
    }

    /// Re-encrypt everything under a new passphrase (new salt, new key, new nonces).
    pub fn change_passphrase(&mut self, new_passphrase: &str) -> Result<()> {
        check_passphrase(new_passphrase)?;
        let plain = self.all()?;
        let kdf = new_kdf();
        let key = derive(new_passphrase, &kdf)?;
        let mut secrets = IndexMap::new();
        for (name, value) in &plain {
            secrets.insert(name.clone(), seal(&key, name, value.as_bytes())?);
        }
        self.file = VaultFile { version: 1, cipher: "xchacha20poly1305".into(), check: seal(&key, CHECK_NAME, CHECK_VALUE)?, kdf, secrets };
        self.key = key;
        self.save()
    }
}

/// Names referenced as `{{vault:name}}` in `text`.
pub fn references(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(end) => {
                if let Some(name) = after[..end].trim().strip_prefix(REFERENCE_PREFIX) {
                    let name = name.trim().to_string();
                    if !out.contains(&name) {
                        out.push(name);
                    }
                }
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> Workspace {
        Workspace::open(Some(std::env::temp_dir().join(format!("rsp-vault-{}", crate::model::new_id())))).unwrap()
    }

    #[test]
    fn roundtrip_listing_and_wrong_passphrase() {
        let ws = ws();
        assert!(!exists(&ws) && names(&ws).unwrap().is_empty());
        assert!(Vault::unlock(&ws, "whatever-long").unwrap_err().to_string().contains("no vault"));
        assert!(Vault::create(&ws, "short").is_err());
        let mut v = Vault::create(&ws, "correct horse battery").unwrap();
        v.set("api_token", "tok-ÄÖ-123").unwrap();
        v.set("db.password", "p@ss").unwrap();
        v.set("api_token", "tok-new").unwrap();
        assert!(v.set("bad name", "x").is_err() && v.set("", "x").is_err());
        drop(v);

        assert_eq!(names(&ws).unwrap(), ["api_token", "db.password"]);
        let on_disk = std::fs::read_to_string(path(&ws)).unwrap();
        for plain in ["tok-new", "p@ss", "correct horse"] {
            assert!(!on_disk.contains(plain), "{plain} is readable in vault.json");
        }
        assert!(Vault::create(&ws, "another passphrase").unwrap_err().to_string().contains("already exists"));
        assert_eq!(Vault::unlock(&ws, "wrong passphrase").err().unwrap().to_string(), "wrong vault passphrase");

        let mut v = Vault::unlock(&ws, "correct horse battery").unwrap();
        assert_eq!(v.get("api_token").unwrap().as_deref(), Some("tok-new"));
        assert_eq!(v.get("missing").unwrap(), None);
        assert_eq!(v.all().unwrap().len(), 2);
        assert!(v.remove("db.password").unwrap() && !v.remove("db.password").unwrap());
        assert_eq!(names(&ws).unwrap(), ["api_token"]);

        v.change_passphrase("a brand new passphrase").unwrap();
        assert!(Vault::unlock(&ws, "correct horse battery").is_err());
        assert_eq!(Vault::unlock(&ws, "a brand new passphrase").unwrap().get("api_token").unwrap().as_deref(), Some("tok-new"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path(&ws)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&ws.root).ok();
    }

    #[test]
    fn tampering_is_detected() {
        let ws = ws();
        let mut v = Vault::create(&ws, "correct horse battery").unwrap();
        v.set("a", "value-a").unwrap();
        v.set("b", "value-b").unwrap();
        drop(v);
        let original: VaultFile = serde_json::from_str(&std::fs::read_to_string(path(&ws)).unwrap()).unwrap();

        // swapping two sealed values is caught: the name is authenticated
        let mut swapped = original.clone();
        let (a, b) = (swapped.secrets["a"].clone(), swapped.secrets["b"].clone());
        swapped.secrets.insert("a".into(), b);
        swapped.secrets.insert("b".into(), a);
        std::fs::write(path(&ws), serde_json::to_string(&swapped).unwrap()).unwrap();
        let v = Vault::unlock(&ws, "correct horse battery").unwrap();
        assert!(v.get("a").unwrap_err().to_string().contains("vault secret `a`"));

        // a flipped ciphertext bit is caught
        let mut flipped = original.clone();
        let mut raw = b64().decode(&flipped.secrets["a"].data).unwrap();
        raw[0] ^= 1;
        flipped.secrets["a"].data = b64().encode(raw);
        std::fs::write(path(&ws), serde_json::to_string(&flipped).unwrap()).unwrap();
        assert!(Vault::unlock(&ws, "correct horse battery").unwrap().get("a").is_err());

        // hostile key-derivation parameters are refused
        let mut heavy = original.clone();
        heavy.kdf.memory_kib = u32::MAX;
        std::fs::write(path(&ws), serde_json::to_string(&heavy).unwrap()).unwrap();
        assert!(Vault::unlock(&ws, "correct horse battery").unwrap_err().to_string().contains("unreasonable"));
        std::fs::write(path(&ws), "{}").unwrap();
        assert!(Vault::unlock(&ws, "correct horse battery").is_err());
        std::fs::remove_dir_all(&ws.root).ok();
    }

    #[test]
    fn nonces_are_unique_and_references_found() {
        let key = [7u8; 32];
        let (a, b) = (seal(&key, "n", b"same").unwrap(), seal(&key, "n", b"same").unwrap());
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.data, b.data);
        assert_eq!(references("Bearer {{vault:tok}} {{ vault: other }} {{plain}} {{vault:tok}} {{vault:open"), ["tok", "other"]);
        assert!(valid_name("a.b-c_1") && !valid_name("a b") && !valid_name("a/b"));
    }
}

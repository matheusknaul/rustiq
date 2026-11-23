//! Authenticated, versioned persistence. Only collection metadata is public.
//! All request configuration is sealed; the random encryption key only lives in
//! the OS credential store and in zeroizing memory while the workspace is open.
use crate::workspace::{Entry, Id, Kind, Persistence, Workspace};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, Generate, KeyInit, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zeroize::Zeroizing;

const SERVICE: &str = "io.rustiq.desktop.workspace-key";
const VERSION: u32 = 3;
const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;
const KEYCHAIN_ERROR: &str = "OS credential store unavailable or locked. Unlock your keychain and retry. Nothing was saved in plaintext.";

pub trait KeyStore {
    fn get(&self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String>;
    fn set(&self, id: &str, key: &[u8]) -> Result<(), String>;
}
#[derive(Default)]
pub struct NativeKeyStore;
impl NativeKeyStore {
    fn entry(id: &str) -> Result<keyring_core::Entry, String> {
        // Initialize explicitly so a service that was unavailable can be retried.
        #[cfg(target_os = "linux")]
        let store: std::sync::Arc<keyring_core::CredentialStore> =
            zbus_secret_service_keyring_store::Store::new().map_err(|_| KEYCHAIN_ERROR)?;
        #[cfg(target_os = "macos")]
        let store: std::sync::Arc<keyring_core::CredentialStore> =
            apple_native_keyring_store::keychain::Store::new().map_err(|_| KEYCHAIN_ERROR)?;
        #[cfg(target_os = "windows")]
        let store: std::sync::Arc<keyring_core::CredentialStore> =
            windows_native_keyring_store::Store::new().map_err(|_| KEYCHAIN_ERROR)?;
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        return keyring_core::Entry::new_in_store(&store, SERVICE, id)
            .map_err(|_| KEYCHAIN_ERROR.into());
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        Err("Secure OS storage is not supported on this platform".into())
    }
}
impl KeyStore for NativeKeyStore {
    fn get(&self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        match Self::entry(id)?.get_secret() {
            Ok(key) => Ok(Some(Zeroizing::new(key))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(_) => Err(KEYCHAIN_ERROR.into()),
        }
    }
    fn set(&self, id: &str, key: &[u8]) -> Result<(), String> {
        Self::entry(id)?
            .set_secret(key)
            .map_err(|_| KEYCHAIN_ERROR.into())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    id: Id,
    parent: Option<Id>,
    kind: Kind,
    name: String,
    method: String,
    expanded: bool,
    persistence: Persistence,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format: String,
    version: u32,
    key_id: String,
    revision: String,
    next_id: Id,
    entries: Vec<Metadata>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    header: Header,
    nonce: String,
    ciphertext: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestData {
    url: String,
    request: rustiq_http::RequestOptions,
    #[serde(default)]
    history: Vec<crate::history::Exchange>,
}

// Probe only the document shape, without allocating plaintext secret strings.
#[derive(Deserialize)]
struct Probe {
    #[serde(default, deserialize_with = "present")]
    header: bool,
    #[serde(default, deserialize_with = "present")]
    ciphertext: bool,
    #[serde(default, deserialize_with = "present")]
    entries: bool,
}
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer)?;
    Ok(true)
}
#[derive(Serialize)]
struct RequestDataRef<'a> {
    url: &'a str,
    request: &'a rustiq_http::RequestOptions,
    history: &'a [crate::history::Exchange],
}
fn cleanup_legacy_temporary(path: &Path) -> Result<(), String> {
    let old_temporary = path.with_extension("json.tmp");
    if fs::symlink_metadata(&old_temporary).is_ok_and(|m| m.file_type().is_file()) {
        fs::remove_file(old_temporary).map_err(|_| "Encrypted workspace is safe, but its legacy plaintext temporary file could not be removed. Fix its permissions and retry.")?;
    }
    Ok(())
}

pub struct Storage<K: KeyStore = NativeKeyStore> {
    path: PathBuf,
    store: K,
    key_id: String,
    key: Option<Zeroizing<Vec<u8>>>,
    // Detect stale windows or external modifications, including edits to metadata.
    expected: Option<[u8; 32]>,
}

pub fn workspace_path() -> Result<PathBuf, String> {
    // Preserve the existing Linux path; use native config locations elsewhere.
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("XDG_CONFIG_HOME must be an absolute directory".into());
        }
        return Ok(path.join("rustiq/workspace.json"));
    }
    directories::BaseDirs::new()
        .map(|base| {
            let native = base.config_dir().join("rustiq/workspace.json");
            let legacy = base.home_dir().join(".config/rustiq/workspace.json");
            if !native.exists() && legacy.exists() {
                legacy
            } else {
                native
            }
        })
        .ok_or_else(|| "Could not find the user configuration directory".into())
}

fn read_file(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err("Workspace path must be a regular file".into());
        }
        Ok(meta) if meta.len() > MAX_FILE_SIZE => {
            return Err("Workspace exceeds the 64 MiB storage limit".into());
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Could not inspect workspace: {e}")),
        _ => {}
    }
    fs::read(path)
        .map(|bytes| Some(Zeroizing::new(bytes)))
        .map_err(|e| format!("Could not read workspace: {e}"))
}
fn fingerprint(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn parent(path: &Path) -> Result<&Path, String> {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "Workspace requires a parent directory".into())
}
fn lock(path: &Path) -> Result<File, String> {
    let directory = parent(path)?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(directory)
        .map_err(|e| format!("Could not create workspace directory: {e}"))?;
    let lock_path = path.with_extension("lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err("Workspace lock must be a regular file".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(lock_path)
        .map_err(|e| format!("Could not open workspace lock: {e}"))?;
    file.try_lock()
        .map_err(|_| "Another RustiQ process is updating this workspace. Retry shortly.")?;
    Ok(file)
}

impl<K: KeyStore> Storage<K> {
    pub fn open(path: PathBuf, store: K) -> Result<(Self, Workspace), String> {
        let guard = lock(&path)?;
        let bytes = read_file(&path)?;
        let mut storage = Self {
            path,
            store,
            key_id: Uuid::new_v4().to_string(),
            key: None,
            expected: bytes.as_deref().map(|b| fingerprint(b)),
        };
        let Some(bytes) = bytes else {
            return Ok((storage, Workspace::default()));
        };
        let probe: Probe = serde_json::from_slice(&bytes)
            .map_err(|_| "Workspace JSON is invalid; the original file was preserved")?;
        // A versioned document must never fall through to the plaintext reader.
        if probe.header || probe.ciphertext {
            let document: Document =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid encrypted workspace format")?;
            if !matches!(document.header.version, 2 | VERSION)
                || document.header.format != "rustiq-encrypted-workspace"
            {
                return Err(
                    "Unsupported workspace version; the original file was preserved".into(),
                );
            }
            Uuid::parse_str(&document.header.key_id)
                .map_err(|_| "Invalid workspace key identifier")?;
            Uuid::parse_str(&document.header.revision).map_err(|_| "Invalid workspace revision")?;
            storage.key_id = document.header.key_id.clone();
            storage.key = Some(storage.store.get(&storage.key_id)?.ok_or("Workspace key is missing from the OS credential store. Restore access to the original key; the encrypted file was preserved.")?);
            let key = storage.key.as_ref().expect("key was loaded");
            let cipher =
                Aes256Gcm::new_from_slice(key).map_err(|_| "Invalid workspace key length")?;
            let nonce = BASE64
                .decode(&document.nonce)
                .map_err(|_| "Invalid workspace nonce")?;
            let nonce: [u8; 12] = nonce
                .try_into()
                .map_err(|_| "Invalid workspace nonce length")?;
            let ciphertext = BASE64
                .decode(&document.ciphertext)
                .map_err(|_| "Invalid workspace ciphertext")?;
            let aad = serde_json::to_vec(&document.header)
                .map_err(|_| "Could not encode workspace metadata")?;
            let cleartext = Zeroizing::new(cipher.decrypt(&Nonce::from(nonce), Payload { msg: &ciphertext, aad: &aad }).map_err(|_| "Workspace authentication failed: data was modified, damaged, or the key is incorrect. The original file was preserved.")?);
            let mut requests: BTreeMap<Id, RequestData> =
                serde_json::from_slice(&cleartext).map_err(|_| "Invalid encrypted request data")?;
            let entries = document
                .header
                .entries
                .into_iter()
                .map(|meta| {
                    let data = match meta.persistence {
                        Persistence::Encrypted => requests
                            .remove(&meta.id)
                            .ok_or("Encrypted request data is missing")?,
                        Persistence::SessionOnly => RequestData {
                            url: String::new(),
                            request: Default::default(),
                            history: Vec::new(),
                        },
                    };
                    Ok(Entry {
                        id: meta.id,
                        parent: meta.parent,
                        kind: meta.kind,
                        name: meta.name,
                        method: meta.method,
                        expanded: meta.expanded,
                        persistence: meta.persistence,
                        url: data.url,
                        request: data.request,
                        history: data.history,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            if !requests.is_empty() {
                return Err("Encrypted workspace contains unknown requests".into());
            }
            let mut workspace = Workspace {
                entries,
                next_id: document.header.next_id,
            };
            workspace.validate()?;
            for entry in &mut workspace.entries {
                for exchange in &mut entry.history {
                    exchange.sanitize();
                }
            }
            crate::history::enforce_limits(&mut workspace);
            cleanup_legacy_temporary(&storage.path)?;
            Ok((storage, workspace))
        } else {
            // Legacy migration never creates a plaintext backup or writes until the
            // OS confirms that the encryption key was actually stored and readable.
            if !probe.entries {
                return Err("Unrecognized legacy workspace format".into());
            }
            let mut workspace: Workspace = serde_json::from_slice(&bytes)
                .map_err(|_| "Invalid legacy workspace; original file preserved")?;
            workspace.validate()?;
            for entry in &mut workspace.entries {
                for exchange in &mut entry.history {
                    exchange.sanitize();
                }
            }
            crate::history::enforce_limits(&mut workspace);
            drop(guard);
            storage.save(&workspace)?;
            // The old writer used this deterministic filename. Remove it only after
            // successful migration to avoid retaining a stale plaintext snapshot.
            cleanup_legacy_temporary(&storage.path)?;
            Ok((storage, workspace))
        }
    }

    fn unlock_or_create_key(&mut self) -> Result<(), String> {
        if self.key.is_some() {
            return Ok(());
        }
        let key = match self.store.get(&self.key_id)? {
            Some(key) => key,
            None => {
                let random = Zeroizing::new(
                    <[u8; 32]>::try_generate()
                        .map_err(|_| "Secure system randomness is unavailable")?,
                );
                let key = Zeroizing::new(random.to_vec());
                self.store.set(&self.key_id, &key)?;
                let stored = self
                    .store
                    .get(&self.key_id)?
                    .ok_or("OS credential store did not retain the workspace key")?;
                if *stored != *key {
                    return Err("OS credential store returned an unexpected workspace key".into());
                }
                key
            }
        };
        if key.len() != 32 {
            return Err("Invalid workspace key length".into());
        }
        self.key = Some(key);
        Ok(())
    }

    pub fn save(&mut self, workspace: &Workspace) -> Result<(), String> {
        self.save_with(workspace, atomic_replace)
    }

    fn save_with(
        &mut self,
        workspace: &Workspace,
        commit: impl FnOnce(&Path, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        workspace.validate()?;
        let _guard = lock(&self.path)?;
        let current = read_file(&self.path)?;
        if current.as_deref().map(|b| fingerprint(b)) != self.expected {
            return Err("Workspace changed in another process. Your edits remain in memory; reopen the workspace before saving to avoid overwriting other changes.".into());
        }
        self.unlock_or_create_key()?;
        let header = Header {
            format: "rustiq-encrypted-workspace".into(),
            version: VERSION,
            key_id: self.key_id.clone(),
            revision: Uuid::new_v4().to_string(),
            next_id: workspace.next_id,
            entries: workspace
                .entries
                .iter()
                .map(|e| Metadata {
                    id: e.id,
                    parent: e.parent,
                    kind: e.kind,
                    name: e.name.clone(),
                    method: e.method.clone(),
                    expanded: e.expanded,
                    persistence: e.persistence,
                })
                .collect(),
        };
        let requests: BTreeMap<_, _> = workspace
            .entries
            .iter()
            .filter(|e| e.persistence == Persistence::Encrypted)
            .map(|e| {
                (
                    e.id,
                    RequestDataRef {
                        url: &e.url,
                        request: &e.request,
                        history: &e.history,
                    },
                )
            })
            .collect();
        let cleartext = Zeroizing::new(
            serde_json::to_vec(&requests).map_err(|_| "Could not encode encrypted request data")?,
        );
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref().expect("key is unlocked"))
            .map_err(|_| "Invalid workspace key length")?;
        let nonce = Nonce::try_generate().map_err(|_| "Secure system randomness is unavailable")?;
        let aad = serde_json::to_vec(&header).map_err(|_| "Could not encode workspace metadata")?;
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &cleartext,
                    aad: &aad,
                },
            )
            .map_err(|_| "Workspace encryption failed")?;
        let document = Document {
            header,
            nonce: BASE64.encode(nonce),
            ciphertext: BASE64.encode(ciphertext),
        };
        let bytes = serde_json::to_vec_pretty(&document)
            .map_err(|_| "Could not encode encrypted workspace")?;
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err("Workspace exceeds the 64 MiB storage limit".into());
        }
        commit(&self.path, &bytes)?;
        self.expected = Some(fingerprint(&bytes));
        #[cfg(unix)]
        File::open(parent(&self.path)?)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| format!("Snapshot saved but directory sync failed: {e}"))?;
        Ok(())
    }
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    // Random temporary files are exclusively created in the workspace directory.
    // Neither secrets nor encryption keys are ever written to temporary files.
    let mut temporary = tempfile::NamedTempFile::new_in(parent(path)?)
        .map_err(|e| format!("Could not create encrypted snapshot: {e}"))?;
    temporary
        .write_all(bytes)
        .map_err(|e| format!("Could not write encrypted snapshot: {e}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| format!("Could not sync encrypted snapshot: {e}"))?;
    temporary
        .persist(path)
        .map_err(|_| "Could not commit encrypted workspace; previous file preserved")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Clone, Default)]
    struct FakeKeyStore {
        keys: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        unavailable: Arc<AtomicBool>,
        discard_writes: Arc<AtomicBool>,
    }
    impl KeyStore for FakeKeyStore {
        fn get(&self, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(KEYCHAIN_ERROR.into());
            }
            Ok(self
                .keys
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .map(Zeroizing::new))
        }
        fn set(&self, id: &str, key: &[u8]) -> Result<(), String> {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(KEYCHAIN_ERROR.into());
            }
            if !self.discard_writes.load(Ordering::SeqCst) {
                self.keys.lock().unwrap().insert(id.into(), key.into());
            }
            Ok(())
        }
    }
    fn workspace() -> Workspace {
        let mut workspace = Workspace::default();
        let collection = workspace
            .create(Kind::Collection, None, "Public collection name")
            .unwrap();
        let id = workspace
            .create(Kind::Request, Some(collection), "Public request name")
            .unwrap();
        let entry = workspace.entry_mut(id).unwrap();
        entry.url = "https://example.com/private?token=url-secret".into();
        entry.request.auth.password = "password-secret".into();
        entry.request.auth.token = "bearer-secret".into();
        entry.request.auth.client_secret = "oauth-secret".into();
        entry.request.auth.key_value = "api-key-secret".into();
        entry.request.headers.push(rustiq_http::Pair {
            name: "X-Custom".into(),
            value: "header-secret".into(),
            enabled: false,
        });
        entry.request.params.push(rustiq_http::Pair {
            name: "q".into(),
            value: "param-secret".into(),
            enabled: true,
        });
        entry.request.cookies.push(rustiq_http::Pair {
            name: "session".into(),
            value: "cookie-secret".into(),
            enabled: true,
        });
        entry.request.body.text = "body-secret".into();
        workspace
    }
    fn file(path: &Path) -> Vec<u8> {
        fs::read(path).unwrap()
    }
    fn secrets_absent(path: &Path) {
        for entry in fs::read_dir(parent(path).unwrap()).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                let bytes = fs::read(entry.path()).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                for secret in [
                    "url-secret",
                    "password-secret",
                    "bearer-secret",
                    "oauth-secret",
                    "api-key-secret",
                    "header-secret",
                    "param-secret",
                    "cookie-secret",
                    "body-secret",
                ] {
                    assert!(
                        !text.contains(secret),
                        "plaintext secret in persistence file"
                    );
                }
            }
        }
    }
    #[test]
    fn round_trip_encrypts_every_request_field_and_uses_private_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut storage, _) = Storage::open(path.clone(), store.clone()).unwrap();
        storage.save(&workspace()).unwrap();
        secrets_absent(&path);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Public collection name"));
        assert!(!text.contains("https://example.com"));
        let (reopened, loaded) = Storage::open(path.clone(), store.clone()).unwrap();
        assert_eq!(reopened.key_id, storage.key_id);
        assert_eq!(loaded.entry(2).unwrap().request.auth.token, "bearer-secret");
        assert_eq!(loaded.entry(2).unwrap().request.body.text, "body-secret");
        assert_eq!(
            loaded.entry(2).unwrap().request.headers[0].value,
            "header-secret"
        );
        assert_eq!(
            loaded.entry(2).unwrap().url,
            "https://example.com/private?token=url-secret"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.with_extension("lock"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        // The raw master key is not part of the JSON or any sibling file.
        let keys = store.keys.lock().unwrap();
        let key = keys.get(&storage.key_id).unwrap();
        assert!(!text.contains(&BASE64.encode(key)));
    }
    #[test]
    fn tampering_and_wrong_keys_fail_closed_without_replacing_anything() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut storage, _) = Storage::open(path.clone(), store.clone()).unwrap();
        storage.save(&workspace()).unwrap();
        let original = file(&path);
        let mut document: Document = serde_json::from_slice(&original).unwrap();
        document.header.entries[0].name = "Tampered".into();
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(Storage::open(path.clone(), store.clone()).is_err());
        assert!(fs::read_to_string(&path).unwrap().contains("Tampered"));
        fs::write(&path, &original).unwrap();
        document = serde_json::from_slice(&original).unwrap();
        let mut ciphertext = BASE64.decode(&document.ciphertext).unwrap();
        ciphertext[0] ^= 1;
        document.ciphertext = BASE64.encode(ciphertext);
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(Storage::open(path.clone(), store.clone()).is_err());
        fs::write(&path, &original).unwrap();
        store
            .keys
            .lock()
            .unwrap()
            .insert(storage.key_id.clone(), vec![7; 32]);
        assert!(Storage::open(path.clone(), store).is_err());
        assert_eq!(file(&path), original);
    }
    #[test]
    fn missing_or_locked_keychain_never_generates_a_replacement_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut storage, _) = Storage::open(path.clone(), store.clone()).unwrap();
        storage.save(&workspace()).unwrap();
        let original = file(&path);
        store.unavailable.store(true, Ordering::SeqCst);
        assert!(Storage::open(path.clone(), store.clone()).is_err());
        store.unavailable.store(false, Ordering::SeqCst);
        store.keys.lock().unwrap().clear();
        assert!(Storage::open(path.clone(), store.clone()).is_err());
        assert!(store.keys.lock().unwrap().is_empty());
        assert_eq!(file(&path), original);
    }
    #[test]
    fn session_only_configuration_is_removed_on_save_and_not_restored() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut storage, _) = Storage::open(path.clone(), store.clone()).unwrap();
        let mut workspace = workspace();
        storage.save(&workspace).unwrap();
        workspace.entry_mut(2).unwrap().persistence = Persistence::SessionOnly;
        storage.save(&workspace).unwrap();
        assert_eq!(
            workspace.entry(2).unwrap().request.auth.token,
            "bearer-secret"
        );
        let (_, loaded) = Storage::open(path.clone(), store).unwrap();
        let entry = loaded.entry(2).unwrap();
        assert!(entry.url.is_empty());
        assert!(entry.request.auth.token.is_empty());
        assert!(entry.request.headers.is_empty());
        assert_eq!(entry.persistence, Persistence::SessionOnly);
        secrets_absent(&path);
    }
    #[test]
    fn legacy_migration_is_transactional_and_removes_legacy_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let original = serde_json::to_vec(&workspace()).unwrap();
        fs::write(&path, &original).unwrap();
        fs::write(path.with_extension("json.tmp"), &original).unwrap();
        store.unavailable.store(true, Ordering::SeqCst);
        assert!(Storage::open(path.clone(), store.clone()).is_err());
        assert_eq!(file(&path), original);
        store.unavailable.store(false, Ordering::SeqCst);
        let (_, loaded) = Storage::open(path.clone(), store.clone()).unwrap();
        assert_eq!(
            loaded.entry(2).unwrap().request.auth.password,
            "password-secret"
        );
        assert!(!path.with_extension("json.tmp").exists());
        secrets_absent(&path);
        assert!(Storage::open(path, store).is_ok());
    }
    #[test]
    fn old_workspaces_without_request_options_are_migrated() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        fs::write(&path, br#"{"entries":[{"id":1,"parent":null,"kind":"Collection","name":"Old","method":"GET","url":"","expanded":true}]}"#).unwrap();
        let (_, loaded) = Storage::open(path, FakeKeyStore::default()).unwrap();
        assert!(loaded.entry(1).unwrap().request.auth.token.is_empty());
    }
    #[test]
    fn key_write_must_be_verified_before_the_first_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        store.discard_writes.store(true, Ordering::SeqCst);
        let (mut storage, _) = Storage::open(path.clone(), store).unwrap();
        assert!(storage.save(&workspace()).is_err());
        assert!(!path.exists());
        secrets_absent(&path);
    }
    #[test]
    fn stale_writers_and_concurrent_updates_cannot_overwrite_saved_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut first, _) = Storage::open(path.clone(), store.clone()).unwrap();
        let (mut stale, _) = Storage::open(path.clone(), store.clone()).unwrap();
        first.save(&workspace()).unwrap();
        let original = file(&path);
        assert!(stale.save(&Workspace::default()).is_err());
        assert_eq!(file(&path), original);
        let guard = lock(&path).unwrap();
        assert!(first.save(&Workspace::default()).is_err());
        drop(guard);
        first.save(&workspace()).unwrap();
        let updated: Document = serde_json::from_slice(&file(&path)).unwrap();
        let previous: Document = serde_json::from_slice(&original).unwrap();
        assert_ne!(updated.nonce, previous.nonce);
        assert_ne!(updated.ciphertext, previous.ciphertext);
        assert!(Storage::open(path, store).is_ok());
    }
    #[test]
    fn failed_commits_keep_the_previous_snapshot_and_can_be_retried() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut storage, _) = Storage::open(path.clone(), store).unwrap();
        let mut workspace = workspace();
        storage.save(&workspace).unwrap();
        let original = file(&path);
        workspace.entry_mut(2).unwrap().request.auth.token = "updated-token".into();
        assert!(
            storage
                .save_with(&workspace, |_, bytes| {
                    assert!(!String::from_utf8_lossy(bytes).contains("updated-token"));
                    Err("Simulated disk failure".into())
                })
                .is_err()
        );
        assert_eq!(file(&path), original);
        storage.save(&workspace).unwrap();
        assert_ne!(file(&path), original);
    }
    #[test]
    fn malformed_future_versions_and_symlinks_are_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        fs::write(&path, b"not-json").unwrap();
        assert!(Storage::open(path.clone(), store.clone()).is_err());
        assert_eq!(file(&path), b"not-json");
        fs::remove_file(&path).unwrap();
        let (mut storage, _) = Storage::open(path.clone(), store.clone()).unwrap();
        storage.save(&workspace()).unwrap();
        let mut doc: Document = serde_json::from_slice(&file(&path)).unwrap();
        doc.header.version = VERSION + 1;
        fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(Storage::open(path.clone(), store.clone()).is_err());
        #[cfg(unix)]
        {
            let link = directory.path().join("linked.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(Storage::open(link, store).is_err());
        }
    }
    #[test]
    fn deleted_requests_do_not_survive_in_the_encrypted_payload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut storage, _) = Storage::open(path.clone(), store.clone()).unwrap();
        let mut workspace = workspace();
        storage.save(&workspace).unwrap();
        workspace.remove(2);
        storage.save(&workspace).unwrap();
        let (_, loaded) = Storage::open(path.clone(), store).unwrap();
        assert!(loaded.entry(2).is_none());
        secrets_absent(&path);
    }
    #[test]
    fn history_round_trip_is_encrypted_clearable_and_respects_session_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut storage, _) = Storage::open(path.clone(), store.clone()).unwrap();
        let mut workspace = workspace();
        crate::history::record(&mut workspace, 2, &crate::history::tests::response()).unwrap();
        storage.save(&workspace).unwrap();
        let bytes = file(&path);
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("response-secret"));
        assert!(!text.contains("original-request"));
        let (mut reopened, mut loaded) = Storage::open(path.clone(), store.clone()).unwrap();
        let history = &loaded.entry(2).unwrap().history;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].response.body, "response-secret");
        assert_eq!(history[0].request.body, "original-request");
        assert_eq!(history[0].response.headers.len(), 1);
        assert_eq!(history[0].request.headers.len(), 1);
        loaded.entry_mut(2).unwrap().history.clear();
        reopened.save(&loaded).unwrap();
        let (mut reopened, mut loaded) = Storage::open(path.clone(), store.clone()).unwrap();
        assert!(loaded.entry(2).unwrap().history.is_empty());
        crate::history::record(&mut loaded, 2, &crate::history::tests::response()).unwrap();
        loaded.entry_mut(2).unwrap().persistence = Persistence::SessionOnly;
        reopened.save(&loaded).unwrap();
        let (_, loaded) = Storage::open(path, store).unwrap();
        assert!(loaded.entry(2).unwrap().history.is_empty());
    }

    #[test]
    fn encrypted_v2_snapshots_without_history_remain_readable_and_upgrade_on_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let store = FakeKeyStore::default();
        let (mut storage, _) = Storage::open(path.clone(), store.clone()).unwrap();
        storage.save(&workspace()).unwrap();
        let mut document: Document = serde_json::from_slice(&file(&path)).unwrap();
        let cipher = Aes256Gcm::new_from_slice(storage.key.as_ref().unwrap()).unwrap();
        let old_nonce: [u8; 12] = BASE64.decode(&document.nonce).unwrap().try_into().unwrap();
        let data = cipher
            .decrypt(
                &Nonce::from(old_nonce),
                Payload {
                    msg: &BASE64.decode(&document.ciphertext).unwrap(),
                    aad: &serde_json::to_vec(&document.header).unwrap(),
                },
            )
            .unwrap();
        let mut data: serde_json::Value = serde_json::from_slice(&data).unwrap();
        for request in data.as_object_mut().unwrap().values_mut() {
            request.as_object_mut().unwrap().remove("history");
        }
        document.header.version = 2;
        let nonce = Nonce::generate();
        document.nonce = BASE64.encode(nonce);
        document.ciphertext = BASE64.encode(
            cipher
                .encrypt(
                    &nonce,
                    Payload {
                        msg: &serde_json::to_vec(&data).unwrap(),
                        aad: &serde_json::to_vec(&document.header).unwrap(),
                    },
                )
                .unwrap(),
        );
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let (mut reopened, loaded) = Storage::open(path.clone(), store).unwrap();
        assert_eq!(loaded.entry(2).unwrap().request.auth.token, "bearer-secret");
        assert!(loaded.entry(2).unwrap().history.is_empty());
        reopened.save(&loaded).unwrap();
        let updated: Document = serde_json::from_slice(&file(&path)).unwrap();
        assert_eq!(updated.header.version, VERSION);
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;

    #[test]
    #[ignore = "requires an unlocked OS keychain; creates and removes a disposable test key"]
    fn native_keychain_round_trip() {
        let id = Uuid::new_v4().to_string();
        let entry = NativeKeyStore::entry(&id).expect("native credential store must be available");
        struct Cleanup(keyring_core::Entry);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.delete_credential();
            }
        }
        let _cleanup = Cleanup(entry);
        let store = NativeKeyStore;
        let key = Zeroizing::new(vec![42; 32]);
        store.set(&id, &key).expect("disposable key must be stored");
        let loaded = store
            .get(&id)
            .expect("disposable key must be readable")
            .expect("key must exist");
        assert!(*loaded == *key, "keychain must return the original bytes");
        drop(_cleanup);
        assert!(store.get(&id).unwrap().is_none());
    }
}

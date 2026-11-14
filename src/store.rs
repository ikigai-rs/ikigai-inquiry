//! Where the block set and the day counters are kept: named documents in a [`Store`] the host
//! chooses. Neither ever holds an address, a name or a message: a block is a keyed hash and a
//! counter is a number.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// A place to keep named documents. A save replaces the whole document.
pub trait Store: Send + Sync {
    /// The document `name`, or `None` when there is none yet.
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, String>;
    /// Replace the document `name` with `bytes`.
    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), String>;
}

/// A store in memory: nothing survives the process.
#[derive(Debug, Default)]
pub struct MemoryStore(Mutex<BTreeMap<String, Vec<u8>>>);

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        MemoryStore::default()
    }
}

impl Store for MemoryStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.0.lock().expect("store lock").get(name).cloned())
    }

    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.0
            .lock()
            .expect("store lock")
            .insert(name.to_string(), bytes.to_vec());
        Ok(())
    }
}

/// A store over a directory: one file per document, `<name>.json`, each replaced by writing a
/// temporary file beside it and renaming it over the old one, so a failed save keeps the
/// previous document.
#[cfg(not(target_family = "wasm"))]
#[derive(Debug)]
pub struct DirStore {
    dir: std::path::PathBuf,
}

#[cfg(not(target_family = "wasm"))]
impl DirStore {
    /// A store over `dir`, created if it is missing.
    pub fn open(dir: impl Into<std::path::PathBuf>) -> Result<DirStore, String> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(DirStore { dir })
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.join(format!("{name}.json"))
    }
}

#[cfg(not(target_family = "wasm"))]
impl Store for DirStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let path = self.path(name);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.path(name);
        let temporary = self.dir.join(format!(".{name}.json.tmp"));
        std::fs::write(&temporary, bytes).map_err(|e| format!("{}: {e}", temporary.display()))?;
        std::fs::rename(&temporary, &path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

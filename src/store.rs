//! Where the block set and the day counters are kept: named documents in a [`Store`] the host
//! chooses. Neither ever holds an address, a name or a message: a block is a keyed hash and a
//! counter is a number.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// A document's version: whatever a store can tell cheaply that changes whenever the document
/// does (for a [`DirStore`], the file's identity, length and times). Equal versions mean the
/// document has not changed, so a reader may keep what it read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Version(pub [u64; 4]);

impl Version {
    /// The version of a document that is not there.
    pub const ABSENT: Version = Version([0; 4]);
}

/// One change to a document: given what it holds now (`None` when there is none yet), the bytes
/// to replace it with, or `None` to leave it as it is.
pub type Change<'a> = dyn FnMut(Option<&[u8]>) -> Result<Option<Vec<u8>>, String> + 'a;

/// A place to keep named documents. A save replaces the whole document.
///
/// More than one process may open the same store (the contact daemon, the HTTP door, a shell's
/// `import-blocks`), so a reader asks [`Store::version`] before trusting what it read, and a
/// writer goes through [`Store::update`], never a load then a save of its own.
pub trait Store: Send + Sync {
    /// The document `name`, or `None` when there is none yet.
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, String>;
    /// Replace the document `name` with `bytes`.
    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), String>;

    /// The version of `name` now, or `None` when this store cannot tell (a reader then loads
    /// it again every time). Read the version before the document, never after, so what was
    /// read is never newer than the version kept beside it.
    fn version(&self, name: &str) -> Result<Option<Version>, String> {
        let _ = name;
        Ok(None)
    }

    /// Load `name`, apply `change` and save what it answers, with no other writer's save in
    /// between. The default is a plain load and save, which only holds for a store no other
    /// process writes; [`MemoryStore`] and [`DirStore`] hold it across threads and processes.
    fn update(&self, name: &str, change: &mut Change<'_>) -> Result<(), String> {
        let current = self.load(name)?;
        match change(current.as_deref())? {
            Some(bytes) => self.save(name, &bytes),
            None => Ok(()),
        }
    }
}

/// A store in memory: nothing survives the process.
#[derive(Debug, Default)]
pub struct MemoryStore(Mutex<BTreeMap<String, (u64, Vec<u8>)>>);

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        MemoryStore::default()
    }
}

fn put(documents: &mut BTreeMap<String, (u64, Vec<u8>)>, name: &str, bytes: &[u8]) {
    let generation = documents.get(name).map_or(0, |(g, _)| *g) + 1;
    documents.insert(name.to_string(), (generation, bytes.to_vec()));
}

impl Store for MemoryStore {
    fn load(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self
            .0
            .lock()
            .expect("store lock")
            .get(name)
            .map(|(_, bytes)| bytes.clone()))
    }

    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        put(&mut self.0.lock().expect("store lock"), name, bytes);
        Ok(())
    }

    fn version(&self, name: &str) -> Result<Option<Version>, String> {
        let documents = self.0.lock().expect("store lock");
        Ok(Some(
            documents
                .get(name)
                .map_or(Version::ABSENT, |(g, _)| Version([*g, 0, 0, 0])),
        ))
    }

    fn update(&self, name: &str, change: &mut Change<'_>) -> Result<(), String> {
        let mut documents = self.0.lock().expect("store lock");
        let current = documents.get(name).map(|(_, bytes)| bytes.as_slice());
        if let Some(bytes) = change(current)? {
            put(&mut documents, name, &bytes);
        }
        Ok(())
    }
}

/// A store over a directory: one file per document, `<name>.json`, each replaced by writing a
/// temporary file beside it and renaming it over the old one, so a failed save keeps the
/// previous document. An update holds an exclusive lock on `.<name>.json.lock` beside it, so
/// writers in other processes wait their turn rather than save over each other.
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
        static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = self.path(name);
        // A temporary file of this save's own, so two saves never rename each other's.
        let n = SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = self
            .dir
            .join(format!(".{name}.json.{}-{n}.tmp", std::process::id()));
        let written = std::fs::write(&temporary, bytes)
            .map_err(|e| format!("{}: {e}", temporary.display()))
            .and_then(|()| {
                std::fs::rename(&temporary, &path).map_err(|e| format!("{}: {e}", path.display()))
            });
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written
    }

    fn version(&self, name: &str) -> Result<Option<Version>, String> {
        let path = self.path(name);
        match std::fs::metadata(&path) {
            Ok(metadata) => Ok(Some(version_of(&metadata))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Some(Version::ABSENT)),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn update(&self, name: &str, change: &mut Change<'_>) -> Result<(), String> {
        let path = self.dir.join(format!(".{name}.json.lock"));
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        // Released when `lock` is closed, on every return below.
        lock.lock()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let current = self.load(name)?;
        match change(current.as_deref())? {
            Some(bytes) => self.save(name, &bytes),
            None => Ok(()),
        }
    }
}

/// A file's version: every save renames a new file over the old, so its identity changes with
/// each save, and its length and modification time change with most.
#[cfg(not(target_family = "wasm"))]
fn version_of(metadata: &std::fs::Metadata) -> Version {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let (device, inode) = (0, 0);
    // Never `ABSENT`: a file that is there has a nonzero first word.
    Version([device | 1 << 63, inode, metadata.len(), modified])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bump(store: &dyn Store) {
        store
            .update("n", &mut |bytes| {
                let n: u64 = bytes.map_or(0, |b| std::str::from_utf8(b).unwrap().parse().unwrap());
                Ok(Some((n + 1).to_string().into_bytes()))
            })
            .unwrap();
    }

    fn versions_follow_saves(store: &dyn Store) {
        assert_eq!(store.version("n").unwrap(), Some(Version::ABSENT));
        bump(store);
        let one = store.version("n").unwrap();
        assert_ne!(one, Some(Version::ABSENT));
        assert_eq!(
            store.version("n").unwrap(),
            one,
            "unchanged, the same version"
        );
        bump(store);
        assert_ne!(store.version("n").unwrap(), one);
        store.update("n", &mut |_| Ok(None)).unwrap();
        assert_eq!(store.load("n").unwrap().as_deref(), Some(&b"2"[..]));
    }

    #[test]
    fn a_memory_store_versions_its_documents() {
        versions_follow_saves(&MemoryStore::new());
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn a_dir_store_versions_its_documents_and_no_update_is_lost() {
        let dir = std::env::temp_dir().join(format!("ikigai-inquiry-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        versions_follow_saves(&DirStore::open(&dir).unwrap());
        std::thread::scope(|s| {
            for _ in 0..8 {
                // Each thread its own store over the directory, as each process has.
                let store = DirStore::open(&dir).unwrap();
                s.spawn(move || (0..25).for_each(|_| bump(&store)));
            }
        });
        let store = DirStore::open(&dir).unwrap();
        assert_eq!(store.load("n").unwrap().as_deref(), Some(&b"202"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

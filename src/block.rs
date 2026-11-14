//! The block set: one record per blocked key, never the address behind it.
//!
//! A record is the key, its scope (`address` or `domain`), an optional label a person gave it,
//! when it was written and when it expires. Past `expires` a block no longer holds: the door's
//! check answers `NotFound`, and the record is dropped at the next write.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ikigai_core::Time;
use serde_json::{json, Value};

use crate::key::{Key, Scope};
use crate::store::Store;

/// The document the block set is kept in.
const DOCUMENT: &str = "blocks";

/// The most blocks the set holds at once. Past it a new block is refused until one is lifted
/// or expires.
pub const MAX_BLOCKS: usize = 100_000;

/// The longest label a block may carry, in characters.
pub const MAX_LABEL_CHARS: usize = 80;

/// One block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The blocked key.
    pub key: Key,
    /// What the key names.
    pub scope: Scope,
    /// A person's label for it, if they gave one.
    pub label: Option<String>,
    /// When it was written.
    pub created: Time,
    /// When it stops holding.
    pub expires: Time,
}

impl Block {
    /// Whether the block holds at `now`.
    pub fn holds_at(&self, now: Time) -> bool {
        now < self.expires
    }

    fn to_json(&self) -> Value {
        json!({
            "created": self.created.as_millis(),
            "expires": self.expires.as_millis(),
            "key": self.key.as_str(),
            "label": self.label,
            "scope": self.scope.as_str(),
        })
    }

    fn from_json(value: &Value) -> Option<Block> {
        Some(Block {
            key: Key::parse(value.get("key")?.as_str()?).ok()?,
            scope: Scope::parse(value.get("scope")?.as_str()?)?,
            label: match value.get("label")? {
                Value::Null => None,
                label => Some(label.as_str()?.to_string()),
            },
            created: Time::from_millis(value.get("created")?.as_u64()?),
            expires: Time::from_millis(value.get("expires")?.as_u64()?),
        })
    }
}

/// Whether `label` may label a block: 1 to [`MAX_LABEL_CHARS`] characters, none a control.
pub fn check_label(label: &str) -> Result<(), String> {
    let chars = label.chars().count();
    if !(1..=MAX_LABEL_CHARS).contains(&chars) || label.chars().any(char::is_control) {
        return Err(format!(
            "a block's label is 1 to {MAX_LABEL_CHARS} characters, none of them a control \
             character"
        ));
    }
    Ok(())
}

/// The blocks, over the store that keeps them.
pub struct BlockSet {
    store: Arc<dyn Store>,
    blocks: Mutex<BTreeMap<Key, Block>>,
}

impl std::fmt::Debug for BlockSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockSet")
            .field("blocks", &self.blocks.lock().expect("block lock").len())
            .finish()
    }
}

impl BlockSet {
    /// The block set in `store`, empty when it holds none yet. A document that is not a block
    /// set is refused, never read as empty.
    pub fn open(store: Arc<dyn Store>) -> Result<BlockSet, String> {
        let mut blocks = BTreeMap::new();
        if let Some(bytes) = store.load(DOCUMENT)? {
            let refused = || format!("the `{DOCUMENT}` document is not a block set: refused");
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| refused())?;
            for record in value
                .get("blocks")
                .and_then(Value::as_array)
                .ok_or_else(refused)?
            {
                let block = Block::from_json(record).ok_or_else(refused)?;
                blocks.insert(block.key.clone(), block);
            }
        }
        Ok(BlockSet {
            store,
            blocks: Mutex::new(blocks),
        })
    }

    /// The block on `key` that holds at `now`, if any.
    pub fn holding(&self, key: &Key, now: Time) -> Option<Block> {
        self.blocks
            .lock()
            .expect("block lock")
            .get(key)
            .filter(|b| b.holds_at(now))
            .cloned()
    }

    /// Write `block`, replacing any block on its key, and drop every block expired at `now`.
    /// Saved before the answer; nothing changes when the save fails.
    pub fn put(&self, block: Block, now: Time) -> Result<(), String> {
        let mut blocks = self.blocks.lock().expect("block lock");
        let mut next = blocks.clone();
        next.retain(|_, b| b.holds_at(now));
        next.insert(block.key.clone(), block);
        if next.len() > MAX_BLOCKS {
            return Err(format!(
                "the block set holds {MAX_BLOCKS} blocks, its bound: lift some first"
            ));
        }
        self.save(&next)?;
        *blocks = next;
        Ok(())
    }

    /// Lift the block on `key`, answering it, or `None` when there was none (lifting is
    /// idempotent). Expired blocks are dropped too.
    pub fn lift(&self, key: &Key, now: Time) -> Result<Option<Block>, String> {
        let mut blocks = self.blocks.lock().expect("block lock");
        let mut next = blocks.clone();
        let lifted = next.remove(key);
        next.retain(|_, b| b.holds_at(now));
        if next.len() != blocks.len() {
            self.save(&next)?;
            *blocks = next;
        }
        Ok(lifted)
    }

    fn save(&self, blocks: &BTreeMap<Key, Block>) -> Result<(), String> {
        let records: Vec<Value> = blocks.values().map(Block::to_json).collect();
        self.store.save(
            DOCUMENT,
            json!({ "blocks": records }).to_string().as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    fn block(key: char, expires: u64) -> Block {
        Block {
            key: Key::parse(&key.to_string().repeat(64)).unwrap(),
            scope: Scope::Address,
            label: Some("seo".into()),
            created: Time::from_millis(10),
            expires: Time::from_millis(expires),
        }
    }

    #[test]
    fn a_block_holds_until_it_expires_and_survives_a_reopen() {
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let set = BlockSet::open(store.clone()).unwrap();
        let b = block('a', 100);
        set.put(b.clone(), Time::from_millis(10)).unwrap();
        assert_eq!(set.holding(&b.key, Time::from_millis(99)), Some(b.clone()));
        assert_eq!(set.holding(&b.key, Time::from_millis(100)), None);
        let again = BlockSet::open(store).unwrap();
        assert_eq!(again.holding(&b.key, Time::from_millis(50)), Some(b));
    }

    #[test]
    fn a_write_drops_the_expired_and_a_lift_is_idempotent() {
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let set = BlockSet::open(store.clone()).unwrap();
        set.put(block('a', 100), Time::from_millis(10)).unwrap();
        set.put(block('b', 1000), Time::from_millis(200)).unwrap();
        let saved = String::from_utf8(store.load(DOCUMENT).unwrap().unwrap()).unwrap();
        assert!(!saved.contains(&"a".repeat(64)), "{saved}");
        let b = block('b', 1000);
        assert_eq!(
            set.lift(&b.key, Time::from_millis(300)).unwrap(),
            Some(b.clone())
        );
        assert_eq!(set.lift(&b.key, Time::from_millis(300)).unwrap(), None);
    }

    #[test]
    fn a_document_that_is_not_a_block_set_is_refused() {
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        store.save(DOCUMENT, b"{\"blocks\": [{}]}").unwrap();
        assert!(BlockSet::open(store).is_err());
    }
}

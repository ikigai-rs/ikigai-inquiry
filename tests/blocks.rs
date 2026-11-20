//! The block set shared by more than one process: the contact daemon, the HTTP door and a
//! shell's `import-blocks` each open their own `BlockSet` over one store directory.

use std::sync::Arc;

use ikigai_core::Time;
use ikigai_inquiry::{Block, BlockSet, DirStore, Key, Scope, Store};

fn dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ikigai-inquiry-blocks-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn store(dir: &std::path::Path) -> Arc<dyn Store> {
    Arc::new(DirStore::open(dir).unwrap())
}

fn key(n: usize) -> Key {
    Key::parse(&format!("{n:064x}")).unwrap()
}

fn block(n: usize) -> Block {
    Block {
        key: key(n),
        scope: Scope::Address,
        label: None,
        created: Time::from_millis(10),
        expires: Time::from_millis(1_000_000),
    }
}

fn now() -> Time {
    Time::from_millis(20)
}

#[test]
fn a_block_one_process_writes_is_seen_by_the_others_check_without_a_reopen() {
    let dir = dir("seen");
    let (daemon, door) = (
        BlockSet::open(store(&dir)).unwrap(),
        BlockSet::open(store(&dir)).unwrap(),
    );
    assert_eq!(door.holding(&key(1), now()), None);
    daemon.put(block(1), now()).unwrap();
    assert_eq!(door.holding(&key(1), now()), Some(block(1)));
    daemon.lift(&key(1), now()).unwrap();
    assert_eq!(door.holding(&key(1), now()), None);
}

#[test]
fn a_write_never_erases_a_block_another_process_wrote() {
    let dir = dir("erase");
    let (daemon, shell) = (
        BlockSet::open(store(&dir)).unwrap(),
        BlockSet::open(store(&dir)).unwrap(),
    );
    daemon.put(block(1), now()).unwrap();
    shell.put(block(2), now()).unwrap();
    daemon.put(block(3), now()).unwrap();
    let fresh = BlockSet::open(store(&dir)).unwrap();
    for n in 1..=3 {
        assert_eq!(fresh.holding(&key(n), now()), Some(block(n)), "block {n}");
    }
}

#[test]
fn a_lift_from_the_shell_is_not_undone_by_the_daemons_next_write() {
    let dir = dir("lift");
    let (daemon, shell) = (
        BlockSet::open(store(&dir)).unwrap(),
        BlockSet::open(store(&dir)).unwrap(),
    );
    daemon.put(block(1), now()).unwrap();
    assert_eq!(shell.lift(&key(1), now()).unwrap(), Some(block(1)));
    daemon.put(block(2), now()).unwrap();
    let fresh = BlockSet::open(store(&dir)).unwrap();
    assert_eq!(fresh.holding(&key(1), now()), None);
    assert_eq!(fresh.holding(&key(2), now()), Some(block(2)));
}

#[test]
fn concurrent_writers_lose_nothing() {
    let dir = dir("concurrent");
    const WRITERS: usize = 8;
    const EACH: usize = 25;
    std::thread::scope(|s| {
        for w in 0..WRITERS {
            // Each writer its own store and set, as each process has.
            let set = BlockSet::open(store(&dir)).unwrap();
            s.spawn(move || {
                for i in 0..EACH {
                    set.put(block(w * EACH + i), now()).unwrap();
                }
            });
        }
    });
    let fresh = BlockSet::open(store(&dir)).unwrap();
    for n in 0..WRITERS * EACH {
        assert_eq!(fresh.holding(&key(n), now()), Some(block(n)), "block {n}");
    }
}

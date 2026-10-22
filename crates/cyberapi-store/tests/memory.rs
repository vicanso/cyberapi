//! The reference backend against the shared rules.

use cyberapi_store::{MemoryStore, conformance};

#[test]
fn memory_store_conforms() {
    let store = MemoryStore::new();
    pollster::block_on(conformance::run(&store.storage(), &store));
}

#[test]
fn memory_store_backs_up() {
    let store = MemoryStore::new();
    pollster::block_on(conformance::backup(&store.storage(), &store));
}

#[test]
fn memory_store_keeps_copies() {
    let store = MemoryStore::new();
    pollster::block_on(conformance::copies(&store.storage(), &store));
}

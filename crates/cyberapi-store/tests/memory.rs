//! The reference backend against the shared rules.

use cyberapi_store::{MemoryStore, conformance};

#[test]
fn memory_store_conforms() {
    let store = MemoryStore::new();
    pollster::block_on(conformance::run(&store.storage(), &store));
}

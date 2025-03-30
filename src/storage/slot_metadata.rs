use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};
use crate::types::RingSlotStore;

pub struct StorageSlotMetadata {
    slot_num: u64,

}

#[derive(Clone)]
pub struct ShredStore {
    inner: Arc<RingSlotStore<StorageSlotMetadata>>
}
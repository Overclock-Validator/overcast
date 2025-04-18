use heapless::spsc::Queue;
use solana_sdk::packet;
use crate::types::ShredInfo;
use crossbeam_channel::{bounded, Sender, Receiver};

pub(crate) const QUEUE_CAPACITY: usize = 8 * 1024 * 1024 / packet::PACKET_DATA_SIZE;
pub static mut PACKET_QUEUE: Queue<ShredInfo, QUEUE_CAPACITY> = Queue::new();
pub static mut REPAIR_REQUEST_QUEUE: Queue<ShredInfo, QUEUE_CAPACITY> = Queue::new();
pub static mut REPAIR_RESPONSE_QUEUE: Queue<ShredInfo, QUEUE_CAPACITY> = Queue::new();

pub fn get_storage_queue() -> (Sender<ShredInfo>, Receiver<ShredInfo>) {
    bounded(QUEUE_CAPACITY)
}

pub fn get_metadata_queue() -> (Sender<ShredInfo>, Receiver<ShredInfo>) {
    bounded(QUEUE_CAPACITY)
}

pub fn get_repair_queue() -> (Sender<u64>, Receiver<u64>) {
    bounded(QUEUE_CAPACITY)
}

pub fn get_timestamp_updates_queue() -> (tokio::sync::mpsc::Sender<u64>, tokio::sync::mpsc::Receiver<u64>) {
    tokio::sync::mpsc::channel::<u64>(QUEUE_CAPACITY)
}
use heapless::spsc::Queue;
use solana_sdk::packet;
use crate::types::ShredType;
use crossbeam_channel::{bounded, Sender, Receiver};

const QUEUE_CAPACITY: usize = 8 * 1024 * 1024 / packet::PACKET_DATA_SIZE;
pub static mut PACKET_QUEUE: Queue<ShredType, QUEUE_CAPACITY> = Queue::new();
pub static mut REPAIR_MONITOR_QUEUE: Queue<ShredType, QUEUE_CAPACITY> = Queue::new();
pub static mut REPAIR_REQUEST_QUEUE: Queue<ShredType, QUEUE_CAPACITY> = Queue::new();
pub static mut REPAIR_RESPONSE_QUEUE: Queue<ShredType, QUEUE_CAPACITY> = Queue::new();

pub fn get_storage_queue() -> (Sender<ShredType>, Receiver<ShredType>) {
    bounded(QUEUE_CAPACITY)
}
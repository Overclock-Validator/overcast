use solana_sdk::packet;

pub type ShredInfo = (usize, [u8; packet::PACKET_DATA_SIZE]);
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::os::unix::io::AsRawFd;
use std::thread;
use crossbeam_channel::{Receiver, Sender};
use libc;
use heapless::spsc::Queue;
use solana_ledger::shred::Shred;
use solana_sdk::packet;
use crate::queues::PACKET_QUEUE;
use crate::types::ShredType;


const BUFFER_SIZE: usize = packet::PACKET_DATA_SIZE;
const BATCH_SIZE: usize = 32;

pub struct TurbineManager {
    socket: UdpSocket,
}

impl TurbineManager {
    pub fn new(addr: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        Ok(TurbineManager { socket })
    }

    pub fn run(self, storage_sender: Sender<ShredType>) {
        let (mut prod, mut cons) = unsafe { PACKET_QUEUE.split() };

        let receiver_thread = thread::spawn(move || {
            let fd = self.socket.as_raw_fd();

            let mut buffers: [[u8; BUFFER_SIZE]; BATCH_SIZE] = [[0; BUFFER_SIZE]; BATCH_SIZE];
            let mut iovecs: [libc::iovec; BATCH_SIZE] = unsafe { std::mem::zeroed() };
            let mut mmsg_hdrs: [libc::mmsghdr; BATCH_SIZE] = unsafe { std::mem::zeroed() };

            for i in 0..BATCH_SIZE {
                iovecs[i] = libc::iovec {
                    iov_base: buffers[i].as_mut_ptr() as *mut libc::c_void,
                    iov_len: BUFFER_SIZE,
                };

                mmsg_hdrs[i].msg_hdr = libc::msghdr {
                    msg_name: std::ptr::null_mut(),
                    msg_namelen: 0,
                    msg_iov: &mut iovecs[i] as *mut libc::iovec,
                    msg_iovlen: 1,
                    msg_control: std::ptr::null_mut(),
                    msg_controllen: 0,
                    msg_flags: 0,
                };
                mmsg_hdrs[i].msg_len = 0;
            }

            loop {
                // Call recvmmsg to receive a batch of messages.
                let ret = unsafe {
                    libc::recvmmsg(
                        fd,
                        mmsg_hdrs.as_mut_ptr(),
                        BATCH_SIZE as libc::c_uint,
                        0,
                        std::ptr::null_mut(), // no timeout
                    )
                };

                if ret < 0 {
                    let err = io::Error::last_os_error();
                    eprintln!("recvmmsg error: {}", err);
                    break;
                }

                let num_messages = ret as usize;
                for i in 0..num_messages {
                    let packet_len = mmsg_hdrs[i].msg_len as usize;
                    // enqueue / yield / spin
                    while prod.enqueue((packet_len,buffers[i])).is_err() {
                        // spin
                        // std::thread::yield_now();
                    }
                }
            }
        });

        let processor_thread = thread::spawn(move || {
            loop {
                // dequeue / yield / spin
                if let Some(packet) = cons.dequeue() {
                    if let Err(e) = storage_sender.send(packet) {
                        eprintln!("Error sending to storage: {:?}",e);
                    }
                    let shred = Shred::new_from_serialized_shred(packet.1[0..packet.0].to_vec()).unwrap();
                } else {
                    // spin
                    // std::thread::yield_now();
                }
            }
        });

        receiver_thread.join().expect("Receiver thread panicked");
        processor_thread.join().expect("Processor thread panicked");
    }
}

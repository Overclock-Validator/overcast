use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::os::unix::io::AsRawFd;
use std::thread;
use libc;
use heapless::spsc::Queue;
use solana_sdk::packet;

const BUFFER_SIZE: usize = packet::PACKET_DATA_SIZE;
const BATCH_SIZE: usize = 32;
const QUEUE_CAPACITY: usize = 8 * 1024 * 1024 / BUFFER_SIZE;

static mut PACKET_QUEUE: Queue<(usize, [u8; BUFFER_SIZE]), QUEUE_CAPACITY> = Queue::new();

pub struct TurbineManager {
    socket: UdpSocket,
}

impl TurbineManager {
    pub fn new(addr: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        Ok(TurbineManager { socket })
    }

    pub fn run(self) {
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
                    let mut packet = [0u8; BUFFER_SIZE];
                    packet[..packet_len].copy_from_slice(&buffers[i][..packet_len]);
                    // enqueue / yield / spin
                    while prod.enqueue((packet_len, packet)).is_err() {
                        // spin
                        // std::thread::yield_now();
                    }
                }
            }
        });

        let processor_thread = thread::spawn(move || {
            loop {
                // dequeue / yield / spin
                if let Some((packet_len, packet)) = cons.dequeue() {
                    println!("Processing packet of length: {}", packet_len);
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

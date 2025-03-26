use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use crossbeam_channel::{Receiver, Sender};
use libc;
use solana_sdk::packet;
use crate::queues::PACKET_QUEUE;
use crate::types::ShredInfo;


const BUFFER_SIZE: usize = packet::PACKET_DATA_SIZE;
const BATCH_SIZE: usize = 32;

pub struct TurbineManager {
    socket: UdpSocket,
    threads: Vec<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl TurbineManager {
    pub fn new(addr: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        Ok(TurbineManager {
            socket,
            threads: Vec::new(),
            running: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn run(&mut self, storage_sender: Sender<ShredInfo>) -> &mut Self {
        let (mut prod, mut cons) = unsafe { PACKET_QUEUE.split() };
        let socket = self.socket.try_clone().expect("Failed to clone socket");
        let running = self.running.clone();

        self.running.store(true, Ordering::SeqCst);

        let _ = socket.set_nonblocking(true);

        let receiver_thread = thread::spawn(move || {
            let fd = socket.as_raw_fd();

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

            let timeout = libc::timespec {
                tv_sec: 0,
                tv_nsec: 100_000_000, // 100ms
            };

            while running.load(Ordering::SeqCst) {
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
                    if err.kind() != io::ErrorKind::WouldBlock {
                        eprintln!("recvmmsg error: {}", err);
                    }

                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                    continue;
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

        let processor_running = self.running.clone();
        let processor_thread = thread::spawn(move || {
            while processor_running.load(Ordering::SeqCst) {
                // dequeue / yield / spin
                if let Some(packet) = cons.dequeue() {
                    if let Err(e) = storage_sender.send(packet) {
                        eprintln!("Error sending to storage: {:?}",e);
                    }
                } else {
                    if !processor_running.load(Ordering::SeqCst) {
                        break;
                    }
                    // spin
                    // std::thread::yield_now();
                }
            }
        });

        self.threads.push(receiver_thread);
        self.threads.push(processor_thread);

        self
    }

    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Stopping TurbineManager threads...");

        self.running.store(false, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(100));

        while let Some(thread) = self.threads.pop() {
            if let Err(e) = thread.join() {
                eprintln!("Error joining thread: {:?}", e);
            }
        }

        println!("TurbineManager threads stopped");
        Ok(())
    }
}

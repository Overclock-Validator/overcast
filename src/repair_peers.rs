use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};
use solana_gossip::cluster_info::{ClusterInfo, Node, NodeConfig};
use solana_gossip::contact_info::{ContactInfo, Protocol};
use solana_gossip::gossip_service::{make_gossip_node, GossipService};
use solana_net_utils::{get_public_ip_addr, PortRange};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_streamer::socket::SocketAddrSpace;

struct RepairPeerInfo {
    socket_addr: SocketAddr,
    last_seen: SystemTime,
}

pub struct RepairPeersManager {
    repair_peers: Arc<Mutex<HashMap<IpAddr, RepairPeerInfo>>>,
    exit: Arc<AtomicBool>,
    refresh_thread: Option<thread::JoinHandle<()>>,
    gossip_service: Option<GossipService>,
    cluster_info: Option<Arc<ClusterInfo>>,
}

impl RepairPeersManager {
    pub fn new() -> Self {
        RepairPeersManager {
            repair_peers: Arc::new(Mutex::new(HashMap::new())),
            exit: Arc::new(AtomicBool::new(false)),
            refresh_thread: None,
            gossip_service: None,
            cluster_info: None,
        }
    }

    pub fn initialize_gossip(&mut self, entrypoint: &str) -> Result<(), Box<dyn std::error::Error>> {
        let keypair = Arc::new(Keypair::new());
        let exit = Arc::clone(&self.exit);

        let entrypoint_addrs: Vec<_> = entrypoint.to_socket_addrs().unwrap().collect();
        let cluster_entrypoints = entrypoint_addrs
            .iter()
            .map(ContactInfo::new_gossip_entry_point)
            .collect::<Vec<_>>();

        let gossip_entry = entrypoint_addrs.first().ok_or("Failed to resolve entrypoint")?;

        let my_ip = get_public_ip_addr(gossip_entry).map_err(|e| format!("Failed to get public IP: {}", e))?;
        let gossip_addr = SocketAddr::new(my_ip, 65509);
        let node_config = NodeConfig {
            gossip_addr ,
            port_range: (65510,65530),
            bind_ip_addr: my_ip,
            public_tpu_addr: None,
            public_tpu_forwards_addr: None,
            num_tvu_sockets: NonZeroUsize::new(1).unwrap(),
            num_quic_endpoints: NonZeroUsize::new(1).unwrap(),
        };

        let mut node = Node::new_with_external_ip(&keypair.pubkey(), node_config);
        let mut cluster_info = ClusterInfo::new(
            node.info.clone(),
            keypair.clone(),
            SocketAddrSpace::Global,
        );
        cluster_info.set_contact_debug_interval(10_000);
        cluster_info.set_entrypoints(cluster_entrypoints);

        let cluster_info = Arc::new(cluster_info);
        let gossip_service = GossipService::new(
            &cluster_info,
            None,
            node.sockets.gossip,
            None,
            false,
            None,
            exit.clone(),
        );

        self.gossip_service = Some(gossip_service);
        self.cluster_info = Some(cluster_info);

        Ok(())
    }

    pub fn lookup_my_info(&self) -> ContactInfo {
        let cluster_info = Arc::clone(self.cluster_info.as_ref().unwrap());
        cluster_info.my_contact_info()
    }

    pub fn lookup_info(&self, pubkey: &Pubkey) -> Option<ContactInfo> {
        let cluster_info = Arc::clone(self.cluster_info.as_ref().unwrap());
        cluster_info.lookup_contact_info(pubkey, |x|x.clone())
    }

    pub fn start_refresh_thread(&mut self, refresh_interval: u64, stale_threshold: u64) -> Result<(), Box<dyn std::error::Error>> {
        if self.cluster_info.is_none() {
            return Err("Gossip not initialized. Call initialize_gossip first.".into());
        }

        let repair_peers = Arc::clone(&self.repair_peers);
        let exit = Arc::clone(&self.exit);
        let cluster_info = Arc::clone(self.cluster_info.as_ref().unwrap());

        let thread = thread::spawn(move || {
            refresh_repair_peers(refresh_interval, stale_threshold, repair_peers, exit, cluster_info);
        });

        self.refresh_thread = Some(thread);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.exit.store(true, Ordering::Relaxed);

        if let Some(thread) = self.refresh_thread.take() {
            thread.join().map_err(|_| "Failed to join refresh thread")?;
        }

        if let Some(gossip_service) = self.gossip_service.take() {
            gossip_service.join().map_err(|_| "Failed to join gossip service")?;
        }

        Ok(())
    }

    pub fn get_repair_peers(&self) -> Vec<SocketAddr> {
        let peers_lock = self.repair_peers.lock().unwrap();
        peers_lock.values().map(|info| info.socket_addr).collect()
    }

    pub fn print_repair_peers(&self) {
        let peers_lock = self.repair_peers.lock().unwrap();
        println!("Current repair peers:");
        for (ip, info) in peers_lock.iter() {
            let duration = SystemTime::now().duration_since(info.last_seen).unwrap_or(Duration::from_secs(0));
            println!("IP: {}, Address: {:?}, Last seen: {} seconds ago", ip, info.socket_addr, duration.as_secs());
        }
        println!("{} repair peers found", peers_lock.len());
    }
}

fn refresh_repair_peers(
    refresh_interval: u64,
    stale_threshold: u64,
    repair_peers: Arc<Mutex<HashMap<IpAddr, RepairPeerInfo>>>,
    exit: Arc<AtomicBool>,
    cluster_info: Arc<ClusterInfo>
) {
    while !exit.load(Ordering::Relaxed) {
        let peers = cluster_info.all_peers();
        let current_time = SystemTime::now();

        {
            let mut peers_lock = repair_peers.lock().unwrap();

            // stale cleanup
            peers_lock.retain(|_, info| {
                if let Ok(duration) = current_time.duration_since(info.last_seen) {
                    duration.as_secs() < stale_threshold
                } else {
                    true
                }
            });

            // update
            for (peer, _) in peers {
                if let Some(peer_repair_addr) = peer.serve_repair(Protocol::UDP) {
                    let ip = peer_repair_addr.ip();
                    peers_lock.insert(ip, RepairPeerInfo {
                        socket_addr: peer_repair_addr,
                        last_seen: current_time,
                    });
                }
            }

            println!("Refreshed repair peers. Current count: {}", peers_lock.len());
        }

        // sleep
        let mut remaining_sleep = refresh_interval;
        while remaining_sleep > 0 && !exit.load(Ordering::Relaxed) {
            // check exit 5 seconds
            let sleep_chunk = std::cmp::min(remaining_sleep, 5);
            std::thread::sleep(Duration::from_secs(sleep_chunk));
            remaining_sleep -= sleep_chunk;
        }
    }
}

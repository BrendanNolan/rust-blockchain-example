use super::{
    block::Block,
    blockchain::{self, BlockAddStatus, BlockChain},
};
use crate::retry;
use libp2p::{
    core::upgrade,
    floodsub::{Floodsub, FloodsubEvent, Topic},
    identity,
    mdns::{Mdns, MdnsEvent},
    mplex,
    noise::{Keypair, NoiseConfig, X25519Spec},
    swarm::{NetworkBehaviourEventProcess, Swarm, SwarmBuilder},
    tcp::TokioTcpConfig,
    NetworkBehaviour, PeerId, Transport,
};
use log::{debug, error, info};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tokio::sync;

pub static KEYS: Lazy<identity::Keypair> = Lazy::new(identity::Keypair::generate_ed25519);
pub static PEER_ID: Lazy<PeerId> = Lazy::new(|| PeerId::from(KEYS.public()));
pub static CHAIN_TOPIC: Lazy<Topic> = Lazy::new(|| Topic::new("chains"));
pub static BLOCK_TOPIC: Lazy<Topic> = Lazy::new(|| Topic::new("blocks"));

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SerializablePeerId(String);

impl SerializablePeerId {
    pub fn new(id: &PeerId) -> Self {
        Self(id.to_string())
    }

    pub fn id(&self) -> &str {
        &self.0
    }

    fn equal(&self, b: &PeerId) -> bool {
        self.0 == b.to_string()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ChainResponse {
    blocks: Vec<Block>,
    receiver: SerializablePeerId,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChainRequest {
    requestee: SerializablePeerId,
}

#[derive(NetworkBehaviour)]
pub struct AppBehaviour {
    floodsub: Floodsub,
    mdns: Mdns,
    #[behaviour(ignore)]
    pub blockchain: BlockChain,
    #[behaviour(ignore)]
    pub tx_init: Option<sync::mpsc::UnboundedSender<()>>,
    #[behaviour(ignore)]
    pub tx_initialized: Option<sync::mpsc::UnboundedSender<()>>,
    #[behaviour(ignore)]
    pub rx_initialized: Option<sync::mpsc::UnboundedReceiver<()>>,
}

impl AppBehaviour {
    pub async fn new(
        blockchain: BlockChain,
        init: sync::mpsc::UnboundedSender<()>,
        tx_initialized: sync::mpsc::UnboundedSender<()>,
        rx_initialized: sync::mpsc::UnboundedReceiver<()>,
    ) -> Self {
        let mut behaviour = Self {
            blockchain,
            tx_init: Some(init),
            tx_initialized: Some(tx_initialized),
            rx_initialized: Some(rx_initialized),
            floodsub: Floodsub::new(*PEER_ID),
            mdns: Mdns::new(Default::default())
                .await
                .expect("can create mdns"),
        };
        behaviour.floodsub.subscribe(CHAIN_TOPIC.clone());
        behaviour.floodsub.subscribe(BLOCK_TOPIC.clone());
        info!("Subscriptions Made.");
        behaviour
    }
}

pub async fn initialize_swarm(
    init_sender: sync::mpsc::UnboundedSender<()>,
    tx_initialized: sync::mpsc::UnboundedSender<()>,
    rx_initialized: sync::mpsc::UnboundedReceiver<()>,
) -> Swarm<AppBehaviour> {
    let auth_keys = Keypair::<X25519Spec>::new()
        .into_authentic(&KEYS)
        .expect("can create auth keys");

    let transp = TokioTcpConfig::new()
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(auth_keys).into_authenticated())
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    let behaviour = AppBehaviour::new(
        BlockChain::new(),
        init_sender,
        tx_initialized,
        rx_initialized,
    )
    .await;

    let mut swarm = SwarmBuilder::new(transp, behaviour, *PEER_ID)
        .executor(Box::new(|fut| {
            tokio::spawn(fut);
        }))
        .build();
    Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/tcp/0"
            .parse()
            .expect("can get a local socket"),
    )
    .expect("swarm can be started");

    swarm
}

pub fn setup_initial_blockchain(swarm: &mut Swarm<AppBehaviour>) {
    info!("setting up initial blockchain");
    let peers = get_peers(swarm);
    assert!(!peers.is_empty(), "no peers found");
    let blockchain = &mut swarm.behaviour_mut().blockchain;
    if blockchain.is_empty() {
        info!("Performing genesis");
        swarm.behaviour_mut().blockchain.genesis();
    }
    let peer = peers.last().expect("can get peer");
    info!(
        "Will ask one of the connected nodes, {}, for its blockchain.",
        peer.id()
    );
    request_chain(swarm, peer.clone());
    info!("Blockchain requested");
}

#[derive(Serialize, Deserialize)]
enum Publication {
    ChainRequest(ChainRequest),
    ChainResponse(ChainResponse),
    Block(Block),
}

impl NetworkBehaviourEventProcess<FloodsubEvent> for AppBehaviour {
    fn inject_event(&mut self, event: FloodsubEvent) {
        let FloodsubEvent::Message(msg) = event else {
            return;
        };
        let Ok(publication) = serde_json::from_slice::<Publication>(&msg.data) else {
            return;
        };
        match publication {
            Publication::ChainRequest(req) => try_send_chain(self, req, &msg.source),
            Publication::ChainResponse(resp) => {
                if try_accept_chain(self, resp, &msg.source)
                    == ChainAcceptance::AcceptedInitialChain
                {
                    if let Some(tx_initialized) = self.tx_initialized.take() {
                        let _ = tx_initialized.send(());
                    }
                }
            }
            Publication::Block(block) => try_add_new_block(self, block, &msg.source),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ChainAcceptance {
    AcceptedChainUpdate,
    AcceptedInitialChain,
    RejectedChainUpdate,
    ChainIntendedForDifferentPeer,
}

fn try_accept_chain(
    app_behaviour: &mut AppBehaviour,
    resp: ChainResponse,
    source: &PeerId,
) -> ChainAcceptance {
    if resp.receiver != SerializablePeerId::new(&PEER_ID) {
        return ChainAcceptance::ChainIntendedForDifferentPeer;
    }
    info!("Chain response from {}:", source);
    resp.blocks.iter().for_each(|r| info!("{:?}", r));

    if app_behaviour.blockchain.is_genesis_block_only() {
        app_behaviour.blockchain.blocks = resp.blocks;
        return ChainAcceptance::AcceptedInitialChain;
    }

    let new_chain =
        blockchain::choose_longer_valid_chain(&app_behaviour.blockchain.blocks, &resp.blocks)
            .into();

    if new_chain != app_behaviour.blockchain.blocks {
        app_behaviour.blockchain.blocks = new_chain;
        ChainAcceptance::AcceptedChainUpdate
    } else {
        ChainAcceptance::RejectedChainUpdate
    }
}

fn try_send_chain(app_behaviour: &mut AppBehaviour, req: ChainRequest, target: &PeerId) {
    if !req.requestee.equal(&PEER_ID) {
        return;
    }
    info!("sending local chain to {}", target.to_string());
    let response = Publication::ChainResponse(ChainResponse {
        blocks: app_behaviour.blockchain.blocks.clone(),
        receiver: SerializablePeerId::new(target),
    });
    let json_resp = serde_json::to_string(&response).expect("can jsonify response");
    app_behaviour
        .floodsub
        .publish(CHAIN_TOPIC.clone(), json_resp.as_bytes());
}

fn try_add_new_block(app_behaviour: &mut AppBehaviour, block: Block, source: &PeerId) {
    info!("received new block from {}", source.to_string());
    match app_behaviour.blockchain.try_add_block(block) {
        BlockAddStatus::Added => {
            info!("added new block from {}", source.to_string())
        }
        BlockAddStatus::NotAdded => error!(
            "failed to add new block from {} - invalid",
            source.to_string()
        ),
    }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for AppBehaviour {
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            MdnsEvent::Discovered(discovered_addresses) => {
                let discovered_addresses = discovered_addresses
                    .map(|(peer, _addr)| peer)
                    .collect::<Vec<_>>();
                for &peer in &discovered_addresses {
                    self.floodsub.add_node_to_partial_view(peer);
                }
                if discovered_addresses.is_empty() {
                    debug!("No peers discovered");
                    return;
                }
                if let Some(tx_init) = self.tx_init.take() {
                    if let Some(rx_initialized) = self.rx_initialized.take() {
                        tokio::spawn(retry::run_retry_loop(
                            rx_initialized,
                            tx_init,
                            std::time::Duration::from_secs(5),
                        ));
                    }
                }
            }
            MdnsEvent::Expired(expired_addresses) => {
                for (peer, _addr) in expired_addresses {
                    if !self.mdns.has_node(&peer) {
                        self.floodsub.remove_node_from_partial_view(&peer);
                    }
                }
            }
        }
    }
}

pub fn get_peers(swarm: &Swarm<AppBehaviour>) -> Vec<SerializablePeerId> {
    info!("Discovered Peers:");
    let nodes = swarm.behaviour().mdns.discovered_nodes();
    let peers: Vec<_> = nodes
        .collect::<HashSet<_>>()
        .iter()
        .map(|p| SerializablePeerId::new(p))
        .collect();
    peers.iter().for_each(|peer| info!("{}", peer.id()));
    peers
}

pub fn print_peers(swarm: &Swarm<AppBehaviour>) {
    let peers = get_peers(swarm);
    peers.iter().for_each(|p| info!("{}", p.0));
}

pub fn print_chain(swarm: &Swarm<AppBehaviour>) {
    info!("Local Blockchain:");
    let pretty_json = serde_json::to_string_pretty(&swarm.behaviour().blockchain.blocks)
        .expect("can jsonify blocks");
    info!("{}", pretty_json);
}

pub fn request_chain(swarm: &mut Swarm<AppBehaviour>, peer: SerializablePeerId) {
    let req = ChainRequest { requestee: peer };
    let json_req =
        serde_json::to_string(&Publication::ChainRequest(req)).expect("can jsonify request");
    swarm
        .behaviour_mut()
        .floodsub
        .publish(CHAIN_TOPIC.clone(), json_req.as_bytes());
}

pub fn send_block(block: Block, swarm: &mut Swarm<AppBehaviour>) {
    let behaviour = swarm.behaviour_mut();
    let json_block =
        serde_json::to_string(&Publication::Block(block)).expect("can jsonify request");
    info!("broadcasting new block");
    behaviour
        .floodsub
        .publish(BLOCK_TOPIC.clone(), json_block.as_bytes());
}

use super::{
    block::Block,
    blockchain::{self, BlockAddStatus, BlockChain},
};
use libp2p::{
    floodsub::{Floodsub, FloodsubEvent, Topic},
    identity,
    mdns::{Mdns, MdnsEvent},
    swarm::{NetworkBehaviourEventProcess, Swarm},
    NetworkBehaviour, PeerId,
};
use log::{error, info};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tokio::sync::mpsc;

pub static KEYS: Lazy<identity::Keypair> = Lazy::new(identity::Keypair::generate_ed25519);
pub static PEER_ID: Lazy<PeerId> = Lazy::new(|| PeerId::from(KEYS.public()));
pub static CHAIN_TOPIC: Lazy<Topic> = Lazy::new(|| Topic::new("chains"));
pub static BLOCK_TOPIC: Lazy<Topic> = Lazy::new(|| Topic::new("blocks"));

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SerializablePeerId(String);

impl SerializablePeerId {
    fn equal(&self, b: &PeerId) -> bool {
        self.0 == b.to_string()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChainResponse {
    pub blocks: Vec<Block>,
    pub receiver: SerializablePeerId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChainRequest {
    pub from_peer_id: SerializablePeerId,
}

pub enum EventType {
    ChainResponse(ChainResponse),
    Input(String),
    Init,
}

#[derive(NetworkBehaviour)]
pub struct AppBehaviour {
    pub floodsub: Floodsub,
    pub mdns: Mdns,
    #[behaviour(ignore)]
    pub response_sender: mpsc::UnboundedSender<ChainResponse>,
    #[behaviour(ignore)]
    pub init_sender: mpsc::UnboundedSender<()>,
    #[behaviour(ignore)]
    pub blockchain: BlockChain,
}

impl AppBehaviour {
    pub async fn new(
        app: BlockChain,
        response_sender: mpsc::UnboundedSender<ChainResponse>,
        init_sender: mpsc::UnboundedSender<()>,
    ) -> Self {
        let mut behaviour = Self {
            blockchain: app,
            floodsub: Floodsub::new(*PEER_ID),
            mdns: Mdns::new(Default::default())
                .await
                .expect("can create mdns"),
            response_sender,
            init_sender,
        };
        behaviour.floodsub.subscribe(CHAIN_TOPIC.clone());
        behaviour.floodsub.subscribe(BLOCK_TOPIC.clone());
        behaviour
    }
}

#[derive(Serialize, Deserialize)]
pub enum Publication {
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
            Publication::ChainResponse(resp) => try_accept_chain(self, resp, &msg.source),
            Publication::Block(block) => try_add_new_block(self, block, &msg.source),
        }
    }
}

fn try_accept_chain(app_behaviour: &mut AppBehaviour, resp: ChainResponse, source: &PeerId) {
    if resp.receiver != SerializablePeerId(PEER_ID.to_string()) {
        return;
    }
    info!("Response from {}:", source);
    resp.blocks.iter().for_each(|r| info!("{:?}", r));

    app_behaviour.blockchain.blocks =
        blockchain::choose_longer_valid_chain(&app_behaviour.blockchain.blocks, &resp.blocks)
            .into();
}

fn try_send_chain(app_behaviour: &mut AppBehaviour, req: ChainRequest, target: &PeerId) {
    if !req.from_peer_id.equal(&PEER_ID) {
        return;
    }
    info!("sending local chain to {}", target.to_string());
    let response = ChainResponse {
        blocks: app_behaviour.blockchain.blocks.clone(),
        receiver: SerializablePeerId(target.to_string()),
    };
    if let Err(e) = app_behaviour.response_sender.send(response) {
        error!("error sending response via channel, {}", e);
    }
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
                for (peer, _addr) in discovered_addresses {
                    self.floodsub.add_node_to_partial_view(peer);
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
    nodes
        .collect::<HashSet<_>>()
        .iter()
        .map(|p| SerializablePeerId(p.to_string()))
        .collect()
}

pub fn handle_print_peers(swarm: &Swarm<AppBehaviour>) {
    let peers = get_peers(swarm);
    peers.iter().for_each(|p| info!("{}", p.0));
}

pub fn handle_print_chain(swarm: &Swarm<AppBehaviour>) {
    info!("Local Blockchain:");
    let pretty_json = serde_json::to_string_pretty(&swarm.behaviour().blockchain.blocks)
        .expect("can jsonify blocks");
    info!("{}", pretty_json);
}

pub fn handle_create_block(cmd: &str, swarm: &mut Swarm<AppBehaviour>) {
    let Some(data) = cmd.strip_prefix("create b") else {
        return;
    };
    let behaviour = swarm.behaviour_mut();
    let latest_block = behaviour
        .blockchain
        .blocks
        .last()
        .expect("there is at least one block");
    let block = Block::new(
        latest_block.id + 1,
        latest_block.hash.clone(),
        data.to_owned(),
    );
    let json_block =
        serde_json::to_string(&Publication::Block(block.clone())).expect("can jsonify request");
    behaviour.blockchain.blocks.push(block);
    info!("broadcasting new block");
    behaviour
        .floodsub
        .publish(BLOCK_TOPIC.clone(), json_block.as_bytes());
}

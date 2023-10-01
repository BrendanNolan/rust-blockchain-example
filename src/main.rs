use crate::p2p::AppBehaviour;
use libp2p::{futures::StreamExt, swarm::Swarm};
use log::{error, info};
use tokio::{
    io::{stdin, AsyncBufReadExt, BufReader},
    select,
};

mod block;
mod blockchain;
mod p2p;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    info!("Peer Id: {}", p2p::PEER_ID.clone());

    let mut swarm = p2p::initialize_swarm().await;
    setup_initial_blockchain(&mut swarm);

    let mut stdin = BufReader::new(stdin()).lines();
    loop {
        select! {
            line = stdin.next_line() => execute_user_command(&line.unwrap().unwrap(), &mut swarm),
            _ = drive_forward(&mut swarm) => {},
        }
    }
}

fn execute_user_command(line: &str, swarm: &mut Swarm<AppBehaviour>) {
    match line {
        "ls p" => p2p::print_peers(swarm),
        cmd if cmd.starts_with("ls c") => p2p::print_chain(swarm),
        cmd if cmd.starts_with("create b") => {
            if let Some(data) = cmd.strip_prefix("create b") {
                let new_block = swarm
                    .behaviour_mut()
                    .blockchain
                    .mine_block_return_mined_clone(data);
                p2p::send_block(new_block, swarm);
            }
        }
        _ => error!("unknown command"),
    }
}

fn setup_initial_blockchain(swarm: &mut Swarm<AppBehaviour>) {
    let peers = p2p::get_peers(swarm);
    swarm.behaviour_mut().blockchain.genesis();
    info!("connected nodes: {}", peers.len());
    if !peers.is_empty() {
        let last_peer = peers.last().expect("can get last peer");
        p2p::request_chain(swarm, last_peer.clone());
    }
}

async fn drive_forward(swarm: &mut Swarm<AppBehaviour>) {
    let event = swarm.select_next_some().await;
    info!(
        "Drove swarm forward by selecting next event: {:?}. But did not handle that event.",
        event
    );
}

use crate::p2p::AppBehaviour;
use libp2p::{futures::StreamExt, swarm::Swarm};
use log::{debug, error, info};
use tokio::{
    io::{stdin, AsyncBufReadExt, BufReader},
    select, sync,
};

mod block;
mod blockchain;
mod p2p;
mod retry;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    info!("Peer Id: {}", p2p::PEER_ID.clone());

    let (init_sender, mut init_receiver) = sync::mpsc::unbounded_channel::<()>();
    let (tx_initialized, rx_initialized) = sync::mpsc::unbounded_channel::<()>();
    let mut swarm = p2p::initialize_swarm(init_sender, tx_initialized, rx_initialized).await;

    let mut stdin = BufReader::new(stdin()).lines();

    loop {
        select! {
            Some(()) = init_receiver.recv() => {
                p2p::setup_initial_blockchain(&mut swarm);
            },
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
                p2p::broadcast_block(new_block, swarm);
            }
        }
        _ => error!("unknown command"),
    }
}

async fn drive_forward(swarm: &mut Swarm<AppBehaviour>) {
    let event = swarm.select_next_some().await;
    debug!("Drove swarm: event {:?}", event);
}

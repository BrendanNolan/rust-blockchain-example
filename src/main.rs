use crate::blockchain::BlockChain;
use libp2p::{
    core::upgrade,
    futures::StreamExt,
    mplex,
    noise::{Keypair, NoiseConfig, X25519Spec},
    swarm::{Swarm, SwarmBuilder},
    tcp::TokioTcpConfig,
    Transport,
};
use log::{error, info};
use std::time::Duration;
use tokio::{
    io::{stdin, AsyncBufReadExt, BufReader},
    select,
    sync::mpsc,
    time::sleep,
};

mod block;
mod blockchain;
mod p2p;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    info!("Peer Id: {}", p2p::PEER_ID.clone());
    let (init_sender, mut init_rcv) = mpsc::unbounded_channel();

    let auth_keys = Keypair::<X25519Spec>::new()
        .into_authentic(&p2p::KEYS)
        .expect("can create auth keys");

    let transp = TokioTcpConfig::new()
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(auth_keys).into_authenticated())
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    let behaviour = p2p::AppBehaviour::new(BlockChain::new()).await;

    let mut swarm = SwarmBuilder::new(transp, behaviour, *p2p::PEER_ID)
        .executor(Box::new(|fut| {
            tokio::spawn(fut);
        }))
        .build();

    let mut stdin = BufReader::new(stdin()).lines();

    Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/tcp/0"
            .parse()
            .expect("can get a local socket"),
    )
    .expect("swarm can be started");

    tokio::spawn(async move {
        sleep(Duration::from_secs(1)).await;
        info!("sending init event");
        init_sender.send(()).expect("can send init event");
    });

    loop {
        select! {
            line = stdin.next_line() => {
                let line = line.unwrap().unwrap();
                match line.as_str() {
                    "ls p" => p2p::print_peers(&swarm),
                    cmd if cmd.starts_with("ls c") => p2p::print_chain(&swarm),
                    cmd if cmd.starts_with("create b") => {
                        if let Some(data) = cmd.strip_prefix("create b") {
                            let new_block = swarm.behaviour_mut().blockchain.mine_block_return_mined_clone(data);
                            p2p::send_block(new_block, &mut swarm);
                        }
                    }
                    _ => error!("unknown command"),
                }
            }
            _ = init_rcv.recv() => {
                let peers = p2p::get_peers(&swarm);
                swarm.behaviour_mut().blockchain.genesis();

                info!("connected nodes: {}", peers.len());
                if !peers.is_empty() {
                    let last_peer = peers.last().expect("can get last peer");
                    p2p::request_chain(&mut swarm, last_peer.clone());
                }
            }
            event = swarm.select_next_some() => {
                info!("Unhandled Swarm Event: {:?}", event);
                continue;
            },
        }
    }
}

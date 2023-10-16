use crate::block::{self, Block};
use log::info;

pub struct BlockChain {
    pub blocks: Vec<Block>,
}

#[derive(PartialEq, Eq)]
pub enum BlockAddStatus {
    Added,
    NotAdded,
}

impl BlockChain {
    pub fn new() -> Self {
        Self { blocks: vec![] }
    }

    pub fn genesis(&mut self) {
        let genesis_block = Block {
            id: 0,
            timestamp: 0,
            previous_hash: String::from("genesis"),
            data: String::from("genesis!"),
            nonce: 2836,
            hash: "0000f816a87f806bb0073dcf026a64fb40c946b5abee2573702828694d5b4c43".to_string(),
        };
        assert!(self.blocks.is_empty());
        self.blocks.push(genesis_block);
    }

    pub fn is_genesis_block_only(&self) -> bool {
        self.blocks.len() == 1
            && self.blocks[0].previous_hash == "genesis"
            && self.blocks[0].data == "genesis!"
            && self.blocks[0].id == 0
    }

    pub fn try_add_block(&mut self, new_block: Block) -> BlockAddStatus {
        let latest_block = self.blocks.last().expect("there is at least one block");
        if block::new_block_valid(&new_block, latest_block) {
            self.blocks.push(new_block);
            BlockAddStatus::Added
        } else {
            BlockAddStatus::NotAdded
        }
    }

    pub fn mine_block(&mut self, data: &str) {
        let latest_block = self.blocks.last().expect("there is at least one block");
        let block = Block::mine(
            latest_block.id + 1,
            latest_block.hash.clone(),
            data.to_owned(),
        );
        self.blocks.push(block);
    }

    pub fn mine_block_return_mined_clone(&mut self, data: &str) -> Block {
        self.mine_block(data);
        self.blocks.last().unwrap().clone()
    }
}

pub fn choose_longer_valid_chain<'a>(local: &'a [Block], remote: &'a [Block]) -> &'a [Block] {
    let is_local_valid = is_valid(local);
    let is_remote_valid = is_valid(remote);

    if is_local_valid && is_remote_valid {
        if local.len() >= remote.len() {
            info!("Local chain longer");
            local
        } else {
            info!("Remote chain longer");
            remote
        }
    } else if is_remote_valid && !is_local_valid {
        info!("Remote chain longer");
        remote
    } else if !is_remote_valid && is_local_valid {
        info!("Local chain longer");
        local
    } else {
        panic!("local and remote chains are both invalid");
    }
}

fn is_valid(chain: &[Block]) -> bool {
    chain
        .windows(2)
        .all(|pair| block::new_block_valid(&pair[1], &pair[0]))
}

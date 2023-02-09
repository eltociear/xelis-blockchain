use crate::{storage::Storage, core::blockchain::Blockchain};
use super::{RpcError, RpcServer};
use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use xelis_common::{
    api::daemon::{
        BlockType,
        BlockResponse,
        DataHash,
        GetBlockAtTopoHeightParams,
        GetBlockByHashParams,
        GetBlockTemplateParams,
        GetBlockTemplateResult,
        SubmitBlockParams,
        GetBalanceParams,
        GetNonceParams,
        SubmitTransactionParams,
        GetTransactionParams,
        P2pStatusResult,
        GetBlocksAtHeightParams,
        GetDagOrderParams, GetBalanceAtTopoHeightParams, GetLastBalanceResult
    },
    async_handler,
    serializer::Serializer,
    transaction::Transaction,
    crypto::hash::Hash,
    block::Block,
};
use std::sync::Arc;
use log::{info, debug};

fn parse_params<P: DeserializeOwned>(value: Value) -> Result<P, RpcError> {
    serde_json::from_value(value).map_err(|e| RpcError::InvalidParams(e))
}

async fn get_block_type_for_block(blockchain: &Blockchain, storage: &Storage, hash: &Hash) -> Result<BlockType, RpcError> {
    Ok(if blockchain.is_block_orphaned_for_storage(storage, hash).await {
        BlockType::Orphaned
    } else if blockchain.is_block_sync(storage, hash).await? {
        BlockType::Sync
    } else if blockchain.is_side_block(storage, hash).await? {
        BlockType::Side
    } else {
        BlockType::Normal
    })
}

async fn get_block_response_for_hash(blockchain: &Blockchain, storage: &Storage, hash: Hash) -> Result<Value, RpcError> {
    let block = storage.get_block_by_hash(&hash).await?;
    let topoheight = if storage.is_block_topological_ordered(&hash).await {
        Some(storage.get_topo_height_for_hash(&hash).await?)
    } else {
        None
    };
    let block_type = get_block_type_for_block(&blockchain, &storage, &hash).await?;
    let cumulative_difficulty = storage.get_cumulative_difficulty_for_block(&hash).await?;
    let difficulty = storage.get_difficulty_for_block(&hash)?;
    let supply = storage.get_supply_for_hash(&hash)?;
    let reward = storage.get_block_reward(&hash)?;

    Ok(json!(BlockResponse { topoheight, block_type, cumulative_difficulty, difficulty, supply, reward, data: DataHash { hash, data: block } }))
}

pub fn register_methods(server: &mut RpcServer) {
    info!("Registering RPC methods...");
    server.register_method("get_height", async_handler!(get_height));
    server.register_method("get_topoheight", async_handler!(get_topoheight));
    server.register_method("get_stableheight", async_handler!(get_stableheight));
    server.register_method("get_block_template", async_handler!(get_block_template));
    server.register_method("get_block_at_topoheight", async_handler!(get_block_at_topoheight));
    server.register_method("get_blocks_at_height", async_handler!(get_blocks_at_height));
    server.register_method("get_block_by_hash", async_handler!(get_block_by_hash));
    server.register_method("get_top_block", async_handler!(get_top_block));
    server.register_method("submit_block", async_handler!(submit_block));
    server.register_method("get_last_balance", async_handler!(get_last_balance));
    server.register_method("get_balance_at_topoheight", async_handler!(get_balance_at_topoheight));
    server.register_method("get_nonce", async_handler!(get_nonce));
    server.register_method("get_assets", async_handler!(get_assets));
    server.register_method("count_transactions", async_handler!(count_transactions));
    server.register_method("submit_transaction", async_handler!(submit_transaction));
    server.register_method("get_transaction", async_handler!(get_transaction));
    server.register_method("p2p_status", async_handler!(p2p_status));
    server.register_method("get_mempool", async_handler!(get_mempool));
    server.register_method("get_tips", async_handler!(get_tips));
    server.register_method("get_dag_order", async_handler!(get_dag_order));
}

async fn get_height(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }
    Ok(json!(blockchain.get_height()))
}

async fn get_topoheight(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }
    Ok(json!(blockchain.get_topo_height()))
}

async fn get_stableheight(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }

    Ok(json!(blockchain.get_stable_height()))
}

async fn get_block_at_topoheight(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetBlockAtTopoHeightParams = parse_params(body)?;
    let storage = blockchain.get_storage().read().await;
    let hash = storage.get_hash_at_topo_height(params.topoheight).await?;
    get_block_response_for_hash(&blockchain, &storage, hash).await
}

async fn get_block_by_hash(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetBlockByHashParams = parse_params(body)?;
    let storage = blockchain.get_storage().read().await;
    get_block_response_for_hash(&blockchain, &storage, params.hash).await
}

async fn get_top_block(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }
    let storage = blockchain.get_storage().read().await;
    let hash = blockchain.get_top_block_hash_for_storage(&storage).await?;
    get_block_response_for_hash(&blockchain, &storage, hash).await
}

async fn get_block_template(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetBlockTemplateParams = parse_params(body)?;
    if !params.address.is_normal() {
        return Err(RpcError::ExpectedNormalAddress)
    }
    let storage = blockchain.get_storage().read().await;
    let block = blockchain.get_block_template_for_storage(&storage, params.address.to_public_key()).await?;
    let difficulty = blockchain.get_difficulty_at_tips(&storage, block.get_tips()).await?;
    Ok(json!(GetBlockTemplateResult { template: block.to_hex(), difficulty }))
}

async fn submit_block(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: SubmitBlockParams = parse_params(body)?;
    let block = Block::from_hex(params.block_template)?;
    // TODO add block hashing blob on block template
    let complete_block = blockchain.build_complete_block_from_block(block).await?;
    blockchain.add_new_block(complete_block, true).await?;
    Ok(json!(true))
}

async fn get_last_balance(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetBalanceParams = parse_params(body)?;
    let storage = blockchain.get_storage().read().await;
    let (topoheight, balance) = storage.get_last_balance(params.address.get_public_key(), &params.asset).await?;
    Ok(json!(GetLastBalanceResult {
        balance,
        topoheight
    }))
}

async fn get_balance_at_topoheight(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetBalanceAtTopoHeightParams = parse_params(body)?;
    let topoheight = blockchain.get_topo_height();
    if params.topoheight > topoheight {
        return Err(RpcError::UnexpectedParams).context("Topoheight cannot be greater than current chain topoheight")?
    }

    let storage = blockchain.get_storage().read().await;
    let balance = storage.get_balance_at_exact_topoheight(params.address.get_public_key(), &params.asset, params.topoheight).await?;
    Ok(json!(balance))
}

async fn get_nonce(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetNonceParams = parse_params(body)?;
    let storage = blockchain.get_storage().read().await;
    let nonce = storage.get_nonce(params.address.get_public_key()).await?;
    Ok(json!(nonce))
}

// TODO Rate limiter
async fn get_assets(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }

    let storage = blockchain.get_storage().read().await;
    let assets = storage.get_assets().await?;
    Ok(json!(assets))
}

// TODO Rate limiter
async fn count_transactions(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }
    let storage = blockchain.get_storage().read().await;
    Ok(json!(storage.count_transactions()))
}

async fn submit_transaction(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: SubmitTransactionParams = parse_params(body)?;
    let transaction = Transaction::from_hex(params.data)?;
    blockchain.add_tx_to_mempool(transaction, true).await?;
    Ok(json!(true))
}

async fn get_transaction(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetTransactionParams = parse_params(body)?;
    let storage = blockchain.get_storage().read().await;
    let tx = storage.get_transaction(&params.hash).await?;
    Ok(json!(tx))
}

async fn p2p_status(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }

    let p2p = blockchain.get_p2p().lock().await;
    match p2p.as_ref() {
        Some(p2p) => {
            let peer_count = p2p.get_peer_count().await;
            let tag = p2p.get_tag();
            let peer_id = p2p.get_peer_id();
            let best_height = p2p.get_best_height().await;
            let max_peers = p2p.get_max_peers();
            let our_height = blockchain.get_height();

            Ok(json!(P2pStatusResult {
                peer_count,
                tag: tag.clone(),
                peer_id,
                our_height,
                best_height,
                max_peers
            }))
        },
        None => return Err(RpcError::NoP2p)
    }
}

async fn get_mempool(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }

    let mempool = blockchain.get_mempool().read().await;
    let mut transactions: Vec<DataHash<Arc<Transaction>>> = Vec::new();
    for tx in mempool.get_sorted_txs() {
        let transaction = mempool.view_tx(tx.get_hash())?;
        transactions.push(DataHash { hash: tx.get_hash().clone(), data: transaction });
    }

    Ok(json!(transactions))
}

async fn get_blocks_at_height(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetBlocksAtHeightParams = parse_params(body)?;
    let storage = blockchain.get_storage().read().await;

    let mut blocks = Vec::new();
    for hash in storage.get_blocks_at_height(params.height).await? {
        blocks.push(get_block_response_for_hash(&blockchain, &storage, hash).await?)
    }
    Ok(json!(blocks))
}

async fn get_tips(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    if body != Value::Null {
        return Err(RpcError::UnexpectedParams)
    }
    let storage = blockchain.get_storage().read().await;
    let tips = storage.get_tips().await?;
    Ok(json!(tips))
}

const MAX_DAG_ORDER: u64 = 64;
// get dag order based on params
// if no params found, get order of last 64 blocks
async fn get_dag_order(blockchain: Arc<Blockchain>, body: Value) -> Result<Value, RpcError> {
    let params: GetDagOrderParams = parse_params(body)?;

    let current_topoheight = blockchain.get_topo_height();
    let start_topoheight = params.start_topoheight.unwrap_or_else(|| {
        if params.end_topoheight.is_none() && current_topoheight > MAX_DAG_ORDER {
            current_topoheight - MAX_DAG_ORDER
        } else {
            0
        }
    });

    let end_topoheight = params.end_topoheight.unwrap_or(current_topoheight);
    if end_topoheight < start_topoheight || end_topoheight > current_topoheight {
        debug!("get dag order range: start = {}, end = {}, max = {}", start_topoheight, end_topoheight, current_topoheight);
        return Err(RpcError::InvalidRequest)
    }

    let count = end_topoheight - start_topoheight;
    if count > MAX_DAG_ORDER { // only retrieve max 64 blocks hash per request
        debug!("get dag order requested count: {}", count);
        return Err(RpcError::InvalidRequest) 
    }

    let storage = blockchain.get_storage().read().await;
    let mut order = Vec::with_capacity(count as usize);
    for i in start_topoheight..=end_topoheight {
        let hash = storage.get_hash_at_topo_height(i).await?;
        order.push(hash);
    }

    Ok(json!(order))
}
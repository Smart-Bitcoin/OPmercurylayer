mod db;

use bitcoin::{Address, Network, Txid};
use secp256k1_zkp::{SecretKey, PublicKey, schnorr::Signature};
use sqlx::Sqlite;

use crate::electrum;

pub struct CoinKeyDetails {
    pub new_tx_n: u32,
    pub client_seckey: SecretKey,
    pub client_pubkey: PublicKey,
    pub amount: u64,
    pub server_pubkey: PublicKey,
    pub aggregated_pubkey: PublicKey,
    pub p2tr_agg_address: Address,
    pub auth_seckey: SecretKey,
    pub signed_statechain_id: Signature,
    pub utxo_tx_hash: Txid,
    pub utxo_vout: u32,
}

pub async fn execute(pool: &sqlx::Pool<Sqlite>, statechain_id: &str, to_address: &Address, fee_rate: u64, network: Network) -> String {
    
    let client = electrum_client::Client::new("tcp://127.0.0.1:50001").unwrap();

    let block_header = electrum::block_headers_subscribe_raw(&client);
    let block_height = block_header.height;

    let coin_key_details = db::get_coin_and_key_info(pool, statechain_id, network).await;
    
    let (tx, client_pub_nonce, server_pub_nonce, blinding_factor) = crate::transaction::new_backup_transaction(
        pool,         
        block_height as u32,
        &statechain_id,
        &coin_key_details.signed_statechain_id,
        &coin_key_details.client_seckey,
        &coin_key_details.client_pubkey,
        &coin_key_details.server_pubkey,
        coin_key_details.utxo_tx_hash, 
        coin_key_details.utxo_vout, 
        &coin_key_details.aggregated_pubkey, 
        &coin_key_details.p2tr_agg_address.script_pubkey(), 
        coin_key_details.amount,
        &to_address).await.unwrap();

    let tx_bytes = bitcoin::consensus::encode::serialize(&tx);

    crate::deposit::db::insert_transaction(
        pool,
        coin_key_details.new_tx_n,
        &tx_bytes, 
        &client_pub_nonce.serialize(), 
        &server_pub_nonce.serialize(), 
        &coin_key_details.client_pubkey, 
        &coin_key_details.server_pubkey, 
        blinding_factor.as_bytes(), 
        &statechain_id, 
        &to_address.to_string()
    ).await.unwrap();

    let client = electrum_client::Client::new("tcp://127.0.0.1:50001").unwrap();

    let txid = electrum::transaction_broadcast_raw(&client, &tx_bytes);

    txid.to_string()
    
}
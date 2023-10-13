pub mod db;

use std::{str::FromStr, thread, time::Duration};

use bitcoin::{Network, secp256k1, hashes::sha256, Address, Txid};
use electrum_client::ListUnspentRes;
use secp256k1_zkp::{Secp256k1, Message, PublicKey, musig::MusigKeyAggCache, schnorr::Signature};
use serde::{Serialize, Deserialize};
use sqlx::Sqlite;

use crate::{key_derivation, error::CError, electrum};

#[derive(Debug, Serialize, Deserialize)]
pub struct DepositRequestPayload {
    amount: u64,
    auth_key: String,
    token_id: String,
    signed_token_id: String,
}

pub async fn execute(pool: &sqlx::Pool<Sqlite>, token_id: uuid::Uuid, amount: u64, network: Network) -> Result<String, CError> {

    let address_data = key_derivation::get_new_address(pool, Some(token_id), Some(amount), network).await;

    let (statechain_id, 
        server_pubkey_share, 
        signed_statechain_id) = init(&address_data, token_id, amount).await;
    
    let secp = Secp256k1::new();

    let aggregate_pubkey = address_data.client_pubkey_share.combine(&server_pubkey_share).unwrap();

    let aggregated_xonly_pubkey = aggregate_pubkey.x_only_public_key().0; 

    let aggregate_address = Address::p2tr(&secp, aggregated_xonly_pubkey, None, network);

    db::insert_agg_pub_key(
        pool, 
        &token_id,
        &statechain_id, 
        amount as u32, 
        &server_pubkey_share, 
        &aggregate_pubkey, 
        &aggregate_address,
        &address_data.client_pubkey_share,
        &signed_statechain_id).await.unwrap();

    let client = electrum_client::Client::new("tcp://127.0.0.1:50001").unwrap();

    println!("address: {}", aggregate_address.to_string());

    println!("waiting for deposit ....");

    let delay = Duration::from_secs(5);

    let mut utxo: Option<ListUnspentRes> = None;

    loop {
        let utxo_list = electrum::get_script_list_unspent(&client, &aggregate_address);

        for unspent in utxo_list {
            if unspent.value == amount {
                utxo = Some(unspent);
                break;
            }
        }

        if utxo.is_some() {
            break;
        }

        thread::sleep(delay);
    }

    let utxo = utxo.unwrap();

    db::update_funding_tx_outpoint(pool, &utxo.tx_hash, utxo.tx_pos as u32, &statechain_id).await;

    let block_header = electrum::block_headers_subscribe_raw(&client);
    let block_height = block_header.height;

    let to_address = address_data.backup_address;

    let (tx, client_pub_nonce, server_pub_nonce, blinding_factor) = crate::transaction::new_backup_transaction(
        pool,         
        block_height as u32,
        &statechain_id,
        &signed_statechain_id,
        &address_data.client_secret_key,
        &address_data.client_pubkey_share,
        &server_pubkey_share,
        utxo.tx_hash, 
        utxo.tx_pos as u32, 
        &aggregate_pubkey, 
        &aggregate_address.script_pubkey(), 
        amount,
        &to_address,
        false,).await.unwrap();

    let lock_time = tx.lock_time.to_consensus_u32();

    db::update_locktime(pool, &statechain_id, lock_time).await;

    let tx_bytes = bitcoin::consensus::encode::serialize(&tx);

    db::insert_transaction(
        pool, 1, 
        &tx_bytes, 
        &client_pub_nonce.serialize(), 
        &server_pub_nonce.serialize(), 
        &address_data.client_pubkey_share, &server_pubkey_share, 
        blinding_factor.as_bytes(), 
        &statechain_id, 
        &address_data.transfer_address
    ).await.unwrap();

    Ok(statechain_id)

}

pub async fn init(address_data: &key_derivation::AddressData ,token_id: uuid::Uuid, amount: u64) -> (String, PublicKey, Signature) {

    let msg = Message::from_hashed_data::<sha256::Hash>(token_id.to_string().as_bytes());

    let secp = Secp256k1::new();
    let auth_secret_key = address_data.auth_secret_key;
    let keypair = secp256k1::KeyPair::from_seckey_slice(&secp, auth_secret_key.as_ref()).unwrap();
    let signed_token_id = secp.sign_schnorr(&msg, &keypair);
    
    let deposit_request_payload = DepositRequestPayload {
        amount,
        auth_key: address_data.auth_xonly_pubkey.to_string(),
        token_id: token_id.to_string(),
        signed_token_id: signed_token_id.to_string(),
    };

    let endpoint = "http://127.0.0.1:8000";
    let path = "deposit/init/pod";

    let client: reqwest::Client = reqwest::Client::new();
    let request = client.post(&format!("{}/{}", endpoint, path));

    let value = match request.json(&deposit_request_payload).send().await {
        Ok(response) => {
            let text = response.text().await.unwrap();
            text
        },
        Err(err) => {
            // return Err(CError::Generic(err.to_string()));
            panic!("error: {}", err);
        },
    };

    #[derive(Serialize, Deserialize)]
    pub struct PublicNonceRequestPayload<'r> {
        server_pubkey: &'r str,
        statechain_id: &'r str,
    }

    let response: PublicNonceRequestPayload = serde_json::from_str(value.as_str()).expect(&format!("failed to parse: {}", value.as_str()));

    let server_pubkey_share = PublicKey::from_str(&response.server_pubkey).unwrap();

    let statechain_id = response.statechain_id.to_string();

    let msg = Message::from_hashed_data::<sha256::Hash>(statechain_id.to_string().as_bytes());
    let signed_statechain_id = secp.sign_schnorr(&msg, &keypair);

    (statechain_id, server_pubkey_share, signed_statechain_id)
}

pub async fn broadcast_backup_tx(pool: &sqlx::Pool<Sqlite>, statechain_id: &str) -> Txid {
    
    let tx_bytes = db::get_backup_tx(pool, statechain_id).await;

    let client = electrum_client::Client::new("tcp://127.0.0.1:50001").unwrap();
    
    let txid = electrum::transaction_broadcast_raw(&client, &tx_bytes);

    txid
}

use std::{collections::{HashMap, HashSet}, str::FromStr};

use crate::{sqlite_manager::{get_wallet, update_wallet, insert_or_update_backup_txs}, client_config::ClientConfig, utils};
use anyhow::{anyhow, Ok, Result};
use bitcoin::{Txid, Address};
use chrono::Utc;
use electrum_client::ElectrumApi;
use mercurylib::{utils::{get_network, InfoConfig}, wallet::{Activity, Coin, CoinStatus}};
use reqwest::StatusCode;

pub async fn new_transfer_address(client_config: &ClientConfig, wallet_name: &str) -> Result<String>{

    let wallet = get_wallet(&client_config.pool, &wallet_name).await?;
    
    let mut wallet = wallet.clone();

    let coin = wallet.get_new_coin()?;

    wallet.coins.push(coin.clone());

    update_wallet(&client_config.pool, &wallet).await?;

    Ok(coin.address)
}

pub struct TransferReceiveResult {
    pub is_there_batch_locked: bool,
    pub received_statechain_ids: Vec<String>,
}

pub async fn execute(client_config: &ClientConfig, wallet_name: &str) -> Result<TransferReceiveResult>{

    let mut wallet = get_wallet(&client_config.pool, &wallet_name).await?;

    let info_config = utils::info_config(&client_config).await.unwrap();

    let mut unique_auth_pubkeys: HashSet<String> = HashSet::new();
    
    for coin in wallet.coins.iter() {
        unique_auth_pubkeys.insert(coin.auth_pubkey.clone());
    }

    let mut enc_msgs_per_auth_pubkey: HashMap<String, Vec<String>> = HashMap::new();

    for auth_pubkey in unique_auth_pubkeys {

        let enc_messages = get_msg_addr(&auth_pubkey, &client_config).await?;
        if enc_messages.len() == 0 {
            continue;
        }

        enc_msgs_per_auth_pubkey.insert(auth_pubkey.clone(), enc_messages);
    }

    let mut is_there_batch_locked = false;

    let mut received_statechain_ids =  Vec::<String>::new();

    let mut temp_coins = wallet.coins.clone();
    let mut temp_activities = wallet.activities.clone();

    for (key, values) in &enc_msgs_per_auth_pubkey {

        let auth_pubkey = key.clone();

        for enc_message in values {

            let coin: Option<&mut Coin> = temp_coins.iter_mut().find(|coin| coin.auth_pubkey == auth_pubkey && coin.status == CoinStatus::INITIALISED);

            if coin.is_some() {

                let mut coin = coin.unwrap();

                let message_result = process_encrypted_message(client_config, &mut coin, enc_message, &wallet.network, &info_config, &mut temp_activities).await;

                if message_result.is_err() {
                    println!("Error: {}", message_result.err().unwrap().to_string());
                    continue;
                }

                let message_result = message_result.unwrap();

                if message_result.is_batch_locked {
                    is_there_batch_locked = true;
                }

                if message_result.statechain_id.is_some() {
                    received_statechain_ids.push(message_result.statechain_id.unwrap());
                }

            } else {

                let new_coin = mercurylib::transfer::receiver::duplicate_coin_to_initialized_state(&wallet, &auth_pubkey);

                if new_coin.is_err() {
                    println!("Error: {}", new_coin.err().unwrap().to_string());
                    continue;
                }

                let mut new_coin = new_coin.unwrap();

                let message_result = process_encrypted_message(client_config, &mut new_coin, enc_message, &wallet.network, &info_config, &mut temp_activities).await;

                if message_result.is_err() {
                    println!("Error: {}", message_result.err().unwrap().to_string());
                    continue;
                }

                temp_coins.push(new_coin);

                let message_result = message_result.unwrap();

                if message_result.is_batch_locked {
                    is_there_batch_locked = true;
                }

                if message_result.statechain_id.is_some() {
                    received_statechain_ids.push(message_result.statechain_id.unwrap());
                }
            }
        }
    }

    wallet.coins = temp_coins.clone();
    wallet.activities = temp_activities.clone();

    update_wallet(&client_config.pool, &wallet).await?;

    Ok(TransferReceiveResult{
        is_there_batch_locked,
        received_statechain_ids
    })
}

async fn get_msg_addr(auth_pubkey: &str, client_config: &ClientConfig) -> Result<Vec<String>> {

    let path = format!("transfer/get_msg_addr/{}", auth_pubkey.to_string());

    let client = client_config.get_reqwest_client()?;
    let request = client.get(&format!("{}/{}", client_config.statechain_entity, path));

    let value = request.send().await?.text().await?;

    let response: mercurylib::transfer::receiver::GetMsgAddrResponsePayload = serde_json::from_str(value.as_str())?;

    Ok(response.list_enc_transfer_msg)
}

pub struct MessageResult {
    pub is_batch_locked: bool,
    pub statechain_id: Option<String>,
}

async fn process_encrypted_message(client_config: &ClientConfig, coin: &mut Coin, enc_message: &str, network: &str, info_config: &InfoConfig, activities: &mut Vec<Activity>) -> Result<MessageResult> {

    let client_auth_key = coin.auth_privkey.clone();
    let new_user_pubkey = coin.user_pubkey.clone();

    let transfer_msg = mercurylib::transfer::receiver::decrypt_transfer_msg(enc_message, &client_auth_key)?;

    let tx0_outpoint = mercurylib::transfer::receiver::get_tx0_outpoint(&transfer_msg.backup_transactions)?;
    
    let tx0_hex = get_tx0(&client_config.electrum_client, &tx0_outpoint.txid).await?;

    let is_transfer_signature_valid = mercurylib::transfer::receiver::verify_transfer_signature(&new_user_pubkey, &tx0_outpoint, &transfer_msg)?; 

    if !is_transfer_signature_valid {
        return Err(anyhow::anyhow!("Invalid transfer signature".to_string()));
    }

    let statechain_info = utils::get_statechain_info(&transfer_msg.statechain_id, &client_config).await?;

    if statechain_info.is_none() {
        return Err(anyhow::anyhow!("Statechain info not found".to_string()));
    }

    let statechain_info = statechain_info.unwrap();

    let is_tx0_output_pubkey_valid = mercurylib::transfer::receiver::validate_tx0_output_pubkey(&statechain_info.enclave_public_key, &transfer_msg, &tx0_outpoint, &tx0_hex, network)?;

    if !is_tx0_output_pubkey_valid {
        return Err(anyhow::anyhow!("Invalid tx0 output pubkey".to_string()));
    }

    let latest_backup_tx_pays_to_user_pubkey = mercurylib::transfer::receiver::verify_latest_backup_tx_pays_to_user_pubkey(&transfer_msg, &new_user_pubkey, network)?;

    if !latest_backup_tx_pays_to_user_pubkey {
        return Err(anyhow::anyhow!("Latest Backup Tx does not pay to the expected public key".to_string()));
    }

    if statechain_info.num_sigs != transfer_msg.backup_transactions.len() as u32 {
        return Err(anyhow::anyhow!("num_sigs is not correct".to_string()));
    }

    let (is_tx0_output_unspent, tx0_status) = verify_tx0_output_is_unspent_and_confirmed(&client_config.electrum_client, &tx0_outpoint, &tx0_hex, &network, client_config.confirmation_target).await?;

    if !is_tx0_output_unspent {
        return Err(anyhow::anyhow!("tx0 output is spent or not confirmed".to_string()));
    }

    let current_fee_rate_sats_per_byte = if info_config.fee_rate_sats_per_byte > client_config.max_fee_rate {
        client_config.max_fee_rate
    } else {
        info_config.fee_rate_sats_per_byte
    };

    let previous_lock_time = mercurylib::transfer::receiver::validate_signature_scheme(
        &transfer_msg, 
        &statechain_info, 
        &tx0_hex, 
        client_config.fee_rate_tolerance, 
        current_fee_rate_sats_per_byte,
        info_config.interval);

    if previous_lock_time.is_err() {
        let error = previous_lock_time.err().unwrap();
        return Err(anyhow!("Signature scheme validation failed. Error {}", error.to_string()));
    }

    let previous_lock_time = previous_lock_time.unwrap();

    let transfer_receiver_request_payload = mercurylib::transfer::receiver::create_transfer_receiver_request_payload(&statechain_info, &transfer_msg, &coin)?;

    // unlock the statecoin - it might be part of a batch

    // the pub_auth_key has not been updated yet in the server (it will be updated after the transfer/receive call)
    // So we need to manually sign the statechain_id with the client_auth_key
    let signed_statechain_id_for_unlock = mercurylib::transfer::receiver::sign_message(&transfer_msg.statechain_id, &coin)?;

    unlock_statecoin(&client_config, &transfer_msg.statechain_id, &signed_statechain_id_for_unlock, &coin.auth_pubkey).await?;

    let transfer_receiver_result = send_transfer_receiver_request_payload(&client_config, &transfer_receiver_request_payload).await;

    let server_public_key_hex = match transfer_receiver_result {
        std::result::Result::Ok(server_public_key_hex) => {

            if server_public_key_hex.is_batch_locked {
                return Ok(MessageResult {
                    is_batch_locked: true,
                    statechain_id: None,
                });
            }

            server_public_key_hex.server_pubkey.unwrap()
        },
        Err(err) => {
            return Err(anyhow::anyhow!("Error: {}", err.to_string()));
        }
    };

    let new_key_info = mercurylib::transfer::receiver::get_new_key_info(&server_public_key_hex, &coin, &transfer_msg.statechain_id, &tx0_outpoint, &tx0_hex, network)?;

    coin.server_pubkey = Some(server_public_key_hex);
    coin.aggregated_pubkey = Some(new_key_info.aggregate_pubkey);
    coin.aggregated_address = Some(new_key_info.aggregate_address);
    coin.statechain_id = Some(transfer_msg.statechain_id.clone());
    coin.signed_statechain_id = Some(new_key_info.signed_statechain_id.clone());
    coin.amount = Some(new_key_info.amount);
    coin.utxo_txid = Some(tx0_outpoint.txid.clone());
    coin.utxo_vout = Some(tx0_outpoint.vout);
    coin.locktime = Some(previous_lock_time);
    coin.status = tx0_status;

    let date = Utc::now(); // This will get the current date and time in UTC
    let iso_string = date.to_rfc3339(); // Converts the date to an ISO 8601 string

    let activity = Activity {
        utxo: tx0_outpoint.txid.clone(),
        amount: new_key_info.amount,
        action: "Receive".to_string(),
        date: iso_string
    };

    activities.push(activity);

    insert_or_update_backup_txs(&client_config.pool, &transfer_msg.statechain_id, &transfer_msg.backup_transactions).await?;

    Ok(MessageResult {
        is_batch_locked: false,
        statechain_id: Some(transfer_msg.statechain_id.clone()),
    })

    // Ok(transfer_msg.statechain_id.clone())
}

async fn get_tx0(electrum_client: &electrum_client::Client, tx0_txid: &str) -> Result<String> {

    let tx0_txid = Txid::from_str(tx0_txid)?;
    let tx_bytes = electrum_client.batch_transaction_get_raw(&[tx0_txid])?;

    if tx_bytes.len() == 0 {
        return Err(anyhow!("tx0 not found"));
    }

    // let tx0 = bitcoin::consensus::encode::deserialize(&tx_bytes[0])?;

    let tx0_hex = hex::encode(&tx_bytes[0]);

    Ok(tx0_hex)
}

async fn verify_tx0_output_is_unspent_and_confirmed(electrum_client: &electrum_client::Client, tx0_outpoint: &mercurylib::transfer::receiver::TxOutpoint, tx0_hex: &str, network: &str, confirmation_target: u32) -> Result<(bool, CoinStatus)> {
    let output_address = mercurylib::transfer::receiver::get_output_address_from_tx0(&tx0_outpoint, &tx0_hex, &network)?;

    let network = get_network(&network)?;
    let address = Address::from_str(&output_address)?.require_network(network)?;
    let script = address.script_pubkey();
    let script = script.as_script();

    let res = electrum_client.script_list_unspent(script)?;

    let block_header = electrum_client.block_headers_subscribe_raw()?;
    let blockheight = block_header.height;

    let mut status = CoinStatus::UNCONFIRMED;

    for unspent in res {
        if (unspent.tx_hash.to_string() == tx0_outpoint.txid) && (unspent.tx_pos as u32 == tx0_outpoint.vout) {
            let confirmations = blockheight - unspent.height + 1;

            if confirmations as u32 >= confirmation_target {
                status = CoinStatus::CONFIRMED;
            }

            return Ok((true, status));
        }
    }

    Ok((false, status))
}

async fn unlock_statecoin(client_config: &ClientConfig, statechain_id: &str, signed_statechain_id: &str, auth_pubkey: &str) -> Result<()> {

    let path = "transfer/unlock";

    let client = client_config.get_reqwest_client()?;
    let request = client.post(&format!("{}/{}", client_config.statechain_entity, path));

    let transfer_unlock_request_payload = mercurylib::transfer::receiver::TransferUnlockRequestPayload {
        statechain_id: statechain_id.to_string(),
        auth_sig: signed_statechain_id.to_string(),
        auth_pub_key: Some(auth_pubkey.to_string()),
    };

    let status = request.json(&transfer_unlock_request_payload).send().await?.status();

    if !status.is_success() {
        return Err(anyhow::anyhow!("Failed to update transfer message".to_string()));
    }

    Ok(())
}

pub struct TransferReceiveRequestResult {
    pub is_batch_locked: bool,
    pub server_pubkey: Option<String>,
}

async fn send_transfer_receiver_request_payload(client_config: &ClientConfig, transfer_receiver_request_payload: &mercurylib::transfer::receiver::TransferReceiverRequestPayload) -> Result<TransferReceiveRequestResult>{

    let path = "transfer/receiver";

    let client = client_config.get_reqwest_client()?;

        let request: reqwest::RequestBuilder = client.post(&format!("{}/{}", client_config.statechain_entity, path));

        let response = request.json(&transfer_receiver_request_payload).send().await?;

        let status = response.status();

        let value = response.text().await?;

        if status == StatusCode::BAD_REQUEST{

            let error: mercurylib::transfer::receiver::TransferReceiverErrorResponsePayload = serde_json::from_str(value.as_str())?;

            match error.code {
                mercurylib::transfer::receiver::TransferReceiverError::ExpiredBatchTimeError => {
                    return Err(anyhow::anyhow!(error.message));
                },
                mercurylib::transfer::receiver::TransferReceiverError::StatecoinBatchLockedError => {
                    return Ok(TransferReceiveRequestResult {
                        is_batch_locked: true,
                        server_pubkey: None,
                    });
                },
            }
        }

        if status == StatusCode::OK {
            let response: mercurylib::transfer::receiver::TransferReceiverPostResponsePayload = serde_json::from_str(value.as_str())?;
            return Ok(TransferReceiveRequestResult {
                is_batch_locked: false,
                server_pubkey: Some(response.server_pubkey)
            });
        } else {
            return Err(anyhow::anyhow!("{}: {}", "Failed to update transfer message".to_string(), value));
        }
    
}
use std::str::FromStr;

use bitcoin::{PrivateKey, Transaction, hashes::sha256, Txid};
use secp256k1_zkp::{PublicKey, schnorr::Signature, Secp256k1, Message};
use serde::{Serialize, Deserialize};
use anyhow::{Result, anyhow};

use crate::wallet::BackupTx;

use super::TransferMsg;

#[derive(Debug, Serialize, Deserialize)]
pub struct TransferReceiverRequestPayload { 
    pub statechain_id: String,
    pub batch_data: Option<String>,
    pub t2: String,
    pub auth_sig: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyUpdateResponsePayload { 
    pub statechain_id: String,
    pub t2: String,
    pub x1: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetMsgAddrResponsePayload {
    pub list_enc_transfer_msg: Vec<String>,
}

pub fn decrypt_transfer_msg(encrypted_message: &str, private_key_wif: &str) -> Result<TransferMsg> {

    let client_auth_key = PrivateKey::from_wif(private_key_wif)?.inner;

    let decoded_enc_message = hex::decode(encrypted_message)?;

    let decrypted_msg = ecies::decrypt(client_auth_key.secret_bytes().as_slice(), decoded_enc_message.as_slice()).unwrap();

    let decrypted_msg_str = String::from_utf8(decrypted_msg).unwrap();

    let transfer_msg: TransferMsg = serde_json::from_str(decrypted_msg_str.as_str()).unwrap();

    Ok(transfer_msg)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TxOutpoint {
    pub txid: String,
    pub vout: u32,
}

pub fn get_tx0_outpoint(backup_transactions: &Vec<BackupTx>) -> Result<TxOutpoint> {

    let mut backup_transactions = backup_transactions.clone();

    backup_transactions.sort_by(|a, b| a.tx_n.cmp(&b.tx_n));

    let bkp_tx1 = backup_transactions.first().ok_or(anyhow!("No backup transaction found"))?;

    let tx1: Transaction = bitcoin::consensus::encode::deserialize(&hex::decode(&bkp_tx1.tx)?)?;

    if tx1.input.len() > 1 {
        return Err(anyhow!("tx1 has more than one input"));
    }

    if tx1.output.len() > 1 {
        return Err(anyhow!("tx1 has more than one output"));
    }

    let tx0_txid = tx1.input[0].previous_output.txid;
    let tx0_vout = tx1.input[0].previous_output.vout as u32;

    Ok(TxOutpoint{ txid: tx0_txid.to_string(), vout: tx0_vout })
}

pub fn verify_transfer_signature(new_user_pubkey: &str, tx0_outpoint: &TxOutpoint, transfer_msg: &TransferMsg) -> Result<bool> {

    let new_user_pubkey = PublicKey::from_str(new_user_pubkey)?;
    let sender_public_key = PublicKey::from_str(&transfer_msg.user_public_key)?.x_only_public_key().0;

    let input_vout = tx0_outpoint.vout;
    let input_txid = Txid::from_str(&tx0_outpoint.txid)?;

    let signature = Signature::from_str(&transfer_msg.transfer_signature)?;

    let secp = Secp256k1::new();

    let mut data_to_verify = Vec::<u8>::new();
    data_to_verify.extend_from_slice(&input_txid[..]);
    data_to_verify.extend_from_slice(&input_vout.to_le_bytes());
    data_to_verify.extend_from_slice(&new_user_pubkey.serialize()[..]);

    let msg = Message::from_hashed_data::<sha256::Hash>(&data_to_verify);

    Ok(secp.verify_schnorr(&signature, &msg, &sender_public_key).is_ok())

}

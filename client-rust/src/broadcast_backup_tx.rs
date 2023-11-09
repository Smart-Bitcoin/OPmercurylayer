use crate::{client_config::ClientConfig, sqlite_manager::{get_backup_txs, get_wallet, update_wallet}};
use anyhow::{anyhow, Result};
use electrum_client::ElectrumApi;
use mercury_lib::wallet::cpfp_tx;

pub async fn execute(client_config: &ClientConfig, wallet_name: &str, statechain_id: &str, to_address: &str, fee_rate: Option<u64>) -> Result<()> {
    
    let mut wallet: mercury_lib::wallet::Wallet = get_wallet(&client_config.pool, &wallet_name).await?;
    // transaction::broadcast_backup_transaction(&client_config, &wallet).await?;

    let backup_txs = get_backup_txs(&client_config.pool, &statechain_id).await?;
    
    let backup_tx = backup_txs.iter().max_by_key(|tx| tx.tx_n);

    if backup_tx.is_none() {
        return Err(anyhow!("No backup transaction found"));
    }

    let backup_tx = backup_tx.unwrap();

    let coin = wallet.coins.iter_mut().find(|tx| tx.statechain_id == Some(statechain_id.to_string()));

    if coin.is_none() {
        return Err(anyhow!("No coin found"));
    }

    let coin = coin.unwrap();

    let fee_rate = match fee_rate {
        Some(fee_rate) => fee_rate,
        None => {
            let fee_rate_btc_per_kb = client_config.electrum_client.estimate_fee(1)?;
            let fee_rate_sats_per_byte = (fee_rate_btc_per_kb * 100000.0) as u64;
            fee_rate_sats_per_byte
        },
    };

    let cpfp_tx = cpfp_tx::create(&backup_tx, &coin, to_address, fee_rate, &wallet.network)?;

    let tx_bytes = hex::decode(&backup_tx.tx)?;
    let txid = client_config.electrum_client.transaction_broadcast_raw(&tx_bytes)?;
    println!("Broadcasting backup transaction: {}", txid);

    let tx_bytes = hex::decode(&cpfp_tx)?;
    let txid = client_config.electrum_client.transaction_broadcast_raw(&tx_bytes)?;
    println!("Broadcasting CPFP transaction: {}", txid);

    coin.tx_cpfp = Some(txid.to_string());

    update_wallet(&client_config.pool, &wallet).await?;

    Ok(())
}
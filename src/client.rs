use std::{thread::sleep, time::Duration};

use bdk_wallet::bitcoin::Amount;
use bitcoincore_rpc::{Auth, Client, RpcApi};

pub fn bitcoind_client(wallet: &str) -> Result<Client, bitcoincore_rpc::Error> {
    let auth = Auth::UserPass("local".to_string(), "local".to_string());
    let mut bitcoind = Client::new("http://0.0.0.0:38332", auth.clone())?;
    let _ = bitcoind
        .create_wallet(wallet, None, None, None, None)
        .map_err(|_| println!("ERROR(create_wallet)"));
    bitcoind = Client::new(
        format!("http://0.0.0.0:38332/wallet/{}", wallet).as_str(),
        auth,
    )?;
    Ok(bitcoind)
}

pub fn fund_client(
    bitcoind: &Client,
    receiver: &Client,
    amount: Amount,
    utxos: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..utxos {
        let address = receiver.get_new_address(None, None)?.assume_checked();
        bitcoind.send_to_address(&address, amount, None, None, None, None, None, None)?;
    }
    Ok(())
}

pub fn get_client_balance(bitcoind: &Client) -> Result<Amount, Box<dyn std::error::Error>> {
    let balance = bitcoind.get_balances()?.mine;
    let total_balance = balance.trusted + balance.untrusted_pending;
    Ok(total_balance)
}

pub fn wait_for_block(bitcoind: &Client, blocks: u64) -> Result<(), Box<dyn std::error::Error>> {
    let initial_block = bitcoind.get_block_count()?;
    let target_block = initial_block + blocks;
    loop {
        let block_num = bitcoind.get_block_count()?;
        if block_num >= target_block {
            break;
        }
        println!("    -> Block {:?} [target={:?}]", block_num, target_block);
        sleep(Duration::from_secs(11));
    }
    Ok(())
}

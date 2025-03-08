mod client;
mod batch;
mod node;
mod payjoin;
mod wallet;

use std::env;

use bdk_wallet::bitcoin::Amount;

use client::{bitcoind_client, fund_client, get_client_balance, wait_for_block};
use batch::methods;
use node::{payjoin_batch, payjoin_open_channel};
use payjoin::{direct::direct_payjoin, payjoin_v1::do_payjoin_v1, payjoin_v2::do_payjoin_v2};
use wallet::{create_wallet, fund_wallet, sync_wallet, wallet_total_balance};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    let mut op = "directly";
    if args.len() >= 2 {
        op = &args[1];
    }

    let mut sub_op = "1";
    if args.len() >= 3 {
        sub_op = &args[2];
    }

    let miner = bitcoind_client("miner").unwrap();

    let sender_seed = &[0u8; 64];
    let receiver_seed = &[1u8; 64];

    let mut funded = false;
    let amount_to_send: Amount = Amount::from_sat(100_000);

    if op == "ldk" {
        payjoin_batch(&miner)?;
    } else if op == "ldk-open-channel" {
        payjoin_open_channel(&miner)?;
    } else if op == "batch" {
        match sub_op {
            "1" => methods::method_1(&miner)?,
            "2" => methods::method_2(&miner)?,
            "3" => methods::method_3(&miner)?,
            "4" => methods::method_4(&miner)?,
            "5" => methods::method_5(&miner)?,
            "6" => methods::method_6(&miner)?,
            _ => println!("ERROR(batch(method)): Invalid method!"),
        }
    } else if op == "directly" {
        // Direct Payjoin (bdk_wallet only)
        println!("===== Payjoin Directly =====");
        let mut sender = create_wallet(sender_seed)?;
        let mut receiver = create_wallet(receiver_seed)?;

        if wallet_total_balance(&miner, &mut sender)? < amount_to_send {
            match fund_wallet(&miner, &mut sender, Amount::from_sat(1_000_000), 25) {
                Ok(_) => {}
                Err(err) => println!("ERROR(fund_wallet(sender)): {:?}", err),
            };
            funded = true;
        }

        if wallet_total_balance(&miner, &mut receiver)? < amount_to_send {
            match fund_wallet(&miner, &mut receiver, Amount::from_sat(500_000), 25) {
                Ok(_) => {}
                Err(err) => println!("ERROR(fund_wallet(receiver)): {:?}", err),
            };
            funded = true;
        }

        if funded {
            wait_for_block(&miner, 2)?;
        }

        sync_wallet(&miner, &mut sender, funded)?;
        sync_wallet(&miner, &mut receiver, funded)?;

        direct_payjoin(&miner, &mut sender, &mut receiver, amount_to_send)?;
    } else {
        println!("===== Payjoin V1/V2 =====");
        let sender = bitcoind_client("sender").unwrap();
        let receiver = bitcoind_client("receiver").unwrap();

        if get_client_balance(&sender)? < amount_to_send {
            match fund_client(&miner, &sender, Amount::from_sat(1_000_000), 25) {
                Ok(_) => {}
                Err(err) => println!("ERROR(fund_client(sender)): {:?}", err),
            };
            funded = true;
        }

        if get_client_balance(&receiver)? < amount_to_send {
            match fund_client(&miner, &receiver, Amount::from_sat(500_000), 25) {
                Ok(_) => {}
                Err(err) => println!("ERROR(fund_client(receiver)): {:?}", err),
            };
            funded = true;
        }

        if funded {
            wait_for_block(&miner, 2)?;
        }

        if op == "v1" {
            // Payjoin V1 (rust-payjoin)
            println!("===== V1 =====");
            do_payjoin_v1(&sender, &receiver, amount_to_send, false)?;
        } else if op == "v2" {
            // Payjoin V2 (rust-payjoin)
            println!("===== V2 =====");
            do_payjoin_v2(&sender, &receiver, amount_to_send).await?;
        }
    }

    Ok(())
}

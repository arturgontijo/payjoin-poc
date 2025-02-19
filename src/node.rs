use std::thread::sleep;
use std::time::Duration;

use bitcoincore_rpc::{Client, RpcApi};
use ldk_node::config::Config;
use ldk_node::lightning::ln::msgs::SocketAddress;
use ldk_node::lightning::routing::gossip::NodeAlias;
use ldk_node::Builder;

use ldk_node::bitcoin::{Amount, Network};

// use ldk_node::bitcoin::{
//     policy::DEFAULT_MIN_RELAY_TX_FEE,
//     FeeRate, locktime::absolute::LockTime,
// };

use crate::client::wait_for_block;

const CHANNEL_READY_CONFIRMATION_BLOCKS: u64 = 6;

fn get_config(node_alias: &str, port_in: u16, port_out: u16) -> Config {
    let mut config = Config::default();

    config.network = Network::Signet;
    println!("Setting network: {}", config.network);

    let rand_dir = format!("data/{}", node_alias);
    println!("Setting random LDK storage dir: {}", rand_dir);
    config.storage_dir_path = rand_dir;

    let address: Vec<SocketAddress> = vec![
        format!("0.0.0.0:{}", port_in).parse().unwrap(),
        format!("0.0.0.0:{}", port_out).parse().unwrap(),
    ];
    println!("Setting random LDK listening addresses: {:?}", address);
    config.listening_addresses = Some(address);

    let alias = format!("ldk-node-{}", node_alias);
    let mut bytes = [0u8; 32];
    bytes[..alias.as_bytes().len()].copy_from_slice(alias.as_bytes());

    println!("Setting random LDK node alias: {:?}", alias);
    config.node_alias = Some(NodeAlias(bytes));

    config
}

pub fn run_nodes(
    bitcoind: &Client,
    seed_a_bytes: &[u8; 64],
    seed_b_bytes: &[u8; 64],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = Builder::from_config(get_config("nodeA", 7777, 7778));
    builder.set_chain_source_bitcoind_rpc(
        "0.0.0.0".to_string(),
        38332,
        "local".to_string(),
        "local".to_string(),
    );

    builder.set_entropy_seed_bytes(seed_a_bytes.to_vec())?;

    let node_a = builder.build().unwrap();

    let mut builder = Builder::from_config(get_config("nodeB", 8888, 8889));
    builder.set_chain_source_bitcoind_rpc(
        "0.0.0.0".to_string(),
        38332,
        "local".to_string(),
        "local".to_string(),
    );

    builder.set_entropy_seed_bytes(seed_b_bytes.to_vec())?;

    let node_b = builder.build().unwrap();

    println!("[LDK-Node Payjoin] Starting NodeA and NodeB...");
    node_a.start().unwrap();
    node_b.start().unwrap();

    let node_a_address = node_a.onchain_payment().new_address()?;
    println!("[LDK-Node Payjoin] node_a_address: {:?}", node_a_address);

    let node_b_address = node_b.onchain_payment().new_address()?;
    println!("[LDK-Node Payjoin] node_b_address: {:?}", node_b_address);

    let amount = Amount::from_sat(10_000_000);

    println!(
        "[LDK-Node Payjoin] NodeA({:?}) onchain balance: {:?}",
        node_a_address,
        node_a.list_balances().spendable_onchain_balance_sats
    );
    if node_a.list_balances().spendable_onchain_balance_sats < amount.to_sat() {
        let txid = bitcoind
            .send_to_address(&node_a_address, amount, None, None, None, None, None, None)
            .unwrap();
        println!(
            "  -> Funding NodeA({:?}) with {:?} -> txid: {:?}",
            node_a_address,
            amount.to_btc(),
            txid
        );
        loop {
            sleep(Duration::from_secs(10));
            println!(
                "[LDK-Node Payjoin] NodeA({:?}) sync_wallets()",
                node_a_address
            );
            node_a.sync_wallets().unwrap();
            if node_a.list_balances().spendable_onchain_balance_sats >= amount.to_sat() {
                break;
            }
        }
    }

    //   // ----- Payjoin ------------------------------------------------------------------------------------------
    // println!("[LDK-Node Payjoin] Sending some UTXOs to the nodes...");
    //   let amount = Amount::from_sat(1_000_000);
    //   bitcoind.send_to_address(&node_a_address, amount, None, None, None, None, None, None).unwrap();
    //   bitcoind.send_to_address(&node_a_address, amount, None, None, None, None, None, None).unwrap();
    //   bitcoind.send_to_address(&node_a_address, amount, None, None, None, None, None, None).unwrap();

    //   bitcoind.send_to_address(&node_b_address, amount, None, None, None, None, None, None).unwrap();
    //   bitcoind.send_to_address(&node_b_address, amount, None, None, None, None, None, None).unwrap();
    //   bitcoind.send_to_address(&node_b_address, amount, None, None, None, None, None, None).unwrap();

    //   println!("[LDK-Node Payjoin] sync_wallets()...");
    //   sleep(Duration::from_secs(12));
    //   node_a.sync_wallets().unwrap();
    //   node_b.sync_wallets().unwrap();
    //   println!("[LDK-Node Payjoin] sync_wallets() -> Done");
    //   let fee_rate = FeeRate::from_sat_per_vb(DEFAULT_MIN_RELAY_TX_FEE as u64).unwrap();
    //   let locktime = LockTime::ZERO;
    //   let mut psbt = node_a.payjoin_open_channel(
    //     node_b_address.script_pubkey(),
    //     amount,
    //     fee_rate,
    //     locktime,
    //   ).unwrap();

    //   println!("[LDK-Node Payjoin] PSBT(inputs.len): {}", psbt.inputs.len());
    //   println!("[LDK-Node Payjoin] PSBT(outputs.len): {}", psbt.outputs.len());

    //   println!("[LDK-Node Payjoin] Adding NodeB UTXOs...");
    //   node_b.payjoin_add_utxos_to_psbt(&mut psbt)?;

    //   println!("[LDK-Node Payjoin] PSBT(inputs.len): {}", psbt.inputs.len());
    //   println!("[LDK-Node Payjoin] PSBT(outputs.len): {}", psbt.outputs.len());

    //   println!("[LDK-Node Payjoin] NodeA signing...");
    //   node_a.payjoin_sign_psbt(&mut psbt)?;
    //   println!("[LDK-Node Payjoin] NodeB signing...");
    //   node_b.payjoin_sign_psbt(&mut psbt)?;

    //   println!("[LDK-Node Payjoin] Extracting PSBT tx...");
    //   let tx = psbt.extract_tx()?;
    //   println!("[LDK-Node Payjoin] Extracting PSBT tx: {}", tx.compute_txid());

    //   // TODO: Use this transaction as a channel funding transaction (?!)

    //   // Dummy: Manually send it really just to validate it.
    //   println!("[LDK-Node Payjoin] Sending Tx...");
    //   let txid = bitcoind.send_raw_transaction(&tx).unwrap();
    //   println!("[LDK-Node Payjoin] Txid: {}", txid);
    //   // --------------------------------------------------------------------------------------------------------

    println!(
        "[LDK-Node Payjoin] NodeA({:?}) (pre-channel): {:?} sats",
        node_a_address,
        node_a.list_balances().spendable_onchain_balance_sats
    );
    let channel_id = node_a
        .open_channel(
            node_b.node_id(),
            node_b
                .listening_addresses()
                .unwrap()
                .first()
                .unwrap()
                .clone(),
            1_000_000,
            None,
            None,
        )
        .unwrap();
    println!("[LDK-Node Payjoin] channel_id(A <-> B): {:?}", channel_id);
    assert!(node_a
        .list_peers()
        .iter()
        .find(|c| { c.node_id == node_b.node_id() })
        .is_some());

    wait_for_block(&bitcoind, CHANNEL_READY_CONFIRMATION_BLOCKS).unwrap();

    println!(
        "[LDK-Node Payjoin] NodeA({:?}) sync_wallets()",
        node_a_address
    );
    node_a.sync_wallets().unwrap();
    println!(
        "[LDK-Node Payjoin] NodeB({:?}) sync_wallets()",
        node_b_address
    );
    node_b.sync_wallets().unwrap();

    println!(
        "[LDK-Node Payjoin] NodeA({:?}) (pos-channel): {:?} sats",
        node_a_address,
        node_a.list_balances().spendable_onchain_balance_sats
    );

    println!("[LDK-Node Payjoin] Stopping NodeA and NodeB...");
    node_a.stop().unwrap();
    node_b.stop().unwrap();

    Ok(())
}

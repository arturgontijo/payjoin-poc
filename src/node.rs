use std::thread::sleep;
use std::time::Duration;

use bitcoincore_rpc::{Client, RpcApi};

use ldk_node::bitcoin::{
    locktime::absolute::LockTime, policy::DEFAULT_MIN_RELAY_TX_FEE, Amount, FeeRate, Network,
};
use ldk_node::config::Config;
use ldk_node::lightning::ln::msgs::SocketAddress;
use ldk_node::lightning::ln::types::ChannelId;
use ldk_node::lightning::routing::gossip::NodeAlias;
use ldk_node::lightning_invoice::{Bolt11InvoiceDescription, Description};
use ldk_node::Builder;
use ldk_node::LightningBalance::ClaimableAwaitingConfirmations;

use crate::client::wait_for_block;

const CHANNEL_READY_CONFIRMATION_BLOCKS: u64 = 6;

fn get_config(
    node_alias: &str,
    port_in: u16,
    port_out: u16,
) -> Result<Config, Box<dyn std::error::Error>> {
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

    Ok(config)
}

pub fn run_nodes(
    bitcoind: &Client,
    seed_a_bytes: &[u8; 64],
    seed_b_bytes: &[u8; 64],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = Builder::from_config(get_config("nodeA", 7777, 7778)?);
    builder.set_chain_source_bitcoind_rpc(
        "0.0.0.0".to_string(),
        38332,
        "local".to_string(),
        "local".to_string(),
    );

    builder.set_entropy_seed_bytes(seed_a_bytes.to_vec())?;

    let node_a = builder.build()?;

    let mut builder = Builder::from_config(get_config("nodeB", 8888, 8889)?);
    builder.set_chain_source_bitcoind_rpc(
        "0.0.0.0".to_string(),
        38332,
        "local".to_string(),
        "local".to_string(),
    );

    builder.set_entropy_seed_bytes(seed_b_bytes.to_vec())?;

    let node_b = builder.build()?;

    println!("[LDK-Node Payjoin] Starting NodeA and NodeB...");
    node_a.start()?;
    node_b.start()?;

    println!("[LDK-Node Payjoin] Sync Wallets...");
    node_a.sync_wallets()?;
    node_b.sync_wallets()?;

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
        let txid = bitcoind.send_to_address(
            &node_a_address,
            amount,
            None,
            None,
            None,
            None,
            None,
            None,
        )?;
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
            node_a.sync_wallets()?;
            if node_a.list_balances().spendable_onchain_balance_sats >= amount.to_sat() {
                break;
            }
        }
    }

    // ----- Payjoin ------------------------------------------------------------------------------------------
    println!("[LDK-Node Payjoin] Sending some UTXOs to the nodes...");
    let amount = Amount::from_sat(1_000_000);
    bitcoind.send_to_address(&node_a_address, amount, None, None, None, None, None, None)?;
    bitcoind.send_to_address(&node_a_address, amount, None, None, None, None, None, None)?;
    bitcoind.send_to_address(&node_a_address, amount, None, None, None, None, None, None)?;

    bitcoind.send_to_address(&node_b_address, amount, None, None, None, None, None, None)?;
    bitcoind.send_to_address(&node_b_address, amount, None, None, None, None, None, None)?;
    bitcoind.send_to_address(&node_b_address, amount, None, None, None, None, None, None)?;

    println!("[LDK-Node Payjoin] sync_wallets()...");
    sleep(Duration::from_secs(12));
    node_a.sync_wallets()?;
    node_b.sync_wallets()?;
    println!("[LDK-Node Payjoin] sync_wallets() -> Done");

    // First step is to signal that we want to use an arbitrary tx to fund a channel
    node_a
        .payjoin_set_current_channel_info(ChannelId::new_zero(), node_b_address.script_pubkey())?;

    let amount = 777_777;

    let counterparty_node_id = node_b.node_id();
    let counterparty_address = node_b
        .listening_addresses()
        .unwrap()
        .first()
        .unwrap()
        .clone();

    // Now we open a channel
    let channel_id = node_a.open_channel(
        counterparty_node_id,
        counterparty_address,
        amount,
        None,
        None,
    )?;

    println!(
        "[LDK-Node Payjoin] UserChannelId (A <-> B): {:?}",
        channel_id
    );

    wait_for_block(&bitcoind, 2)?;

    // The FundingGenerationReady event will be triggered and we will get the necessary data (channelId, scriptbuf) to fund the channel
    if let Some((channel_id, channel_output_script)) = node_a.payjoin_get_current_channel_info()? {
        println!("[LDK-Node Payjoin] ChannelId (A <-> B): {:?}", channel_id);

        let fee_rate = FeeRate::from_sat_per_vb(DEFAULT_MIN_RELAY_TX_FEE as u64).unwrap();
        let locktime = LockTime::ZERO;
        let mut psbt = node_a.payjoin_build_psbt(
            channel_output_script,
            Amount::from_sat(amount),
            fee_rate,
            locktime,
        )?;

        println!("[LDK-Node Payjoin] PSBT(inputs.len): {}", psbt.inputs.len());
        println!(
            "[LDK-Node Payjoin] PSBT(outputs.len): {}",
            psbt.outputs.len()
        );

        println!("[LDK-Node Payjoin] Adding NodeB UTXOs...");
        node_b.payjoin_add_utxos_to_psbt(&mut psbt)?;

        println!("[LDK-Node Payjoin] PSBT(inputs.len): {}", psbt.inputs.len());
        println!(
            "[LDK-Node Payjoin] PSBT(outputs.len): {}",
            psbt.outputs.len()
        );

        println!("[LDK-Node Payjoin] NodeA signing...");
        node_a.payjoin_sign_psbt(&mut psbt)?;
        println!("[LDK-Node Payjoin] NodeB signing...");
        node_b.payjoin_sign_psbt(&mut psbt)?;

        println!(
            "[LDK-Node Payjoin] NodeA({:?}) (pre-open-channel): {:?} sats",
            node_a_address,
            node_a.list_balances().spendable_onchain_balance_sats
        );

        // Use the Payjoin PSBT as the channel's funding transaction
        node_a.payjoin_fund_channel(channel_id, node_b.node_id(), psbt)?;

        // We can set the (channelId, scriptbuf) back to None now
        node_a.payjoin_reset_current_channel_info()?;
    }

    assert!(node_a
        .list_peers()
        .iter()
        .find(|c| { c.node_id == node_b.node_id() })
        .is_some());

    wait_for_block(&bitcoind, CHANNEL_READY_CONFIRMATION_BLOCKS + 1)?;

    println!(
        "[LDK-Node Payjoin] NodeA({:?}) sync_wallets()",
        node_a_address
    );
    node_a.sync_wallets()?;
    println!(
        "[LDK-Node Payjoin] NodeB({:?}) sync_wallets()",
        node_b_address
    );
    node_b.sync_wallets()?;

    println!(
        "[LDK-Node Payjoin] NodeA({:?}) (pos-open-channel): {:?} sats",
        node_a_address,
        node_a.list_balances().spendable_onchain_balance_sats
    );

    println!("[LDK-Node Payjoin] Sending payment NodeA -> NodeB...");

    let invoice_description =
        Bolt11InvoiceDescription::Direct(Description::new(String::from("asdf")).unwrap());
    let invoice = node_b
        .bolt11_payment()
        .receive(5_000_000, &invoice_description.clone().into(), 9217)
        .unwrap();
    let payment_id = node_a.bolt11_payment().send(&invoice, None)?;

    wait_for_block(&bitcoind, 2)?;

    let status = node_a.payment(&payment_id).unwrap().status;
    println!(
        "[LDK-Node Payjoin] Payment sent: id={} | status(from NodeA): {:?}",
        payment_id, status
    );

    println!(
        "[LDK-Node Payjoin] NodeB({:?}) (pre-close-channel): {:?} sats",
        node_b_address,
        node_b.list_balances().spendable_onchain_balance_sats
    );

    let channels = node_b.list_channels();
    if let Some(channel) = channels.first() {
        node_b.close_channel(&channel.user_channel_id, channel.counterparty_node_id)?;
    }
    wait_for_block(&bitcoind, 2)?;

    let mut confirmation_block = CHANNEL_READY_CONFIRMATION_BLOCKS;
    for ln_balance in node_b.list_balances().lightning_balances {
        match ln_balance {
            ClaimableAwaitingConfirmations {
                confirmation_height,
                ..
            } => confirmation_block = confirmation_height as u64,
            _ => {}
        }
    }

    println!(
        "[LDK-Node Payjoin] ClaimableAwaitingConfirmations at {}",
        confirmation_block
    );

    let current_block = bitcoind.get_block_count()?;
    wait_for_block(&bitcoind, confirmation_block - current_block + 1)?;

    node_b.sync_wallets()?;
    println!(
        "[LDK-Node Payjoin] NodeB({:?}) (pos-close-channel): {:?}",
        node_b_address,
        node_b.list_balances().spendable_onchain_balance_sats
    );

    println!("[LDK-Node Payjoin] Stopping NodeA and NodeB...");
    node_a.stop()?;
    node_b.stop()?;

    Ok(())
}

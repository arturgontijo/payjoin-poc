use std::thread::sleep;
use std::time::Duration;

use bdk_wallet::{KeychainKind, SignOptions};
use bitcoincore_rpc::{Client, RpcApi};

use ldk_node::config::Config;
use ldk_node::lightning::ln::msgs::SocketAddress;
use ldk_node::lightning::ln::types::ChannelId;
use ldk_node::lightning::routing::gossip::NodeAlias;
use ldk_node::lightning_invoice::{Bolt11InvoiceDescription, Description};
use ldk_node::LightningBalance::ClaimableAwaitingConfirmations;
use ldk_node::{
    bitcoin::{
        key::rand::{thread_rng, Rng},
        locktime::absolute::LockTime,
        policy::DEFAULT_MIN_RELAY_TX_FEE,
        Amount, FeeRate, Network, Psbt,
    },
    UserChannelId,
};
use ldk_node::{Builder, Node};

use crate::{
    batch::methods::{add_utxos_to_psbt, build_psbt},
    client::wait_for_block,
    wallet::{create_wallet, fund_wallet, sync_wallet, wallet_total_balance},
};

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

fn random_port() -> u16 {
    let mut rng = thread_rng();
    rng.gen_range(7000..8000)
}

fn setup_nodes(count: u8) -> Result<Vec<Node>, Box<dyn std::error::Error>> {
    let mut nodes = vec![];
    for i in 0..count {
        let port = random_port();
        let node_alias = format!("node_{}", i);
        let mut builder = Builder::from_config(get_config(node_alias.as_str(), port, port + 1)?);
        builder.set_chain_source_bitcoind_rpc(
            "0.0.0.0".to_string(),
            38332,
            "local".to_string(),
            "local".to_string(),
        );
        let seed_bytes = &[i; 64];
        builder.set_entropy_seed_bytes(seed_bytes.to_vec())?;
        let node = builder.build()?;

        println!("[LDK-Node Payjoin][{}] Starting...", node_alias);
        node.start()?;

        nodes.push(node);
    }
    Ok(nodes)
}

fn fund_node(
    bitcoind: &Client,
    node: &Node,
    amount: Amount,
    count: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let node_address = node.onchain_payment().new_address()?;
    for _ in 0..count {
        bitcoind.send_to_address(&node_address, amount, None, None, None, None, None, None)?;
    }
    Ok(())
}

fn open_channel(
    node_a: &Node,
    node_b: &Node,
    amount: Amount,
) -> Result<UserChannelId, Box<dyn std::error::Error>> {
    let user_channel_id = node_a.open_channel(
        node_b.node_id(),
        node_b
            .listening_addresses()
            .unwrap()
            .first()
            .unwrap()
            .clone(),
        amount.to_sat(),
        None,
        None,
    )?;

    println!(
        "[LDK-Node Payjoin] UserChannelId ({} <-> {}): {:?}",
        node_a.node_alias().unwrap().to_string(),
        node_b.node_alias().unwrap().to_string(),
        user_channel_id
    );

    Ok(user_channel_id)
}

pub fn payjoin_batch(bitcoind: &Client) -> Result<(), Box<dyn std::error::Error>> {
    println!("[LDK-Node Payjoin] Setting up Sender and Receiver wallets...");
    let mut sender = create_wallet(&[254u8; 64])?;
    let mut receiver = create_wallet(&[255u8; 64])?;

    fund_wallet(bitcoind, &mut sender, Amount::from_sat(10_000_000), 5)?;

    wait_for_block(bitcoind, 2)?;

    sync_wallet(bitcoind, &mut sender, true)?;

    // ----- Batch Payjoin ------------------------------------------------------------------------------------
    let mut nodes = setup_nodes(5)?;

    println!("[LDK-Node Payjoin] Sending some UTXOs to the nodes...");
    let amount = Amount::from_sat(1_000_000);
    for node in &nodes {
        fund_node(bitcoind, node, amount, 5)?;
    }

    println!("[LDK-Node Payjoin] sync_wallets()...");
    wait_for_block(bitcoind, 2)?;
    for node in &nodes {
        println!(
            "[LDK-Node Payjoin][{}] SyncWallets...",
            node.node_alias().unwrap().to_string()
        );
        node.sync_wallets()?;
    }

    println!("[LDK-Node Payjoin] Setup channel topology...");
    //                    (1M:0)- N2 -(1M:0)
    //                   /                  \
    //  N0 -(100k:0)-> N1                    N4
    //                   \                  /
    //                    (1M:0)- N3 -(1M:0)
    let first_hop_user_channel_id = open_channel(&nodes[0], &nodes[1], Amount::from_sat(500_000))?;
    open_channel(&nodes[1], &nodes[2], Amount::from_sat(500_000))?;
    open_channel(&nodes[1], &nodes[3], Amount::from_sat(500_000))?;
    open_channel(&nodes[2], &nodes[4], Amount::from_sat(500_000))?;
    open_channel(&nodes[3], &nodes[4], Amount::from_sat(500_000))?;

    wait_for_block(bitcoind, 2)?;
    for node in &nodes {
        println!(
            "[LDK-Node Payjoin][{}] SyncWallets...",
            node.node_alias().unwrap().to_string()
        );
        node.sync_wallets()?;
    }

    // Sender wants to batch UTXOs
    let amount = Amount::from_sat(777_777);
    let script_pubkey = receiver
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();

    println!("[LDK-Node Payjoin] Sender builds the initial PSBT...");
    let mut psbt = build_psbt(&mut sender, script_pubkey, amount, 2)?;

    let fee_per_participant = Amount::from_sat(99_999);
    let max_participants = 4;

    // Sender must cover all batch fees (we alredy have 2 UTXOs in from build_psbt() above but just to be super sure)
    println!("[LDK-Node Payjoin] Sender adds more UTXO to the initial PSBT to cover all fee_per_participant...");
    let total_fee = fee_per_participant * (max_participants as u64);
    add_utxos_to_psbt(&mut sender, &mut psbt, 2, None, total_fee, true)?;

    let psbt_hex = psbt.serialize_hex();

    // Sender must start the Batch workflow by selecting an initial Node
    println!("\n[LDK-Node Payjoin] Sender starts the Batch workflow by sending initial PSBT to a Node.\n");
    nodes[0].payjoin_init_psbt_batch(
        nodes[1].node_id(),
        &first_hop_user_channel_id,
        Some(amount),
        fee_per_participant,
        max_participants,
        psbt_hex,
    )?;

    wait_for_block(&bitcoind, 2)?;

    // Sender gets the final PSBT (signed by all participants) from the initial Node
    println!("\n[LDK-Node Payjoin] Sender gets the fully signed PSBT from initial Node.\n");
    let batch_psbts = nodes[0].payjoin_get_batch_psbts()?;
    assert!(batch_psbts.len() == 1);
    let psbt_hex = batch_psbts.first().unwrap();

    let mut psbt = Psbt::deserialize(&hex::decode(psbt_hex).unwrap()).unwrap();

    // Sender must sign and broadcast the final PSBT
    println!("[LDK-Node Payjoin] Sender signs the final PSBT...");
    sender.sign(&mut psbt, SignOptions::default())?;

    println!("[LDK-Node Payjoin] Extracting Tx...\n");
    let tx = psbt.clone().extract_tx()?;

    for node in &nodes {
        println!(
            "[LDK-Node Payjoin][{}] SyncWallets (pre-send-tx)...",
            node.node_alias().unwrap().to_string()
        );
        node.sync_wallets()?;
    }

    let sender_initial_balance = wallet_total_balance(bitcoind, &mut sender)?;
    let receiver_initial_balance = wallet_total_balance(bitcoind, &mut receiver)?;

    let mut nodes_balance = vec![];
    for node in nodes.iter_mut() {
        nodes_balance.push(Amount::from_sat(
            node.list_balances().total_onchain_balance_sats,
        ));
    }

    println!("\nTx Inputs/Outputs:\n");
    for input in tx.input.iter() {
        let tx_info = bitcoind.get_raw_transaction_info(&input.previous_output.txid, None)?;
        let value = tx_info.vout[input.previous_output.vout as usize].value;
        println!("====> Inputs  ({})", value);
    }

    for output in tx.output.iter() {
        println!("====> Outputs ({})", output.value);
    }

    println!(
        "\n[LDK-Node Payjoin] Sending Tx (id={})...\n",
        tx.compute_txid()
    );
    bitcoind.send_raw_transaction(&tx).unwrap();

    wait_for_block(bitcoind, 3)?;

    for node in &nodes {
        println!(
            "[LDK-Node Payjoin][{}] SyncWallets (post-send-tx)...",
            node.node_alias().unwrap().to_string()
        );
        node.sync_wallets()?;
    }

    let balance = wallet_total_balance(bitcoind, &mut sender)?;
    println!(
        "\n[LDK-Node Payjoin] Sender Balances (b/a/delta)  : {} | {} | {}",
        sender_initial_balance,
        balance,
        sender_initial_balance - balance,
    );

    let balance = wallet_total_balance(bitcoind, &mut receiver)?;
    println!(
        "[LDK-Node Payjoin] Receiver Balances (b/a/delta): {} | {} | {}\n",
        receiver_initial_balance,
        balance,
        balance - receiver_initial_balance,
    );

    for (idx, node) in nodes.iter_mut().enumerate() {
        let before = nodes_balance[idx];
        let balance = Amount::from_sat(node.list_balances().total_onchain_balance_sats);
        println!(
            "[LDK-Node Payjoin] Node {} Balances (b/a/delta)  : {} | {} | {}",
            idx,
            before,
            balance,
            balance - before,
        );
    }

    println!("\n[LDK-Node Payjoin] Stopping Nodes...");
    for node in nodes {
        node.stop()?;
    }

    Ok(())
}

pub fn payjoin_open_channel(bitcoind: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let nodes = setup_nodes(2)?;

    let node_a = &nodes[0];
    let node_b = &nodes[1];

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
    let user_channel_id = node_a.open_channel(
        counterparty_node_id,
        counterparty_address,
        amount,
        None,
        None,
    )?;

    println!(
        "[LDK-Node Payjoin] UserChannelId (A <-> B): {:?}",
        user_channel_id
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

use bdk_wallet::{
    bitcoin::{
        locktime::absolute::LockTime,
        policy::DEFAULT_MIN_RELAY_TX_FEE,
        psbt::{Input, Output, Psbt},
        Amount, FeeRate, ScriptBuf, Transaction, TxIn, TxOut, Weight,
    },
    KeychainKind, LocalOutput, SignOptions, Wallet,
};
use bitcoincore_rpc::{Client, RpcApi};

use crate::{
    client::wait_for_block,
    wallet::{create_wallet, fund_wallet, get_wallet_utxos, sync_wallet, wallet_total_balance},
};

pub fn add_utxos_to_psbt(
    wallet: &mut Wallet,
    psbt: &mut Psbt,
    max_count: u16,
    uniform_amount: Option<Amount>,
    fee: Amount,
    payer: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut count = 0;
    let mut receiver_utxos_value = Amount::from_sat(0);
    let utxos = get_wallet_utxos(wallet);
    for utxo in utxos {
        let mut inserted = false;
        for input in psbt.unsigned_tx.input.clone() {
            if input.previous_output.txid == utxo.outpoint.txid
                && input.previous_output.vout == utxo.outpoint.vout
            {
                inserted = true;
            }
        }
        if inserted {
            continue;
        }
        println!(
            "[Batch] Adding UTXO [txid={:?} | vout={:?}]",
            utxo.outpoint.txid, utxo.outpoint.vout
        );
        if let Some(canonical_tx) = wallet
            .transactions()
            .find(|tx| tx.tx_node.compute_txid() == utxo.outpoint.txid)
        {
            let tx = (*canonical_tx.tx_node.tx).clone();
            let input = TxIn {
                previous_output: utxo.outpoint,
                script_sig: Default::default(),
                sequence: Default::default(),
                witness: Default::default(),
            };
            psbt.inputs.push(Input {
                non_witness_utxo: Some(tx),
                ..Default::default()
            });
            psbt.unsigned_tx.input.push(input);
            receiver_utxos_value += utxo.txout.value;

            if let Some(uniform_amount) = uniform_amount {
                if receiver_utxos_value >= uniform_amount + fee {
                    break;
                }
            }

            count += 1;
            if count >= max_count {
                break;
            }
        };
    }

    let mut value = receiver_utxos_value;
    if payer {
        value -= fee;
    } else {
        value += fee
    }

    if let Some(uniform_amount) = uniform_amount {
        let script_pubkey = wallet
            .reveal_next_address(KeychainKind::External)
            .address
            .script_pubkey();

        let output = TxOut {
            value: uniform_amount,
            script_pubkey,
        };
        psbt.outputs.push(Output::default());
        psbt.unsigned_tx.output.push(output);
        value -= uniform_amount;
    }

    let script_pubkey = wallet
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();

    let output = TxOut {
        value,
        script_pubkey,
    };
    psbt.outputs.push(Output::default());
    psbt.unsigned_tx.output.push(output);

    Ok(())
}

fn add_utxos(
    wallet: &mut Wallet,
    psbt_hex: String,
    fee: Amount,
    payer: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let data = hex::decode(psbt_hex)?;
    let mut psbt = Psbt::deserialize(&data).unwrap();
    let mut count = 0;
    let mut receiver_utxos_value = Amount::from_sat(0);
    let utxos = get_wallet_utxos(wallet);
    for utxo in utxos {
        let mut inserted = false;
        for input in psbt.unsigned_tx.input.clone() {
            if input.previous_output.txid == utxo.outpoint.txid
                && input.previous_output.vout == utxo.outpoint.vout
            {
                inserted = true;
            }
        }
        if inserted {
            continue;
        }
        println!(
            "[Batch] Adding UTXO [txid={:?} | vout={:?}]",
            utxo.outpoint.txid, utxo.outpoint.vout
        );
        if let Some(canonical_tx) = wallet
            .transactions()
            .find(|tx| tx.tx_node.compute_txid() == utxo.outpoint.txid)
        {
            let tx = (*canonical_tx.tx_node.tx).clone();
            let input = TxIn {
                previous_output: utxo.outpoint,
                script_sig: Default::default(),
                sequence: Default::default(),
                witness: Default::default(),
            };
            psbt.inputs.push(Input {
                non_witness_utxo: Some(tx),
                ..Default::default()
            });
            psbt.unsigned_tx.input.push(input);
            receiver_utxos_value += utxo.txout.value;

            count += 1;
            if count >= 1 {
                break;
            }
        };
    }

    let script_pubkey = wallet
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();

    let mut value = receiver_utxos_value;
    if payer {
        value -= fee;
    } else {
        value += fee
    }

    let output = TxOut {
        value,
        script_pubkey,
    };
    psbt.outputs.push(Output {
        ..Default::default()
    });
    psbt.unsigned_tx.output.push(output);

    Ok(psbt.serialize_hex())
}

fn add_utxos_from_pool(
    psbt: &mut Psbt,
    utxos: Vec<(LocalOutput, Transaction)>,
    script_pubkey: ScriptBuf,
    fee: Amount,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut count = 0;
    let mut receiver_utxos_value = Amount::from_sat(0);
    for (utxo, tx) in utxos {
        let mut inserted = false;
        for input in psbt.unsigned_tx.input.clone() {
            if input.previous_output.txid == utxo.outpoint.txid
                && input.previous_output.vout == utxo.outpoint.vout
            {
                inserted = true;
            }
        }
        if inserted {
            continue;
        }
        println!(
            "[Batch] Adding UTXO [txid={:?} | vout={:?}]",
            utxo.outpoint.txid, utxo.outpoint.vout
        );

        let input = TxIn {
            previous_output: utxo.outpoint,
            script_sig: Default::default(),
            sequence: Default::default(),
            witness: Default::default(),
        };
        psbt.inputs.push(Input {
            non_witness_utxo: Some(tx),
            ..Default::default()
        });
        psbt.unsigned_tx.input.push(input);
        receiver_utxos_value += utxo.txout.value;

        count += 1;
        if count >= 2 {
            break;
        }
    }

    let output = TxOut {
        value: receiver_utxos_value + fee,
        script_pubkey,
    };
    psbt.outputs.push(Output {
        ..Default::default()
    });
    psbt.unsigned_tx.output.push(output);

    Ok(())
}

pub fn build_psbt(
    sender: &mut Wallet,
    script_pubkey: ScriptBuf,
    amount: Amount,
    count: usize,
) -> Result<Psbt, Box<dyn std::error::Error>> {
    let utxos = get_wallet_utxos(sender);

    let fee_rate = FeeRate::from_sat_per_vb(DEFAULT_MIN_RELAY_TX_FEE as u64).unwrap();
    let locktime = LockTime::ZERO;

    let mut builder = sender.build_tx();
    builder
        .add_recipient(script_pubkey, amount)
        .fee_rate(fee_rate)
        .nlocktime(locktime)
        .manually_selected_only();

    for (idx, utxo) in utxos.iter().enumerate() {
        if idx >= count {
            break;
        }
        builder.add_utxo(utxo.outpoint)?;
    }

    let psbt = builder.finish().unwrap();

    Ok(psbt)
}

fn get_input_value(psbt: &Psbt) -> (Amount, Amount) {
    let mut total_witness_utxo = Amount::ZERO;
    let mut total_non_witness_utxo = Amount::ZERO;

    for input in &psbt.inputs {
        if let Some(witness_utxo) = &input.witness_utxo {
            total_witness_utxo += witness_utxo.value;
        } else if let Some(non_witness_utxo) = &input.non_witness_utxo {
            for inner_input in &psbt.unsigned_tx.input {
                let vout = inner_input.previous_output.vout as usize;
                if let Some(value) = non_witness_utxo.output.get(vout).map(|o| o.value) {
                    total_non_witness_utxo += value;
                }
            }
        }
    }
    (total_witness_utxo, total_non_witness_utxo)
}

fn get_total_output(psbt: &Psbt) -> Amount {
    psbt.unsigned_tx.output.iter().map(|o| o.value).sum()
}

fn setup(
    bitcoind: &Client,
    count: u8,
) -> Result<(Wallet, Wallet, Vec<Wallet>), Box<dyn std::error::Error>> {
    println!("[Batch] Starting...");
    let mut nodes = vec![];
    for idx in 1..=count {
        nodes.push(create_wallet(&[idx; 64])?);
    }

    for mut node in nodes.iter_mut() {
        fund_wallet(bitcoind, &mut node, Amount::from_sat(1_000_000), 10)?;
    }

    let mut sender = create_wallet(&[0u8; 64])?;
    let receiver = create_wallet(&[7u8; 64])?;

    fund_wallet(bitcoind, &mut sender, Amount::from_sat(10_000_000), 4)?;

    wait_for_block(bitcoind, 3)?;

    sync_wallet(bitcoind, &mut sender, true)?;

    for mut node in nodes.iter_mut() {
        sync_wallet(bitcoind, &mut node, true)?;
    }
    Ok((sender, receiver, nodes))
}

// Method 1: Build a initial PSBT and circle it between nodes
//   Requires:
//     1 - Circle the origial PSBT between nodes
//     2 - Each node adds their UTXOs to that PSBT
//     3 - Once its done the final PSBT is circle between each node so they can sign it
pub fn method_1(bitcoind: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let (mut sender, mut receiver, mut nodes) = setup(bitcoind, 5)?;

    println!(
        "[Batch] Sender Balance: {:?}",
        wallet_total_balance(bitcoind, &mut sender)?
    );
    println!(
        "[Batch] Receiver Balance: {:?}",
        wallet_total_balance(bitcoind, &mut receiver)?
    );
    for (idx, node) in nodes.iter_mut().enumerate() {
        println!(
            "[Batch] Node {} Balance: {:?}",
            idx,
            wallet_total_balance(bitcoind, node)?
        );
    }

    // Starting the PSBT
    println!("[Batch] Sender PSBT...");

    let amount = Amount::from_sat(777_777);
    let script_pubkey = receiver
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();
    let mut psbt = build_psbt(&mut sender, script_pubkey, amount, 2)?;

    let fee_per_participant = Amount::from_sat(77_777);
    let participants = nodes.len() as u64;

    println!("[Batch] Getting PSBT from Network...");
    for node in nodes.iter_mut() {
        add_utxos_to_psbt(node, &mut psbt, 2, None, fee_per_participant, false)?;
    }

    // Check total inputs/outputs amount (DEBUG)
    let (wit, non_wit) = get_input_value(&psbt);
    let total_output = get_total_output(&psbt);
    println!("[Batch] Inputs(wit) ({})", wit);
    println!("[Batch] Inputs(nwt) ({})", non_wit);
    println!("[Batch] Outputs     ({})", total_output);
    let total_fee = fee_per_participant * participants;
    println!("[Batch] TotalFee    ({})", total_fee);
    println!("[Batch] Delta       ({})", total_output - wit - total_fee);

    // To cover fees
    add_utxos_to_psbt(&mut sender, &mut psbt, 2, None, total_fee, true)?;

    for node in nodes.iter_mut() {
        node.sign(&mut psbt, SignOptions::default()).unwrap();
    }

    sender.sign(&mut psbt, SignOptions::default()).unwrap();

    println!("[Batch] Extracting Tx...");
    let tx = psbt.clone().extract_tx()?;

    println!("[Batch] Sending Tx...");
    bitcoind.send_raw_transaction(&tx).unwrap();

    wait_for_block(bitcoind, 2)?;

    println!(
        "[Batch] Sender Balance: {:?}",
        wallet_total_balance(bitcoind, &mut sender)?
    );
    println!(
        "[Batch] Receiver Balance: {:?}",
        wallet_total_balance(bitcoind, &mut receiver)?
    );
    for (idx, node) in nodes.iter_mut().enumerate() {
        println!(
            "[Batch] Node {} Balance: {:?}",
            idx,
            wallet_total_balance(bitcoind, node)?
        );
    }

    Ok(())
}

// Method 2: Merging PSBTs.
//   Requires:
//     1 - Sender builds a PSBT
//     2 - Each node builds their own PSBT
//     3 - Sender "merge" them into a final PSBT
//     4 - Once its done the final PSBT is circle between each node so they can sign it
pub fn method_2(bitcoind: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let (mut sender, mut receiver, mut nodes) = setup(bitcoind, 5)?;
    // Starting the PSBT
    println!("[Batch] Sender PSBT...");
    let amount = Amount::from_sat(777_777);
    let script_pubkey = receiver
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();
    let mut sender_psbt = build_psbt(&mut sender, script_pubkey, amount, 2)?;

    println!("[Batch] Getting PSBT from Network...");
    let mut psbts = vec![];
    for node in nodes.iter_mut() {
        let script_pubkey = node
            .reveal_next_address(KeychainKind::External)
            .address
            .script_pubkey();
        let psbt = build_psbt(node, script_pubkey, Amount::from_sat(500_000), 2)?;
        psbts.push(psbt);
    }

    println!("[Batch] Building final PSBT from Network's one...");
    for psbt in psbts {
        sender_psbt
            .unsigned_tx
            .input
            .extend(psbt.unsigned_tx.input.clone());
        sender_psbt
            .unsigned_tx
            .output
            .extend(psbt.unsigned_tx.output.clone());
        sender_psbt.inputs.extend(psbt.inputs.clone());
        sender_psbt.outputs.extend(psbt.outputs.clone());
    }

    sender.sign(&mut sender_psbt, SignOptions::default())?;
    for node in nodes.iter_mut() {
        node.sign(&mut sender_psbt, SignOptions::default())?;
    }

    println!("[Batch] Extracting Tx...");
    let tx = sender_psbt.clone().extract_tx()?;

    let total_output: Amount = tx.output.iter().map(|output| output.value).sum();
    println!("====> Outputs ({})", total_output);

    println!("[Batch] Sending Tx...");
    bitcoind.send_raw_transaction(&tx).unwrap();

    Ok(())
}

// Method 3: Adding foreign UTXOs to the sender's PSBT.
//   Requires:
//     1 - Sender builds a PSBT
//     2 - Each node builds shared their UTXOs
//     3 - Sender adds the nodes' UTXOs to the original PSBT
//     4 - Once its done the final PSBT is circle between each node so they can sign it
pub fn method_3(bitcoind: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let (mut sender, mut receiver, mut nodes) = setup(bitcoind, 5)?;
    // Starting the PSBT
    println!("[Batch] Sender PSBT...");
    let amount = Amount::from_sat(777_777);
    let script_pubkey = receiver
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();

    let fee_rate = FeeRate::from_sat_per_vb(DEFAULT_MIN_RELAY_TX_FEE as u64).unwrap();
    let locktime = LockTime::ZERO;

    let mut builder = sender.build_tx();
    builder
        .add_recipient(script_pubkey, amount)
        .fee_rate(fee_rate)
        .nlocktime(locktime);

    println!("[Batch] Getting UTXOs from Network...");
    for node in nodes.iter_mut() {
        let utxos = get_wallet_utxos(&node);
        for utxo in utxos {
            if let Some(canonical_tx) = node
                .transactions()
                .find(|tx| tx.tx_node.compute_txid() == utxo.outpoint.txid)
            {
                let tx = (*canonical_tx.tx_node.tx).clone();
                let psbt_input = Input {
                    non_witness_utxo: Some(tx),
                    ..Default::default()
                };
                let satisfaction_weight = match psbt_input.clone().witness_utxo {
                    Some(w) => w.weight(),
                    None => Weight::MIN,
                };
                builder.add_foreign_utxo(utxo.outpoint, psbt_input, satisfaction_weight)?;
            }
        }
    }

    let mut sender_psbt = builder.finish()?;

    sender.sign(&mut sender_psbt, SignOptions::default())?;
    for node in nodes.iter_mut() {
        node.sign(&mut sender_psbt, SignOptions::default())?;
    }

    println!("[Batch] Extracting Tx...");
    let tx = sender_psbt.clone().extract_tx()?;

    let total_output: Amount = tx.output.iter().map(|output| output.value).sum();
    println!("====> Outputs ({})", total_output);

    println!("[Batch] Sending Tx...");
    bitcoind.send_raw_transaction(&tx).unwrap();

    Ok(())
}

// Method 4: Hex PSBTs.
//   Requires:
//     1 - Sender builds a PSBT and serializes it into hex
//     2 - Each node receives the hex PSBT, deselializes it and adds their own UTXOs (incrementally)
//     3 - Sender get the final hex, deserializes it into the final PSBT
//     4 - Once its done the final PSBT is circle between each node so they can sign it
pub fn method_4(bitcoind: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let (mut sender, mut receiver, mut nodes) = setup(bitcoind, 5)?;

    println!(
        "[Batch] Sender Balance: {:?}",
        wallet_total_balance(bitcoind, &mut sender)?
    );
    println!(
        "[Batch] Receiver Balance: {:?}",
        wallet_total_balance(bitcoind, &mut receiver)?
    );
    for (idx, node) in nodes.iter_mut().enumerate() {
        println!(
            "[Batch] Node {} Balance: {:?}",
            idx,
            wallet_total_balance(bitcoind, node)?
        );
    }

    // Starting the PSBT
    println!("[Batch] Sender PSBT...");

    let amount = Amount::from_sat(777_777);
    let script_pubkey = receiver
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();
    let mut psbt = build_psbt(&mut sender, script_pubkey, amount, 2)?;

    let fee_per_participant = Amount::from_sat(77_777);
    let participants = nodes.len() as u64;

    println!("[Batch] Getting PSBT from Network...");
    let mut psbt_hex = psbt.serialize_hex();
    for node in nodes.iter_mut() {
        println!("\n[Batch] PSBT(hex) from Network: {}\n", psbt_hex);
        psbt_hex = add_utxos(node, psbt_hex, fee_per_participant, false)?;
    }

    // DEBUG
    psbt = Psbt::deserialize(&hex::decode(psbt_hex)?)?;

    let total_fee = fee_per_participant * participants;
    println!("[Batch] TotalFee    ({})", total_fee);

    // To cover fees
    psbt_hex = psbt.serialize_hex();
    println!("\n[Batch] PSBT(hex) from sender: {}\n", psbt_hex);
    psbt_hex = add_utxos(&mut sender, psbt_hex, total_fee, true)?;

    psbt = Psbt::deserialize(&hex::decode(psbt_hex)?)?;

    println!("[Batch] Nodes signing...");
    for node in nodes.iter_mut() {
        node.sign(&mut psbt, SignOptions::default()).unwrap();
    }

    println!("[Batch] Sender signing...");
    sender.sign(&mut psbt, SignOptions::default()).unwrap();

    println!("[Batch] Extracting Tx...");
    let tx = psbt.clone().extract_tx()?;

    println!("[Batch] Sending Tx...");
    bitcoind.send_raw_transaction(&tx).unwrap();

    wait_for_block(bitcoind, 2)?;

    println!(
        "[Batch] Sender Balance: {:?}",
        wallet_total_balance(bitcoind, &mut sender)?
    );
    println!(
        "[Batch] Receiver Balance: {:?}",
        wallet_total_balance(bitcoind, &mut receiver)?
    );
    for (idx, node) in nodes.iter_mut().enumerate() {
        println!(
            "[Batch] Node {} Balance: {:?}",
            idx,
            wallet_total_balance(bitcoind, node)?
        );
    }

    Ok(())
}

// Method 5: Pool.
//   Requires:
//     1 - Sender builds a PSBT by selecting nodes' UTXOs to be added to the PSBT (via a Pool of UTXOs data)
//     2 - Once its done the final PSBT is circle between each node so they can sign it
pub fn method_5(bitcoind: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let (mut sender, mut receiver, mut nodes) = setup(bitcoind, 5)?;

    println!(
        "[Batch] Sender Balance: {:?}",
        wallet_total_balance(bitcoind, &mut sender)?
    );
    println!(
        "[Batch] Receiver Balance: {:?}",
        wallet_total_balance(bitcoind, &mut receiver)?
    );
    for (idx, node) in nodes.iter_mut().enumerate() {
        println!(
            "[Batch] Node {} Balance: {:?}",
            idx,
            wallet_total_balance(bitcoind, node)?
        );
    }

    // Starting the PSBT
    println!("[Batch] Sender PSBT...");

    let amount = Amount::from_sat(777_777);
    let script_pubkey = receiver
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();
    let mut psbt = build_psbt(&mut sender, script_pubkey, amount, 2)?;

    let participants = nodes.len() as u64;
    let fee_per_participant = Amount::from_sat(77_777);

    println!("[Batch] Nodes send their avail txs to Network Pool...");
    let mut pool = vec![];
    for node in nodes.iter_mut() {
        let mut nodes_utxos = vec![];
        let script_pubkey = node
            .reveal_next_address(KeychainKind::External)
            .address
            .script_pubkey();

        let utxos = get_wallet_utxos(node);
        for utxo in utxos {
            if let Some(canonical_tx) = node
                .transactions()
                .find(|tx| tx.tx_node.compute_txid() == utxo.outpoint.txid)
            {
                let tx = (*canonical_tx.tx_node.tx).clone();
                nodes_utxos.push((utxo, tx));
            }
        }
        pool.push((script_pubkey, nodes_utxos));
    }

    println!("[Batch] Getting transaction from Network Pool");
    for (script_buf, utxos_txs) in pool {
        add_utxos_from_pool(&mut psbt, utxos_txs, script_buf, fee_per_participant)?;
    }

    let total_fee = fee_per_participant * participants;
    println!("[Batch] TotalFee    ({})", total_fee);

    // To cover fees
    add_utxos_to_psbt(&mut sender, &mut psbt, 2, None, total_fee, true)?;

    for node in nodes.iter_mut() {
        node.sign(&mut psbt, SignOptions::default()).unwrap();
    }

    sender.sign(&mut psbt, SignOptions::default()).unwrap();

    println!("[Batch] Extracting Tx...");
    let tx = psbt.clone().extract_tx()?;

    println!("[Batch] Sending Tx...");
    bitcoind.send_raw_transaction(&tx).unwrap();

    wait_for_block(bitcoind, 3)?;

    println!(
        "[Batch] Sender Balance: {:?}",
        wallet_total_balance(bitcoind, &mut sender)?
    );
    println!(
        "[Batch] Receiver Balance: {:?}",
        wallet_total_balance(bitcoind, &mut receiver)?
    );
    for (idx, node) in nodes.iter_mut().enumerate() {
        println!(
            "[Batch] Node {} Balance: {:?}",
            idx,
            wallet_total_balance(bitcoind, node)?
        );
    }

    Ok(())
}

// Method 6: Uniform output sizes.
//   Requires:
//     1 - Circle the origial PSBT between nodes
//     2 - Each node adds their UTXOs to that PSBT with, at least, one output with receiver's amount (uniform)
//     3 - Once its done the final PSBT is circle back to each node so they can sign it
pub fn method_6(bitcoind: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let (mut sender, mut receiver, mut nodes) = setup(bitcoind, 5)?;
    // Starting the PSBT
    println!("[Batch] Sender PSBT...");
    let amount = Amount::from_sat(777_777);
    let script_pubkey = receiver
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();

    let mut psbt = build_psbt(&mut sender, script_pubkey, amount, 2)?;

    let participants = nodes.len() as u64;
    let fee_per_participant = Amount::from_sat(77_777);

    let total_fee = fee_per_participant * participants;
    println!("[Batch] TotalFee    ({})", total_fee);

    // To cover fees
    add_utxos_to_psbt(&mut sender, &mut psbt, 2, None, total_fee, true)?;

    println!("[Batch] Sending PSBT to the Network...");
    for node in nodes.iter_mut() {
        add_utxos_to_psbt(node, &mut psbt, 2, Some(amount), fee_per_participant, false)?;
    }

    sender.sign(&mut psbt, SignOptions::default())?;
    for node in nodes.iter_mut() {
        node.sign(&mut psbt, SignOptions::default())?;
    }

    println!("[Batch] Extracting Tx...");
    let tx = psbt.clone().extract_tx()?;

    for input in tx.input.iter() {
        let tx = bitcoind.get_raw_transaction_info(&input.previous_output.txid, None)?;
        let value = tx.vout[input.previous_output.vout as usize].value;
        println!("====> Input ({})", value);
    }

    for output in tx.output.iter() {
        println!("====> Outputs ({})", output.value);
    }

    let sender_initial_balance = wallet_total_balance(bitcoind, &mut sender)?;
    let receiver_initial_balance = wallet_total_balance(bitcoind, &mut receiver)?;

    let mut nodes_balance = vec![];
    for node in nodes.iter_mut() {
        nodes_balance.push(wallet_total_balance(bitcoind, node)?);
    }

    println!("[Batch] Sending Tx...");
    bitcoind.send_raw_transaction(&tx).unwrap();

    wait_for_block(bitcoind, 3)?;

    let balance = wallet_total_balance(bitcoind, &mut sender)?;
    println!(
        "[Batch] Sender Balance (b/a/delta)  : {} | {} | {}",
        sender_initial_balance,
        balance,
        sender_initial_balance - balance,
    );

    let balance = wallet_total_balance(bitcoind, &mut receiver)?;
    println!(
        "[Batch] Receiver Balance (b/a/delta): {} | {} | {}",
        receiver_initial_balance,
        balance,
        balance - receiver_initial_balance,
    );

    for (idx, node) in nodes.iter_mut().enumerate() {
        let before = nodes_balance[idx];
        let balance = wallet_total_balance(bitcoind, node)?;
        println!(
            "[Batch] Node {} Balance (b/a/delta)  : {} | {} | {}",
            idx,
            before,
            balance,
            balance - before,
        );
    }

    Ok(())
}

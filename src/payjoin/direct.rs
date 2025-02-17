use bdk_wallet::{
    bitcoin::{
        policy::DEFAULT_MIN_RELAY_TX_FEE,
        psbt::{Input, Output},
        Amount, FeeRate, TxIn, TxOut,
    },
    KeychainKind, SignOptions, Wallet,
};
use bitcoincore_rpc::{Client, RpcApi};

use crate::{
    client::wait_for_block,
    wallet::{get_wallet_utxos, wallet_total_balance},
};

pub fn direct_payjoin(
    bitcoind: &Client,
    sender: &mut Wallet,
    receiver: &mut Wallet,
    amount: Amount,
) -> Result<bool, Box<dyn std::error::Error>> {
    let sender_utxos = get_wallet_utxos(&sender);

    let script_pubkey = receiver
        .reveal_next_address(KeychainKind::External)
        .address
        .script_pubkey();
    let mut builder = sender.build_tx();
    builder.add_recipient(script_pubkey.clone(), amount);
    builder.fee_rate(FeeRate::from_sat_per_vb(DEFAULT_MIN_RELAY_TX_FEE as u64).unwrap());
    builder.manually_selected_only();

    // Add sender's UTXOs
    let mut count = 0;
    let mut sender_utxos_value = Amount::from_sat(0);
    for utxo in sender_utxos {
        println!(
            "[Payjoin] Adding sender UTXO [txid={:?} | vout={:?}]",
            utxo.outpoint.txid, utxo.outpoint.vout
        );
        builder.add_utxo(utxo.outpoint).unwrap();
        sender_utxos_value += utxo.txout.value;
        count += 1;
        if count >= 3 {
            break;
        }
    }

    let mut psbt = builder.finish().unwrap();

    // Add receiver's UTXOs
    let mut count = 0;
    let mut receiver_utxos_value = Amount::from_sat(0);
    for utxo in get_wallet_utxos(&receiver) {
        println!(
            "[Payjoin] Adding receiver UTXO [txid={:?} | vout={:?}]",
            utxo.outpoint.txid, utxo.outpoint.vout
        );
        let tx = bitcoind.get_raw_transaction(&utxo.outpoint.txid, None)?;
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

    // Output
    println!("[Payjoin] Adding receiver output (sending amount + receiver's UTXO values) [amount={:?} | value={:?}]", amount.to_btc(), receiver_utxos_value.to_btc());
    for (idx, out) in psbt.clone().unsigned_tx.output.iter().enumerate() {
        if out.script_pubkey == script_pubkey {
            let script_pubkey = receiver
                .reveal_next_address(KeychainKind::External)
                .address
                .script_pubkey();
            let output = TxOut {
                value: amount + receiver_utxos_value,
                script_pubkey,
            };
            psbt.outputs[idx] = Output {
                ..Default::default()
            };
            psbt.unsigned_tx.output[idx] = output;
            break;
        }
    }

    println!("[Payjoin] Sender signing PSBT...");
    sender.sign(&mut psbt, SignOptions::default()).unwrap();

    println!("[Payjoin] Receiver signing PSBT...");
    receiver.sign(&mut psbt, SignOptions::default()).unwrap();

    println!("[Payjoin] Sender finalizing PSBT...");
    sender
        .finalize_psbt(&mut psbt, SignOptions::default())
        .unwrap();

    println!(
        "[Payjoin] Snd(before): {:?}",
        wallet_total_balance(&bitcoind, sender)?.to_btc()
    );
    println!(
        "[Payjoin] Rcv(before): {:?}",
        wallet_total_balance(&bitcoind, receiver)?.to_btc()
    );

    let tx = psbt.clone().extract_tx()?;
    println!("[Payjoin] Sending Tx...");
    bitcoind.send_raw_transaction(&tx).unwrap();

    wait_for_block(&bitcoind, 3)?;

    let fee = psbt.fee()?;
    let sender_balance = wallet_total_balance(&bitcoind, sender)?;
    println!(
        "[Payjoin] Snd(after): {:?} (fee={:?}) -> {:?}",
        sender_balance.to_btc(),
        fee.to_btc(),
        (sender_balance + fee).to_btc()
    );
    println!(
        "[Payjoin] Rcv(after) : {:?}",
        wallet_total_balance(&bitcoind, receiver)?.to_btc()
    );

    Ok(true)
}

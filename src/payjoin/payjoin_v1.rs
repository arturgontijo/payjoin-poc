use bitcoincore_rpc::{json::WalletProcessPsbtResult, Client, RpcApi};

use payjoin::{
    bitcoin::{
        self, policy::DEFAULT_MIN_RELAY_TX_FEE, psbt::Input as PsbtInput,
        transaction::InputWeightPrediction, Address, Amount, FeeRate, Network, Psbt, TxIn, TxOut,
        Weight,
    },
    receive::{Headers, InputPair},
    send::SenderBuilder,
    PjUri, PjUriBuilder, Request, Uri, UriExt, Url,
};

use std::{collections::HashMap, str::FromStr};

use crate::client::get_client_balance;

pub type BoxError = Box<dyn std::error::Error + 'static>;

struct MockHeaders {
    length: String,
}

impl MockHeaders {
    fn new(length: u64) -> MockHeaders {
        MockHeaders {
            length: length.to_string(),
        }
    }
}

impl Headers for MockHeaders {
    fn get_header(&self, key: &str) -> Option<&str> {
        match key {
            "content-length" => Some(&self.length),
            "content-type" => Some("text/plain"),
            _ => None,
        }
    }
}

pub fn build_v1_pj_uri<'a>(address: bitcoin::Address, endpoint: payjoin::Url) -> PjUri<'a> {
    PjUriBuilder::new(address, endpoint, None, None, None).build()
}

fn build_original_psbt(
    sender: &bitcoincore_rpc::Client,
    address: &Address,
    amount: Amount,
    // pj_uri: &PjUri,
) -> Result<Psbt, BoxError> {
    let mut outputs = HashMap::with_capacity(1);
    outputs.insert(address.to_string(), amount);

    let options = bitcoincore_rpc::json::WalletCreateFundedPsbtOptions {
        lock_unspent: Some(true),
        // The minimum relay feerate ensures that tests fail if the receiver would add inputs/outputs
        // that cannot be covered by the sender's additional fee contributions.
        fee_rate: Some(Amount::from_sat(DEFAULT_MIN_RELAY_TX_FEE.into())),
        ..Default::default()
    };

    let psbt = sender
        .wallet_create_funded_psbt(
            &[], // inputs
            &outputs,
            None, // locktime
            Some(options),
            Some(true), // check that the sender properly clears keypaths
        )?
        .psbt;

    let psbt = sender
        .wallet_process_psbt(&psbt.to_string(), None, None, None)?
        .psbt;

    Ok(Psbt::from_str(&psbt)?)
}

pub fn input_pair_from_list_unspent(
    utxo: bitcoincore_rpc::bitcoincore_rpc_json::ListUnspentResultEntry,
) -> InputPair {
    let psbtin = PsbtInput {
        // NOTE: non_witness_utxo is not necessary because bitcoin-cli always supplies
        // witness_utxo, even for non-witness inputs
        witness_utxo: Some(bitcoin::TxOut {
            value: utxo.amount,
            script_pubkey: utxo.script_pub_key.clone(),
        }),
        redeem_script: utxo.redeem_script.clone(),
        witness_script: utxo.witness_script.clone(),
        ..Default::default()
    };
    let txin = TxIn {
        previous_output: bitcoin::OutPoint {
            txid: utxo.txid,
            vout: utxo.vout,
        },
        ..Default::default()
    };
    InputPair::new(txin, psbtin).expect("Input pair should be valid")
}

fn handle_proposal(
    proposal: payjoin::receive::UncheckedProposal,
    receiver: &bitcoincore_rpc::Client,
    custom_outputs: Option<Vec<TxOut>>,
    drain_script: Option<&bitcoin::Script>,
    custom_inputs: Option<Vec<InputPair>>,
) -> Result<payjoin::receive::PayjoinProposal, BoxError> {
    // in a payment processor where the sender could go offline, this is where you schedule to broadcast the original_tx
    let _to_broadcast_in_failure_case = proposal.extract_tx_to_schedule_broadcast();

    // Receive Check 1: Can Broadcast
    let proposal = proposal.check_broadcast_suitability(None, |tx| {
        Ok(receiver
            .test_mempool_accept(&[bitcoin::consensus::encode::serialize_hex(&tx)])
            .unwrap()
            .first()
            .unwrap()
            .allowed)
    })?;

    // Receive Check 2: receiver can't sign for proposal inputs
    let proposal = proposal.check_inputs_not_owned(|input| {
        let address = bitcoin::Address::from_script(input, Network::Signet).unwrap();
        Ok(receiver
            .get_address_info(&address)
            .map(|info| info.is_mine.unwrap_or(false))
            .unwrap())
    })?;

    // Receive Check 3: have we seen this input before? More of a check for non-interactive i.e. payment processor receivers.
    let payjoin = proposal
        .check_no_inputs_seen_before(|_| Ok(false))?
        .identify_receiver_outputs(|output_script| {
            let address = bitcoin::Address::from_script(output_script, Network::Signet).unwrap();
            Ok(receiver
                .get_address_info(&address)
                .map(|info| info.is_mine.unwrap_or(false))
                .unwrap())
        })?;

    let payjoin = match custom_outputs {
        Some(txos) => payjoin.replace_receiver_outputs(
            txos,
            drain_script.expect("drain_script should be provided with custom_outputs"),
        )?,
        None => payjoin.substitute_receiver_script(
            &receiver
                .get_new_address(None, None)?
                .assume_checked()
                .script_pubkey(),
        )?,
    }
    .commit_outputs();

    let inputs = match custom_inputs {
        Some(inputs) => inputs,
        None => {
            let candidate_inputs = receiver
                .list_unspent(None, None, None, None, None)?
                .into_iter()
                .map(input_pair_from_list_unspent);
            let selected_input = payjoin
                .try_preserving_privacy(candidate_inputs)
                .map_err(|e| format!("Failed to make privacy preserving selection: {:?}", e))?;
            vec![selected_input]
        }
    };
    let payjoin = payjoin
        .contribute_inputs(inputs)
        .map_err(|e| format!("Failed to contribute inputs: {:?}", e))?
        .commit_inputs();

    let payjoin_proposal = payjoin.finalize_proposal(
        |psbt: &Psbt| {
            Ok(receiver
                .wallet_process_psbt(
                    &psbt.to_string(),
                    None,
                    None,
                    Some(true), // check that the receiver properly clears keypaths
                )
                .map(|res: WalletProcessPsbtResult| {
                    Psbt::from_str(&res.psbt).expect("psbt should be valid")
                })
                .unwrap())
        },
        Some(FeeRate::BROADCAST_MIN),
        Some(FeeRate::from_sat_per_vb_unchecked(2)).unwrap(),
    )?;
    Ok(payjoin_proposal)
}

fn handle_v1_pj_request(
    req: Request,
    receiver: &bitcoincore_rpc::Client,
    custom_outputs: Option<Vec<TxOut>>,
    drain_script: Option<&bitcoin::Script>,
    custom_inputs: Option<Vec<InputPair>>,
) -> Result<String, BoxError> {
    // Receiver receive payjoin proposal, IRL it will be an HTTP request (over ssl or onion)
    let headers = MockHeaders::new(req.body.len() as u64);
    let proposal = payjoin::receive::UncheckedProposal::from_request(
        req.body.as_slice(),
        req.url.query().unwrap_or(""),
        headers,
    )?;

    let proposal = handle_proposal(
        proposal,
        receiver,
        custom_outputs,
        drain_script,
        custom_inputs,
    )?;

    assert!(!proposal.is_output_substitution_disabled());
    let psbt = proposal.psbt();
    println!(
        "[PayjoinV1] Receiver's Payjoin proposal PSBT(inputs.len): {:#?}",
        &psbt.inputs.len()
    );
    println!(
        "[PayjoinV1] Receiver's Payjoin proposal PSBT(outputs.len): {:#?}",
        &psbt.outputs.len()
    );
    Ok(psbt.to_string())
}

fn extract_pj_tx(
    sender: &bitcoincore_rpc::Client,
    psbt: Psbt,
) -> Result<bitcoin::Transaction, Box<dyn std::error::Error>> {
    let payjoin_psbt = sender
        .wallet_process_psbt(&psbt.to_string(), None, None, None)?
        .psbt;
    let payjoin_psbt = sender
        .finalize_psbt(&payjoin_psbt, Some(false))?
        .psbt
        .expect("should contain a PSBT");
    let payjoin_psbt = Psbt::from_str(&payjoin_psbt)?;
    println!(
        "[PayjoinV1] Final Payjoin PSBT(inputs.len): {:#?}",
        &payjoin_psbt.inputs.len()
    );
    println!(
        "[PayjoinV1] Final Payjoin PSBT(outputs.len): {:#?}",
        &payjoin_psbt.outputs.len()
    );
    Ok(payjoin_psbt.extract_tx()?)
}

/// Simplified input weight predictions for a fully-signed transaction
fn predicted_tx_weight(tx: &bitcoin::Transaction) -> Weight {
    let input_weight_predictions = tx.input.iter().map(|txin| {
        // See https://bitcoin.stackexchange.com/a/107873
        match (txin.script_sig.is_empty(), txin.witness.is_empty()) {
            // witness is empty: legacy input
            (false, true) => InputWeightPrediction::P2PKH_COMPRESSED_MAX,
            // script sig is empty: native segwit input
            (true, false) => match txin.witness.len() {
                // <signature>
                1 => InputWeightPrediction::P2TR_KEY_DEFAULT_SIGHASH,
                // <signature> <public_key>
                2 => InputWeightPrediction::P2WPKH_MAX,
                _ => panic!("unsupported witness"),
            },
            // neither are empty: nested segwit (p2wpkh-in-p2sh) input
            (false, false) => InputWeightPrediction::from_slice(23, &[72, 33]),
            _ => panic!("one of script_sig or witness should be non-empty"),
        }
    });
    bitcoin::transaction::predict_weight(input_weight_predictions, tx.script_pubkey_lens())
}

pub fn do_payjoin_v1(
    sender: &Client,
    receiver: &Client,
    amount: Amount,
    is_p2pkh: bool,
) -> Result<(), BoxError> {
    println!(
        "[PayjoinV1] Snd(before): {:?}",
        get_client_balance(sender)?.to_btc()
    );
    println!(
        "[PayjoinV1] Rcv(before): {:?}",
        get_client_balance(receiver)?.to_btc()
    );

    // Receiver creates the payjoin URI
    let pj_receiver_address = receiver.get_new_address(None, None)?.assume_checked();
    let endpoint = Url::parse("https://example.com")?;
    let mut pj_uri = build_v1_pj_uri(pj_receiver_address.clone(), endpoint);
    pj_uri.amount = Some(amount);

    // **********************
    // Inside the Sender:
    // Sender create a funded PSBT (not broadcasted) to address with amount given in the pj_uri
    let uri = Uri::from_str(&pj_uri.to_string())
        .map_err(|e| e.to_string())?
        .assume_checked()
        .check_pj_supported()
        .map_err(|e| e.to_string())?;

    let psbt = build_original_psbt(&sender, &pj_receiver_address, amount)?;
    println!(
        "[PayjoinV1] Sender's Payjoin proposal PSBT(inputs.len): {:#?}",
        &psbt.inputs.len()
    );
    println!(
        "[PayjoinV1] Sender's Payjoin proposal PSBT(outputs.len): {:#?}",
        &psbt.outputs.len()
    );

    let (req, ctx) = SenderBuilder::from_psbt_and_uri(psbt, uri)?
        .build_with_additional_fee(Amount::from_sat(10000), None, FeeRate::ZERO, false)?
        .extract_v1()?;

    // **********************
    // Inside the Receiver:
    // this data would transit from one party to another over the network in production
    let response = handle_v1_pj_request(req, &receiver, None, None, None)?;
    // this response would be returned as http response to the sender

    // **********************
    // Inside the Sender:
    // Sender checks, signs, finalizes, extracts, and broadcasts
    let checked_payjoin_proposal_psbt = ctx.process_response(&mut response.as_bytes())?;
    let payjoin_tx = extract_pj_tx(&sender, checked_payjoin_proposal_psbt)?;
    sender.send_raw_transaction(&payjoin_tx)?;

    // Check resulting transaction and balances
    let mut predicted_tx_weight = predicted_tx_weight(&payjoin_tx);
    if is_p2pkh {
        // HACK:
        // bitcoin-cli always grinds signatures to save 1 byte (4WU) and simplify fee
        // estimates. This results in the original PSBT having a fee of 219 sats
        // instead of the "worst case" 220 sats assuming a maximum-size signature.
        // Note that this also affects weight predictions for segwit inputs, but the
        // resulting signatures are only 1WU smaller (.25 bytes) and therefore don't
        // affect our weight predictions for the original sender inputs.
        predicted_tx_weight -= Weight::from_non_witness_data_size(1);
    }

    let network_fees = predicted_tx_weight * FeeRate::BROADCAST_MIN;

    // assert_eq!(payjoin_tx.input.len(), 2);
    // assert_eq!(payjoin_tx.output.len(), 2);
    // assert_eq!(receiver.get_balances()?.mine.untrusted_pending, amount + receiver_initial_balance);
    // assert_eq!(sender.get_balances()?.mine.untrusted_pending, sender_initial_balance - amount - network_fees);

    println!(
        "[PayjoinV1] Snd(after): {:?}",
        get_client_balance(sender)?.to_btc()
    );
    println!(
        "[PayjoinV1] Rcv(after): {:?}",
        get_client_balance(receiver)?.to_btc()
    );
    println!("[PayjoinV1] Fee     : {}", network_fees);

    Ok(())
}

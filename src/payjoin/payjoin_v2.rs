use bitcoincore_rpc::bitcoin::psbt::Psbt;
use bitcoincore_rpc::bitcoin::Amount;
use bitcoincore_rpc::bitcoin::Txid;
use bitcoincore_rpc::{Client, RpcApi};

use payjoin::io::fetch_ohttp_keys;
use payjoin::send::SenderBuilder;
use url::Url;

use std::collections::HashMap;
use std::str::FromStr;

use crate::client::get_client_balance;

fn https_agent() -> reqwest::Client {
    let https = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("failed to build https client");
    https
}

pub async fn do_payjoin_v2(
    sender: &Client,
    receiver: &Client,
    amount: Amount,
) -> Result<Txid, Box<dyn std::error::Error>> {
    println!(
        "[PayjoinV2] Snd(before): {:?}",
        get_client_balance(sender)?.to_btc()
    );
    println!(
        "[PayjoinV2] Rcv(before): {:?}",
        get_client_balance(receiver)?.to_btc()
    );

    let ohttp_relay = Url::parse("https://pj.bobspacebkk.com")?;
    let directory = Url::parse("https://payjo.in")?;

    let receiver_address = receiver.get_new_address(None, None)?.assume_checked();

    // Preparing Payjoin URI
    let ohttp_keys = fetch_ohttp_keys(ohttp_relay.clone(), directory.clone()).await?;
    let reveiver_session = payjoin::receive::v2::Receiver::new(
        receiver_address,
        directory.clone(),
        ohttp_keys,
        ohttp_relay.clone(),
        Some(std::time::Duration::from_secs(600)),
    );

    let payjoin_uri = reveiver_session.pj_uri_builder().amount(amount).build();

    println!("[PayjoinV2] URI:\n{}", payjoin_uri.to_string());

    let amount_to_send = payjoin_uri.amount.unwrap();
    let receiver_address = payjoin_uri.address.clone();

    let mut outputs = HashMap::with_capacity(1);
    outputs.insert(receiver_address.to_string(), amount_to_send);
    let options = bitcoincore_rpc::json::WalletCreateFundedPsbtOptions {
        lock_unspent: Some(false),
        fee_rate: Some(bitcoincore_rpc::bitcoin::Amount::from_sat(10000)),
        ..Default::default()
    };

    let sender_psbt = sender.wallet_create_funded_psbt(
        &[], // inputs
        &outputs,
        None, // locktime
        Some(options),
        None,
    )?;

    let psbt = sender
        .wallet_process_psbt(&sender_psbt.psbt, None, None, None)?
        .psbt;

    let psbt = Psbt::from_str(&psbt)?;

    let (req, send_ctx) = SenderBuilder::from_psbt_and_uri(psbt.clone(), payjoin_uri)?
        .build_with_additional_fee(
            bitcoincore_rpc::bitcoin::Amount::from_sat(1),
            None,
            bitcoincore_rpc::bitcoin::FeeRate::MIN,
            true,
        )?
        .extract_v2(ohttp_relay)?;

    let res = https_agent()
        .post(req.url)
        .body(req.body)
        .header("Content-Type", payjoin::V2_REQ_CONTENT_TYPE)
        .send()
        .await?;

    send_ctx.process_response(&res.bytes().await?)?;

    let psbt = sender
        .wallet_process_psbt(&psbt.to_string(), None, None, None)?
        .psbt;

    println!("[PayjoinV2] Finalizing PSBT...");
    if let Some(tx) = sender.finalize_psbt(&psbt, Some(true))?.hex {
        println!("[PayjoinV2] Sending Tx...");
        let txid = sender.send_raw_transaction(&tx)?;

        println!(
            "[PayjoinV2] Snd(after): {:?}",
            get_client_balance(sender)?.to_btc()
        );
        println!(
            "[PayjoinV2] Rcv(after): {:?}",
            get_client_balance(receiver)?.to_btc()
        );

        Ok(txid)
    } else {
        Err("fail".into())
    }
}

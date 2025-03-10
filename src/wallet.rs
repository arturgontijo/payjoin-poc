use bdk_wallet::{
    bitcoin::{
        bip32::Xpriv,
        key::rand::{thread_rng, Rng},
        Amount, Network,
    },
    template::Bip84,
    KeychainKind, LocalOutput, Wallet,
};

use bitcoincore_rpc::{Client, RpcApi};

pub fn create_wallet(seed_bytes: &[u8]) -> Result<Wallet, Box<dyn std::error::Error>> {
    let network = Network::Signet;

    let xprv = Xpriv::new_master(network, seed_bytes)
        .map_err(|e| format!("Failed to derive master secret: {}", e))?;

    let descriptor = Bip84(xprv, KeychainKind::External);
    let change_descriptor = Bip84(xprv, KeychainKind::Internal);

    let wallet = Wallet::create(descriptor, change_descriptor)
        .network(network)
        .create_wallet_no_persist()
        .map_err(|e| format!("Failed to set up wallet: {}", e))?;

    Ok(wallet)
}

pub fn fund_wallet(
    client: &Client,
    wallet: &mut Wallet,
    amount: Amount,
    utxos: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = thread_rng();
    for _ in 0..utxos {
        let address = wallet.reveal_next_address(KeychainKind::External).address;
        // range -15% and +15%
        let variation_factor = rng.gen_range(-0.15..=0.15);
        let amount_u64 = amount.to_sat();
        let random_amount = amount_u64 as f64 * (1.0 + variation_factor);
        client.send_to_address(
            &address,
            Amount::from_sat(random_amount.round() as u64),
            None,
            None,
            None,
            None,
            None,
            None,
        )?;
        println!("SENDER(addr): {}", address);
    }
    Ok(())
}

pub fn sync_wallet(
    client: &Client,
    wallet: &mut Wallet,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let latest = client.get_block_count()?;
    let stored = wallet.latest_checkpoint().block_id().height as u64;
    if debug {
        println!(
            "    -> WalletSyncBlock: (stored={} | latest={})",
            stored, latest
        );
    }
    for height in stored..latest {
        let hash = client.get_block_hash(height)?;
        let block = client.get_block(&hash)?;
        wallet.apply_block(&block, height as u32)?;
    }
    if debug {
        println!("    -> WalletSyncBlock: Done!");
    }
    Ok(())
}

pub fn wallet_total_balance(
    bitcoind: &Client,
    wallet: &mut Wallet,
) -> Result<Amount, Box<dyn std::error::Error>> {
    sync_wallet(&bitcoind, wallet, false)?;
    let balance = wallet.balance();
    Ok(balance.total())
}

pub fn get_wallet_utxos(wallet: &Wallet) -> Vec<LocalOutput> {
    let utxos: Vec<_> = wallet.list_unspent().collect();
    utxos
}

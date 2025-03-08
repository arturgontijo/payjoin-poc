# P2PE (Payjoin)
Research on P2PE (Payjoin) to use it with ldk-node

## Signet Setup
Be sure to have an up and running signet at `0.0.0.0:38332`.
You can use [signet-local](https://github.com/arturgontijo/signet-local) to spawn it using Docker.

## Run

Payjoin "directly" -> Sender and Receiver changing a PSBT:
```bash
cargo run
# or
cargo run -- directly
```

Payjoin using [rust-payjoin](https://github.com/payjoin/rust-payjoin) V1:
```bash
cargo run -- v1
```

Payjoin using [rust-payjoin](https://github.com/payjoin/rust-payjoin) V2:
```bash
cargo run -- v2
```

Payjoin to open channel between 2 [ldk-node](https://github.com/lightningdevkit/ldk-node/):
```bash
cargo run -- ldk-open-channel
# You should delete the ./data/ in order to get a fresh re-run
rm -rf data && cargo run -- ldk-open-channel
```

[WIP] Payjoin Batch between wallets:
```bash
# Build a PSBT and circle it between wallets
cargo run -- batch 1
# Merge multiple PSBTs
cargo run -- batch 2
# Add foreign UTXOs to a PSBT
cargo run -- batch 3
# Circle serialized PSBTs (hex) between participants 
cargo run -- batch 4
# Introduce the "Pool of UTXOs" idea
cargo run -- batch 5
# Build a PSBT and circle it between wallets ensuring uniform output sizes. 
cargo run -- batch 6
```

Payjoin Batch between [ldk-node](https://github.com/lightningdevkit/ldk-node/):
```bash
cargo run -- ldk
# You should delete the ./data/ in order to get a fresh re-run
rm -rf data && cargo run -- ldk
```

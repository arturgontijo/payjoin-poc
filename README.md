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

<div align="center">
  <h1>DeWallet</h1>

  <p>Library for building blockchain / web3 apps.</p>
</div>

- Generation of mnemonic phrases
- Importing of mnemonic phrases
- Generation of Hierarchical Deterministic (HD) wallets
- Importing of HD wallets
- Support for Bitcoin, Ethereum, and other blockchains

## Installing Rust

Rust can be installed using your package manager of choice or
[rustup.rs](https://rustup.rs).

## Building

The library can be built and tested using [`cargo`](https://github.com/rust-lang/cargo/):

```
git clone git@github.com:walletd/walletd.git
cd walletd
cargo build
```

You can run tests with:

```
cargo test
```

Please refer to the [`cargo` documentation](https://doc.rust-lang.org/stable/cargo/) for more detailed instructions.

### Building the docs

Docs can be built using the following:

```
cargo doc --no-deps --open
```

</div>
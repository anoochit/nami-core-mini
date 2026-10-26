# Getting Started with Rust

Rust is a systems programming language focused on safety, speed, and concurrency.

## 1. Installing Rust

The official installer for Rust is `rustup`.

On Linux and macOS:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On Windows:
Download and run `rustup-init.exe` from <https://rustup.rs/>

Verify installation:

```bash
rustc --version
cargo --version
```

## 2. Managing Projects with Cargo

`cargo` is the Rust package manager and build tool.

- **Create a new binary project:**

  ```bash
  cargo new my_project
  ```

- **Compile and run during development:**

  ```bash
  cargo run
  ```

- **Quick syntax and type check without building a binary:**

  ```bash
  cargo check
  ```

- **Build an optimized release binary:**

  ```bash
  cargo build --release
  ```

## 3. Project File Layout

A standard Cargo project contains:

- `Cargo.toml`: Package manifest listing metadata and dependencies.
- `src/main.rs`: Entry point for executable binary crates.
- `src/lib.rs`: Entry point for library crates.

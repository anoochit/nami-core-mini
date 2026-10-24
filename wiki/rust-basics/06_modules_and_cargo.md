# Modules, Packages, and Dependencies

## 1. Packages and Crates

- **Crate:** A tree of modules that produces a library (`src/lib.rs`) or executable binary (`src/main.rs`).
- **Package:** One or more crates defined by a `Cargo.toml` file.

## 2. Module System

Modules control code organization, scope, and privacy.

```rust
// src/network.rs or mod network inside main.rs
pub mod client {
    pub fn connect() {
        println!("Connected");
    }
}
```

Items in modules are private by default. Use `pub` to make functions, structs, or fields public.

## 3. Bringing Paths into Scope with `use`

The `use` keyword brings paths into local scope:

```rust
use std::collections::HashMap;

fn main() {
    let mut map = HashMap::new();
    map.insert("key", "value");
}
```

## 4. Managing Dependencies in `Cargo.toml`

Add external crates from crates.io under `[dependencies]`:

```toml
[package]
name = "my_app"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.0", features = ["full"] }
```

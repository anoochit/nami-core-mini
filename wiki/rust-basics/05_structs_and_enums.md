# Structs, Enums, and Pattern Matching

## 1. Structs

Structs group related data fields:

```rust
struct User {
    username: String,
    email: String,
    active: bool,
}

let user1 = User {
    username: String::from("alice"),
    email: String::from("alice@example.com"),
    active: true,
};
```

### Implementing Methods

Methods are defined inside an `impl` block:

```rust
struct Rectangle {
    width: u32,
    height: u32,
}

impl Rectangle {
    fn area(&self) -> u32 {
        self.width * self.height
    }
}
```

## 2. Enums

Enums define a type with multiple variants:

```rust
enum Message {
    Quit,
    Move { x: i32, y: i32 },
    Write(String),
}
```

### Standard Library Enums

- **`Option<T>`:** Represents optional values (replaces null pointers):
  ```rust
  enum Option<T> {
      Some(T),
      None,
  }
  ```

- **`Result<T, E>`:** Handles recoverable errors:
  ```rust
  enum Result<T, E> {
      Ok(T),
      Err(E),
  }
  ```

## 3. Pattern Matching (`match` and `if let`)

The `match` expression provides exhaustive pattern matching:

```rust
fn process_message(msg: Message) {
    match msg {
        Message::Quit => println!("Quit"),
        Message::Move { x, y } => println!("Move to {x}, {y}"),
        Message::Write(text) => println!("Text: {text}"),
    }
}
```

Use `if let` for concise matching on single variants:

```rust
let some_number = Some(5);
if let Some(x) = some_number {
    println!("Number is {x}");
}
```

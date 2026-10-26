# Control Flow and Functions

## 1. Functions

Function definitions start with `fn`. Return types are indicated by `->`.

```rust
fn add(a: i32, b: i32) -> i32 {
    a + b // implicit return (expression without semicolon)
}
```

In Rust, the final expression in a function body is returned implicitly if there is no semicolon.

## 2. Expressions vs Statements

- **Statements** perform actions and do not return a value (e.g. `let x = 6;`).
- **Expressions** evaluate to a resulting value (e.g. `{ let x = 3; x + 1 }` evaluates to `4`).

## 3. Control Flow

### `if` Expressions

`if` branches are expressions and must evaluate to boolean conditions:

```rust
let number = 6;

let description = if number % 2 == 0 {
    "even"
} else {
    "odd"
};
```

### Loops

Rust provides three loop mechanisms:

- **`loop`:** Infinite loop, can yield a value with `break`:

  ```rust
  let mut counter = 0;
  let result = loop {
      counter += 1;
      if counter == 10 {
          break counter * 2;
      }
  };
  ```

- **`while`:** Conditional loop:

  ```rust
  let mut number = 3;
  while number > 0 {
      number -= 1;
  }
  ```

- **`for`:** Iterates over a collection or range:

  ```rust
  for item in 1..5 { // 1 to 4
      println!("{item}");
  }
  ```

# Ownership and Borrowing

Ownership is Rust's central memory management system, enabling memory safety without a garbage collector.

## 1. Rules of Ownership

1. Each value in Rust has an **owner** (a variable).
2. There can only be **one owner** at a time.
3. When the owner goes out of scope, the value is **dropped** (freed).

## 2. Move Semantics

Assigning a heap-allocated value (like `String`) to another variable transfers ownership (a "move"):

```rust
let s1 = String::from("hello");
let s2 = s1; // s1 is moved to s2

// println!("{}", s1); // Compile error: borrow of moved value `s1`
```

Types that implement the `Copy` trait (like integers or booleans) copy instead of moving.

## 3. Borrowing and References

Borrowing lets you access data without taking ownership.

- **Immutable Reference (`&T`):** Allows reading data without modifying it. Multiple immutable references are permitted simultaneously.
  ```rust
  fn calculate_length(s: &String) -> usize {
      s.len()
  }
  ```

- **Mutable Reference (`&mut T`):** Allows modifying the borrowed data.
  ```rust
  fn append_world(s: &mut String) {
      s.push_str(", world");
  }
  ```

## 4. The Borrowing Rules

To prevent data races at compile time:
1. You can have either **one mutable reference** or **any number of immutable references** at a given time.
2. References must always be valid (no dangling references).

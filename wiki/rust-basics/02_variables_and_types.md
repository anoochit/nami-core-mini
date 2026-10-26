# Variables and Primitive Types

## 1. Variables and Immutability

By default, variables in Rust are immutable.

```rust
let x = 5;
// x = 6; // Compile error: cannot assign twice to immutable variable
```

To allow modification, declare the variable with `mut`:

```rust
let mut x = 5;
x = 6; // Allowed
```

## 2. Constants and Shadowing

Constants are always immutable and must specify a type annotation:

```rust
const MAX_POINTS: u32 = 100_000;
```

Variable shadowing allows re-declaring a variable name in the same scope:

```rust
let x = 5;
let x = x + 1; // x is now 6
let x = x * 2; // x is now 12
```

## 3. Data Types

Rust is statically typed, but features type inference.

### Scalar Types

- **Integers:** Signed (`i8`, `i16`, `i32`, `i64`, `i128`, `isize`) and Unsigned (`u8`, `u16`, `u32`, `u64`, `u128`, `usize`). Default is `i32`.
- **Floating-point:** `f32` and `f64`. Default is `f64`.
- **Booleans:** `bool` (`true` or `false`).
- **Characters:** `char` (4-byte Unicode scalar value, written with single quotes e.g. `'a'`, `'🚀'`).

### Compound Types

- **Tuples:** Fixed-size grouping of multiple types:

  ```rust
  let tup: (i32, f64, u8) = (500, 6.4, 1);
  let (x, y, z) = tup; // destructuring
  let five_hundred = tup.0; // index access
  ```

- **Arrays:** Fixed length, single type allocated on the stack:

  ```rust
  let a: [i32; 5] = [1, 2, 3, 4, 5];
  let first = a[0];
  ```

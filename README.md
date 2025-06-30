TODO: Cancellation


# Testing

## Testing
```bash
$ cargo install cargo-fuzz
```
## Loom
```bash
RUSTFLAGS="--cfg loomtest" cargo test --test loom_ --release
```
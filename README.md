# asycnal
> Fast asynchronous signalling primitives for multithreaded and singlethreaded runtimes

<div align="center">

[![GitHub Actions Status](https://github.com/DiscordJim/asyncnal/actions/workflows/rust.yml/badge.svg)](https://github.com/DiscordJim/asyncnal/actions) · [![Crates.io Version](https://img.shields.io/crates/v/asyncnal.svg)](https://crates.io/crates/asyncnal) · [![Crates.io Downloads](https://img.shields.io/crates/d/asyncnal.svg)](https://crates.io/crates/asyncnal) · [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/DiscordJim/asyncnal/blob/main/LICENSE)

</div>

# Features
- Minimal `unsafe` code.
- Extremely fast under high-contention, built on efficient lock-free queueing. 
- Executor agnostic.
- Supports thread-per-core (TPC) and local runtimes.

# Quickstart
You can get started with the following code:
```rust
use asyncnal::{Event, EventSetter};

let event = Event::new();
assert!(!event.has_waiters());

// We'll pre-set the event.
event.set_one();
 
// This will immediately return.
event.wait().await;
```
For more information, please read [the documentation](https://docs.rs/asyncnal) which contains an extensive set of examples on how this crate can be used along with the variants contained within the crate.

# Testing
First, there are quite a few feature options on this crate, so if you are making
any changes I would highly recommend 
```bash
$ cargo install cargo-all-features
$ cargo check-all-features
```
Normal tests can be run as follows:
```bash
$ cargo test
```
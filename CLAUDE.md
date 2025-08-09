# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This repository contains Quinn, a pure-Rust implementation of the IETF QUIC transport protocol. The project consists of several crates:

1. **quinn** - High-level async API based on tokio for most developers
2. **quinn-proto** - Deterministic state machine of the protocol with no I/O
3. **quinn-udp** - UDP sockets with ECN information tuned for QUIC

## Common Development Commands

### Building
```bash
cargo build
cargo build --all-features
cargo build --release
```

### Testing
```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p quinn
cargo test -p quinn-proto
cargo test -p quinn-udp

# Run tests with specific features
cargo test --features rustls-aws-lc-rs

# Run ignored stress tests
cargo test -- --ignored stress

# Run benchmarks as tests
cargo test -p quinn-udp --benches

# Run fuzz tests
cargo test --manifest-path fuzz/Cargo.toml

# Test specific platforms
cargo test --target wasm32-unknown-unknown
```

### Linting and Formatting
```bash
# Check formatting
cargo fmt --all -- --check

# Run clippy lints
cargo clippy --all-targets -- -D warnings

# Check documentation
cargo doc --no-deps --document-private-items
```

### Running Examples
```bash
# Start server
cargo run --example server ./

# Run client
cargo run --example client https://localhost:4433/Cargo.toml
```

## Code Architecture

### Quinn Crate (High-level API)
- Main entry point: `Endpoint` struct for creating clients and servers
- Connection management with `Connection` struct
- Stream handling with `SendStream` and `RecvStream`
- Support for multiple async runtimes (tokio, async-std, smol)

### Quinn-proto Crate (Protocol State Machine)
- Core QUIC protocol implementation without I/O
- Connection state management
- Packet processing and frame handling
- Congestion control algorithms
- Cryptography integration points

### Quinn-udp Crate (Low-level Networking)
- Platform-specific UDP socket implementations
- ECN (Explicit Congestion Notification) support
- GRO (Generic Receive Offload) and GSO (Generic Segmentation Offload)
- Cross-platform abstractions for advanced UDP features

## Feature Flags

Key feature flags for development:
- `rustls-ring` - Default TLS provider using ring
- `rustls-aws-lc-rs` - Alternative TLS provider using aws-lc-rs
- `runtime-tokio` - Tokio async runtime (default)
- `runtime-async-std` - Async-std runtime support
- `runtime-smol` - Smol runtime support
- `bloom` - Enables BloomTokenLog for token validation
- `lock_tracking` - Records how long locks are held and warns if held >= 1ms
- `platform-verifier` - Provides `ClientConfig::with_platform_verifier()` convenience method
- `qlog` - Enable qlog support for protocol analysis

## Testing Different Platforms

The project is tested on multiple platforms including:
- Linux, macOS, Windows (primary)
- FreeBSD, NetBSD, Solaris, Illumos
- Android (via NDK)
- WebAssembly (wasm32-unknown-unknown)

## Minimum Supported Rust Version (MSRV)

Current MSRV is 1.74.1, as specified in the workspace configuration.
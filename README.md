
[![Build Status](https://github.com/nehemming/aggbook/actions/workflows/release.yml/badge.svg?branch=main)](https://github.com/nehemming/aggbook/actions/workflows/release.yml)
[![codecov](https://codecov.io/gh/nehemming/aggbook/graph/badge.svg?token=aLearRYwfl)](https://codecov.io/gh/nehemming/aggbook)
[![License](https://img.shields.io/github/license/nehemming/aggbook.svg)](https://github.com/nehemming/aggbook/blob/main/LICENSE)

> ⚠️ **Beta Release**: This library is currently under active development. Its components and behavior are subject to change. Feedback and issues are welcome.
It is intended to demonstrate implementation technique rather than hardened production code.

# aggbook

aggbook is a Rust-based system for real-time aggregation of cryptocurrency order books from multiple exchanges. It connects to exchange WebSocket feeds, merges order book data for a specific trading pair, and publishes a combined view via a gRPC server. This project was originally developed as part of a technical challenge focused on high-performance market data streaming.

## Features

- Connects to Binance and Bitstamp WebSocket feeds concurrently
- Streams level 2 order book data for a configurable trading pair (e.g. ETHBTC)
- Merges and sorts the order books into a single aggregated view
- Publishes the aggregated top 10 bids and asks along with the spread
- Streams results over a gRPC service
- Includes gRPC endpoint for both default and per-symbol order book requests
- Measures and includes latency from WebSocket read to gRPC transmission (50µs to 1.5ms)
- Supports multiple concurrent gRPC clients
- Designed to support multiple trading symbols (server already supports this; client can be extended)
- Modular architecture: separate crates for client, server, and shared components

## Folder Structure

```
.
├── Cargo.toml
├── Dockerfile.client
├── Dockerfile.server
├── Makefile
├── crates
│   ├── aggclient         # gRPC client
│   ├── aggcommon         # Shared code and Protobuf definitions
│   └── aggserver         # gRPC server, exchange handlers, aggregator
└── docker-compose.yml    # Docker setup for client and server
```

## Protobuf Schema

aggbook uses the following gRPC schema for publishing the aggregated order book stream:

```proto
syntax = "proto3";
package orderbook;

service OrderbookAggregator {
    rpc BookSummary(Empty) returns (stream Summary);
    rpc BookSummaryForSymbol(SummaryRequest) returns (stream Summary);
}

message Empty {}

message SummaryRequest {
    string symbol = 1;
}

message Summary {
    double spread = 1;
    repeated Level bids = 2;
    repeated Level asks = 3;
    uint64 arrival_time = 40; // Time in nanoseconds since epoch of data arriving in grpc service.
}

message Level {
    string exchange = 1;
    double price = 2;
    double amount = 3;
}
```

## Getting Started

### Prerequisites

- Rust (2024 edition or later)
- protoc (Protocol Buffers compiler)
- Docker (optional, for containerized execution)

### Building the Project

To build all components locally and run tests:

make all

To build without running tests:

make local

### Running Locally

To start the gRPC server:

```sh
target/debug/aggserver --port 8099
```

To run the client and connect to the server:

```sh
target/debug/aggclient --server-addr http://localhost:8099
```

The client connects using the BookSummary endpoint and prints a live stream of the aggregated order book for the default symbol.

### Using Docker for Local Testing

Docker support is provided to test the system in an isolated environment. Use the following targets from the Makefile:

Build Docker images:

```
make docker-client  
make docker-server
```
Start the client and server with Docker Compose:

```
make docker-up
```

Stop and remove Docker containers:

```
make docker-down
```

The gRPC service runs on port 8099 by default, configurable via the PORT variable in the Makefile.

## Performance Notes

aggbook includes end-to-end latency measurements. The time between WebSocket message receipt and gRPC stream publication typically ranges between 50 microseconds and 1.5 milliseconds. Each Summary message includes an arrival_time field to support analysis and monitoring.

## Implementation Notes

The implementation is based on prior experience building market data feed handlers, but was developed from scratch in Rust. Copilot / ChatGPT were used to assist in development.  The design is original, idiomatic and reflects real-world production architectures. Further optimizations and refinements are possible with additional time and profiling.

## License

This project is licensed under the MIT License.

# IEEE 2030.5 Client (Smart Energy Profile 2.0) (SEP2)

`sep2_client` is a (WIP) Rust library for developing IEEE 2030.5 compliant clients on Linux[^1] based operating systems.

It relies on, and should be used alongside, the [`sep2_common`](https://github.com/ethanndickson/IEEE-2030.5-Common) crate, and it's implementation of the IEEE 2030.5 XSD.

This crate uses async rust, and currently only supports the [`tokio`](https://github.com/tokio-rs/tokio) runtime.

# Contents

[`client`](client) - Implementation of an IEEE 2030.5 Client Library, including documentation & examples

[`test_server`](test_server) - Dumb IEEE 2030.5 Server for testing

[`docs`](docs) - Thesis Project Reports & Seminars

[`IEEE-2030.5-Common`](IEEE-2030.5-Common) - Git submodule for the IEEE 2030.5 Rust server & client common library

# Progress
### Core Features
- [x] Application Support Function Set (TCP, HTTP)
- [x] Security Function Set (TLS + Certificate Verification, HTTPS)
- [x] IEEE 2030.5 Base Client Capabilities (GET, POST, PUT, DELETE) 
- [x] Asynchronous Resource Polling
- [x] Notification / Subscription Client Server Mechanism
- [x] Global Time Offset (Server Time Sync)
- [x] Event Scheduler
  - [x] DER
  - [x] DRLC
  - [x] Messaging
  - [x] Flow Reservation
  - [x] Pricing
  - [x] Per-Schedule Time Offset 
- [x] Tests / Documentation
  - [x] IEEE 2030.5 Examples as System Tests
  - [x] Event Scheduler Tests
  - [x] Subscription/Notification Tests
  - [x] DER Non-Aggregate Client Sample Impl.
- [x] Australian CSIP Extensions
### Future
- [ ] DNS-SD
- [ ] [rustls ECDHE-ECDSA-AES128-CCM8 Support](https://github.com/rustls/rustls/issues/1034)


# Examples
A client that synchronises it's time with the server:
```rust
use sep2_client::{client::Client, time::update_time_offset};
use sep2_common::packages::{dcap::DeviceCapability, time::Time};

#[tokio::main]
async fn main() {
    // Create a HTTPS client for a specific server
    let client = Client::new_https(
        "https://127.0.0.1:1337",
        "client_cert.pem",
        "client_private_key.pem",
        "serca.pem",
        // No KeepAlive
        None,
        // Default Poll Tick Rate (10 minutes)
        None,
    )
    .expect("Couldn't create client");
    let dcap = client
        .get::<DeviceCapability>("/dcap")
        .await
        .expect("Couldn't retrieve dcap");
    let time_link = &dcap.time_link.unwrap();
    let time = client.get::<Time>(&time_link.href).await.unwrap();
    // Sync client time
    update_time_offset(time);
}
```

More comprehensive examples can be found in the [`client/examples`](client/examples) directory

# Cargo Features
Features can be enabled or disabled through your crate's Cargo.toml

```toml
[dependencies.sep2_client]
features = ["der","pubsub"]
```

### Full list of features
- `default`: All mandatory IEEE 2030.5 Client function sets. Application Support, Security & Time.
- `event`: A Generic Event Schedule interface
- `der`: A Scheduler for DER Function Set Events
- `pricing`: A Scheduler for Pricing Function Set Events
- `messaging`: A Scheduler for Messaging Function Set Events
- `drlc`: A Scheduler for DRLC Function Set Events
- `pubsub`: A lightweight server for the Subscription / Notification function set.
- `csip_aus`: CSIP-AUS Extensions
- `all`: All of the above


# Dependencies
Due to the security requirements of IEEE 2030.5, this library only supports TLS using OpenSSL.
To use this library you will require a local installation of OpenSSL with support for `ECDHE-ECDSA-AES128-CCM8`.


[^1]: The library happens to performs as expected on macOS. If you would like to test the client locally on macOS, ensure `openssl` does not refer to `libressl`, as is the case by default. 

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms
or conditions.
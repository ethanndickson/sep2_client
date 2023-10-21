# IEEE 2030.5 Client (Smart Energy Profile 2.0) (SEP2)

`sep2_client` is a (WIP) Rust library for developing IEEE 2030.5 compliant clients on Linux based operating systems.

It relies on, and should be used alongside, the [`sep2_common`](https://github.com/ethanndickson/IEEE-2030.5-Common) crate, and it's implementation of the IEEE 2030.5 XSD.

# Contents

`client` - Implementation of an IEEE 2030.5 Client Library, including documentation & examples

`test_server` - Dumb IEEE 2030.5 Server for testing

`docs` - Thesis Project Reports & Seminars

`IEEE-2030.5-Common` - Git submodule for the IEEE 2030.5 Rust server & client common library

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
### Future
- [ ] DNS-SD
- [ ] Australian CSIP Extensions
- [ ] [rustls ECDHE-ECDSA-AES128-CCM8 Support](https://github.com/rustls/rustls/issues/1034)
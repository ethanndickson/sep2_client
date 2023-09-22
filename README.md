# IEEE 2030.5 Client

`client` - Implementation of an IEEE 2030.5 Client Library

`server` - Dumb IEEE 2030.5 Server for testing

`docs` - Project Reports & Seminars

`IEEE-2030.5-Common` - Git submodule for the IEEE 2030.5 Rust server-client common library

# Progress
### Core
- [x] Application Support Function Set (TCP, HTTP)
- [x] Security Function Set (TLS, HTTPS, Certificate Management)
- [x] IEEE 2030.5 Base Client Capabilities (GET, POST, PUT, DELETE) 
- [x] Asynchronous Resource Polling
- [x] Notification / Subscription Client Server Mechanism
- [ ] Event Scheduler
  - [x] DER
  - [ ] DRLC
  - [ ] Messaging
  - [ ] Flow Reservation
  - [ ] Pricing
- [ ] Global Time Offset (Server Time Sync)
- [ ] Tests / Documentation
  - [x] IEEE 2030.5 Examples as System Tests
  - [ ] Event Scheduler Tests
  - [ ] Subscription/Notification Tests
  - [x] DER Non-Aggregate Client Sample Impl.
### Stretch Goals
- [ ] DNS-SD
- [ ] Australian CSIP Extensions
- [ ] Rust EXI Library
- [ ] [rustls ECDHE-ECDSA-AES128-CCM8 Support](https://github.com/rustls/rustls/issues/1034)


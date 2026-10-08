//! SMS module: PDU codec, message store, send/receive service and API.
//!
//! The module owns everything SMS-specific:
//!
//! ```text
//! commands.rs  AT+CMGF/CMGS/CMGL/CMGD/CPMS/CSCA construction
//! parser.rs    AT response -> domain model (PDU list, storage, centre, IMS)
//! state.rs     SmsMessage / SmsStorage / SmsSettings domain model
//! service.rs   send + list + storage + IMS transactions through the scheduler
//! pdu.rs       SMS-SUBMIT encoder and SMS-DELIVER decoder (GSM 7-bit / UCS-2)
//! ussd.rs      AT+CUSD codec: GSM 7-bit packing, the reply decoder, the copy
//! api.rs       routes exposed to LuCI, WebUI and the CLI
//! ```
//!
//! It never touches the serial port: outgoing messages are handed to the
//! scheduler as an AT request, and the daemon is the only writer on the tty.

pub mod api;
pub mod commands;
pub mod parser;
pub mod pdu;
pub mod service;
pub mod state;
pub mod ussd;

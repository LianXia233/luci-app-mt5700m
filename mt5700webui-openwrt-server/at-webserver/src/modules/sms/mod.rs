//! SMS module: PDU codec, message store, send/receive service and API.
//!
//! The module owns everything SMS-specific:
//!
//! ```text
//! commands.rs  AT+CMGF/CMGS/CMGL/CMGD construction
//! parser.rs    AT response -> domain model (text mode and PDU mode)
//! state.rs     SmsMessage / SmsState domain model
//! service.rs   send + refresh transactions through the AT scheduler
//! pdu.rs       SMS-SUBMIT PDU encoder (GSM 7-bit / UCS-2 / multipart)
//! api.rs       routes exposed to LuCI, WebUI and the CLI
//! ```
//!
//! It never touches the serial port: outgoing messages are handed to the
//! scheduler as an AT request, and the daemon is the only writer on the tty.

pub mod pdu;

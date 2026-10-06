//! `+CMGL` / `+CPMS` / `+CSCA` / `^IMSSWITCH` / `+CMGF` decoders.
//!
//! Everything the SMS pages used to regex out of a reply — the PDU list, the
//! storage planes, the centre number, the IMS switch — is decoded here, once.

use crate::modules::sms::pdu;
use crate::modules::sms::state::{SmsMessage, SmsStorage, SmsStorageSlot};

/// `+CMGF: 0` -> the message format (0 = PDU, 1 = text).
pub fn parse_cmgf(raw: &str) -> Option<i64> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("+CMGF:"))?
        .trim();
    body.split(',').next()?.trim().parse::<i64>().ok()
}

/// `+CSCA: "+8613800138000",145` -> the centre number.
pub fn parse_csca(raw: &str) -> Option<String> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("+CSCA:"))?
        .trim();
    let number = body.split(',').next()?.trim().trim_matches('"');
    if number.is_empty() {
        None
    } else {
        Some(number.to_string())
    }
}

/// `^IMSSWITCH: <on>,<a>,<b>` -> whether SMS over IMS is enabled.
pub fn parse_imsswitch(raw: &str) -> Option<bool> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^IMSSWITCH:"))?
        .trim();
    body.split(',').next()?.trim().parse::<i64>().ok().map(|n| n == 1)
}

/// `+CPMS: "ME",3,50,"ME",3,50,"SM",0,30` -> the three storage planes.
///
/// Some firmware answers with only one or two planes; a plane it did not report
/// keeps an empty name, and the settings card then shows what it got.
pub fn parse_cpms(raw: &str) -> SmsStorage {
    let Some(body) = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("+CPMS:"))
    else {
        return SmsStorage::default();
    };
    let fields: Vec<String> = body
        .split(',')
        .map(|f| f.trim().trim_matches('"').to_string())
        .collect();
    let plane = |offset: usize| -> SmsStorageSlot {
        let name = fields.get(offset).cloned().unwrap_or_default();
        let used = fields
            .get(offset + 1)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        let total = fields
            .get(offset + 2)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        SmsStorageSlot { name, used, total }
    };
    SmsStorage {
        read: plane(0),
        write: plane(3),
        receive: plane(6),
    }
}

/// `+CMGL: <index>,<status>,,<length>` followed by the PDU line, repeated.
///
/// Every PDU is decoded by `pdu::decode_deliver`; a line that does not decode
/// (concatenation references the modem stored oddly, or a submit) is skipped
/// rather than rendered as an empty message. Multipart messages are merged into
/// one message here — the domain model is "a message", not "a part" — so the
/// frontends never reassemble PDUs.
pub fn parse_cmgl(raw: &str) -> Vec<SmsMessage> {
    let mut out: Vec<SmsMessage> = Vec::new();
    let mut pending_index: Option<i64> = None;

    for line_raw in raw.lines() {
        let line = line_raw.trim();
        if line.is_empty() || line == "OK" || line.starts_with("AT+") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("+CMGL:") {
            let index = rest
                .trim()
                .split(',')
                .next()
                .and_then(|v| v.trim().parse::<i64>().ok());
            pending_index = index;
            continue;
        }
        let Some(index) = pending_index.take() else {
            continue;
        };
        let Some(deliver) = pdu::decode_deliver(line) else {
            continue;
        };
        let mut msg = SmsMessage::received(index);
        msg.content = deliver.text;
        msg.number = deliver.number;
        msg.time = deliver.time;
        if let Some((reference, total, seq)) = deliver.concat {
            msg.is_concatenated = true;
            msg.concatenated_ref = Some(reference);
            msg.concatenated_total = Some(total);
            msg.concatenated_seq = Some(seq);
        }
        out.push(msg);
    }
    merge_concatenated(out)
}

/// Join the parts of one multipart message, in sequence order.
///
/// Parts of the same message share the sender, the concatenation reference and
/// the total part count (`+CMGL` order is not guaranteed, so they are sorted by
/// `concatenatedSeq`). The first part of the sequence contributes the index and
/// timestamp the list renders; the text is the concatenation. A part with no
/// reference is a single message and passes through untouched.
pub fn merge_concatenated(messages: Vec<SmsMessage>) -> Vec<SmsMessage> {
    let mut merged: Vec<SmsMessage> = Vec::new();
    let mut groups: std::collections::BTreeMap<String, Vec<SmsMessage>> =
        std::collections::BTreeMap::new();
    for msg in messages {
        match (msg.is_concatenated, msg.concatenated_ref, msg.concatenated_total) {
            (true, Some(reference), Some(total)) => {
                let key = format!("{}-{}-{}", msg.number, reference, total);
                groups.entry(key).or_default().push(msg);
            }
            _ => merged.push(msg),
        }
    }
    for (_, mut parts) in groups {
        parts.sort_by_key(|p| p.concatenated_seq.unwrap_or(0));
        let first = parts[0].clone();
        let content: String = parts.iter().map(|p| p.content.as_str()).collect();
        merged.push(SmsMessage {
            content,
            is_concatenated: true,
            ..first
        });
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_queries() {
        assert_eq!(parse_cmgf("+CMGF: 0\nOK"), Some(0));
        assert_eq!(parse_cmgf("+CMGF: 1\nOK"), Some(1));
        assert_eq!(parse_cmgf("OK"), None);
        assert_eq!(
            parse_csca("+CSCA: \"+8613800138000\",145\nOK").as_deref(),
            Some("+8613800138000")
        );
        assert_eq!(parse_csca("+CSCA: \"\"\nOK"), None);
        assert_eq!(parse_imsswitch("^IMSSWITCH: 1,0,0\nOK"), Some(true));
        assert_eq!(parse_imsswitch("^IMSSWITCH: 0,0,0\nOK"), Some(false));
        assert_eq!(parse_imsswitch("ERROR"), None);
    }

    #[test]
    fn cpms_planes() {
        let st = parse_cpms("+CPMS: \"ME\",3,50,\"ME\",3,50,\"SM\",0,30\nOK");
        assert_eq!(st.read.name, "ME");
        assert_eq!(st.read.used, 3);
        assert_eq!(st.read.total, 50);
        assert_eq!(st.write.name, "ME");
        assert_eq!(st.receive.name, "SM");
        assert_eq!(st.receive.total, 30);
        assert_eq!(st.names(), vec!["ME".to_string(), "SM".to_string()]);
        // A single-plane reply still fills the read plane.
        let one = parse_cpms("+CPMS: \"SM\",1,20\nOK");
        assert_eq!(one.read.name, "SM");
        assert!(one.write.name.is_empty());
        assert!(parse_cpms("ERROR").is_empty());
    }

    #[test]
    fn cmgl_lists_and_decodes_every_pdu() {
        let raw = "AT+CMGL=4\r\n\
            +CMGL: 1,1,,26\r\n\
            00040D91683108108300F000006280624182632305E8329BFD06\r\n\
            +CMGL: 2,1,,22\r\n\
            00040D91683108108300F0000862806241826323044F60597D\r\n\
            OK";
        let list = parse_cmgl(raw);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].index, 1);
        assert_eq!(list[0].content, "hello");
        assert_eq!(list[0].kind, "received");
        assert!(!list[0].is_concatenated);
        assert_eq!(list[1].index, 2);
        assert_eq!(list[1].content, "你好");
        assert_eq!(list[1].time, "26/08/26,14:28:36");
    }

    #[test]
    fn cmgl_keeps_concatenation_metadata_and_skips_bad_lines() {
        let raw = "AT+CMGL=4\r\n\
            +CMGL: 7,1,,23\r\n\
            00440D91683108108300F000006280624182632309050003AB0201D069\r\n\
            +CMGL: 8,1,,20\r\n\
            not-a-pdu\r\n\
            OK";
        let list = parse_cmgl(raw);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].index, 7);
        assert_eq!(list[0].content, "hi");
        assert!(list[0].is_concatenated);
        assert_eq!(list[0].concatenated_ref, Some(0xAB));
        assert_eq!(list[0].concatenated_seq, Some(1));
        assert_eq!(list[0].concatenated_total, Some(2));
    }

    #[test]
    fn multipart_parts_are_merged_in_sequence_order() {
        // Two parts of the same message (ref 0xAB, 1/2 and 2/2) plus the second
        // part listed first, which is what the modem sometimes answers.
        let raw = "AT+CMGL=4\r\n\
            +CMGL: 5,1,,23\r\n\
            00440D91683108108300F000006280624182632309050003AB0202F26F\r\n\
            +CMGL: 4,1,,23\r\n\
            00440D91683108108300F000006280624182632309050003AB0201D069\r\n\
            OK";
        let list = parse_cmgl(raw);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, "hiyo");
        assert_eq!(list[0].index, 4);
        assert!(list[0].is_concatenated);
        assert_eq!(list[0].concatenated_ref, Some(0xAB));
        assert_eq!(list[0].concatenated_total, Some(2));
        assert_eq!(list[0].concatenated_seq, Some(1));
        // Different senders never merge, even with the same reference: the
        // second part below comes from +8613900139000.
        let twice = parse_cmgl(
            "+CMGL: 1,1,,23\r\n\
             00440D91683108108300F000006280624182632309050003AB0201D069\r\n\
             +CMGL: 2,1,,23\r\n\
             00440D91683109109300F000006280624182632309050003AB0202F26F\r\n\
             OK",
        );
        assert_eq!(twice.len(), 2);
        assert_eq!(twice[1].content, "yo");
        assert_eq!(twice[1].number, "8613900139000");
    }

    #[test]
    fn empty_and_error_replies_yield_nothing() {
        assert!(parse_cmgl("OK").is_empty());
        assert!(parse_cmgl("ERROR").is_empty());
        assert!(parse_cmgl("+CMGL: 1,1,,26").is_empty());
    }
}

//! `^NRSSBID` decoder — the only place its offsets are known.
//!
//! Reply layout (manual 13.26):
//!
//! ```text
//! ^NRSSBID: <arfcn>,<cid>,<pci>,<rsrp>,<sinr>,<ta>,
//!           <ssb0 id>,<ssb0 rsrp>, … <ssb7 id>,<ssb7 rsrp>,
//!           <neighbor count>,
//!           <pci>,<arfcn>,<rsrp>,<sinr>,<ssb id>,<ssb rsrp> ×4   (per neighbour)
//! ```
//!
//! Beam slots the modem did not measure are sent as `255` / `32767` and are
//! dropped — the page showed only the beams it actually measured, and a slot
//! that is "not measured" is not a 32767 dBm signal.

use crate::core::radio::arfcn_to_band;
use crate::modules::beam::state::{SsbBeam, SsbNeighborCell, SsbServingCell, SsbState};

/// SSB id that means "not measured".
const NO_SSB: i64 = 255;
/// RSRP that means "not measured".
const NO_RSRP: i64 = 32767;
/// Serving-cell beam slots in the reply.
const SERVING_SLOTS: usize = 8;
/// Neighbour beam slots per neighbour.
const NEIGHBOR_SLOTS: usize = 4;
/// Fields per neighbour block in the reply.
const NEIGHBOR_STRIDE: usize = 12;

/// Decode an `^NRSSBID` reply.
///
/// `None` when the reply carries no `^NRSSBID:` line at all: the page then keeps
/// the previously displayed report instead of an empty card.
pub fn parse_ssbid(raw: &str) -> Option<SsbState> {
    let line = raw
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with("^NRSSBID:"))?;
    let body = line.trim_start_matches("^NRSSBID:").trim();
    let data: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    let text = |i: usize| -> Option<String> {
        data.get(i)
            .map(|v| v.trim_matches('"').trim())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
    };
    let number = |i: usize| -> Option<i64> { data.get(i).and_then(|v| v.parse::<i64>().ok()) };
    let beams = |start: usize, slots: usize| -> Vec<SsbBeam> {
        let mut out = Vec::new();
        for slot in 0..slots {
            let id = match number(start + slot * 2) {
                Some(v) => v,
                None => continue,
            };
            let rsrp = match number(start + slot * 2 + 1) {
                Some(v) => v,
                None => continue,
            };
            if id == NO_SSB || rsrp == NO_RSRP {
                continue;
            }
            out.push(SsbBeam {
                ssb_id: id,
                rsrp,
            });
        }
        out
    };

    // Both the serving cell and the neighbours are NR (`^NRSSBID`), so their
    // band comes from the one ARFCN table in `core::radio`.
    let band_of = |raw: Option<&String>| {
        raw.and_then(|s| s.parse::<i64>().ok())
            .and_then(|arfcn| arfcn_to_band("NR", arfcn))
    };
    let serving = SsbServingCell {
        arfcn: text(0),
        cid: text(1),
        pci: text(2),
        band: band_of(text(0).as_ref()),
        rsrp: number(3),
        sinr: number(4),
        ta: number(5),
        ssbs: beams(6, SERVING_SLOTS),
    };

    let mut neighbors = Vec::new();
    let count = number(6 + SERVING_SLOTS * 2).unwrap_or(0).max(0) as usize;
    let mut offset = 7 + SERVING_SLOTS * 2;
    for _ in 0..count {
        // A truncated reply ends the list instead of inventing cells.
        if offset + 3 >= data.len() {
            break;
        }
        let arfcn = text(offset + 1);
        neighbors.push(SsbNeighborCell {
            pci: text(offset),
            band: band_of(arfcn.as_ref()),
            arfcn,
            rsrp: number(offset + 2),
            sinr: number(offset + 3),
            ssbs: beams(offset + 4, NEIGHBOR_SLOTS),
        });
        offset += NEIGHBOR_STRIDE;
    }

    Some(SsbState {
        serving_cell: Some(serving),
        neighbors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The firmware's reply from the demo/mock fixture.
    const SAMPLE: &str = "^NRSSBID: 636648,1A2B3C,506,85,50,1,0,90,1,80,2,70,3,60,255,32767,255,32767,255,32767,255,32767,1,506,632448,88,45,0,90,1,80,2,70,255,32767\nOK";

    #[test]
    fn serving_cell_fields_and_measured_beams() {
        let st = parse_ssbid(SAMPLE).expect("report");
        let serving = st.serving_cell.expect("serving cell");
        assert_eq!(serving.arfcn.as_deref(), Some("636648"));
        assert_eq!(serving.cid.as_deref(), Some("1A2B3C"));
        assert_eq!(serving.pci.as_deref(), Some("506"));
        assert_eq!(serving.rsrp, Some(85));
        assert_eq!(serving.sinr, Some(50));
        assert_eq!(serving.ta, Some(1));
        // Four measured beams; the four 255/32767 slots are dropped.
        assert_eq!(serving.ssbs.len(), 4);
        assert_eq!(serving.ssbs[0].ssb_id, 0);
        assert_eq!(serving.ssbs[0].rsrp, 90);
        assert_eq!(serving.ssbs[3].ssb_id, 3);
        assert_eq!(serving.ssbs[3].rsrp, 60);
    }

    #[test]
    fn neighbor_cells_carry_their_own_beams() {
        let st = parse_ssbid(SAMPLE).expect("report");
        assert_eq!(st.neighbors.len(), 1);
        let nb = &st.neighbors[0];
        assert_eq!(nb.pci.as_deref(), Some("506"));
        assert_eq!(nb.arfcn.as_deref(), Some("632448"));
        assert_eq!(nb.band, Some(78));
        assert_eq!(nb.rsrp, Some(88));
        assert_eq!(nb.sinr, Some(45));
        assert_eq!(nb.ssbs.len(), 3);
        assert_eq!(nb.ssbs[2].ssb_id, 2);
        assert_eq!(nb.ssbs[2].rsrp, 70);
    }

    #[test]
    fn missing_line_is_none_and_truncated_reply_stops_cleanly() {
        assert!(parse_ssbid("OK").is_none());
        // Neighbour count says 2 but only one block arrived.
        let truncated = "^NRSSBID: 636648,1,506,85,50,1,255,32767,255,32767,255,32767,255,32767,255,32767,255,32767,255,32767,255,32767,2,506,632448,88,45";
        let st = parse_ssbid(truncated).expect("report");
        assert_eq!(st.neighbors.len(), 1);
        assert!(st.serving_cell.unwrap().ssbs.is_empty());
    }
}

//! AT commands owned by the beam module.

/// NR SSB (beam) measurement report: serving cell, its SSB strengths, and the
/// neighbour cells with theirs. A vendor `?` query with a long reply.
pub const NRSSBID: &str = "AT^NRSSBID?";

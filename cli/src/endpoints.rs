//! Futures API routes for the oracle (the futures routes live in
//! `futures::api`). Query values are percent-encoded here.

/// Every month's phase, sources and settlement status, with source inventories.
pub const ORACLE_SNAPSHOT: &str = "/v1/oracle/public-snapshot";
/// Each product's periods and settlement times.
pub const ORACLE_CATALOG: &str = "/v1/oracle/catalog";
/// The source-candidate store (at the API root, not under `/v1`).
pub const ORACLE_CANDIDATES: &str = "/oracle/source-candidates";

/// Observation records of comparison series (at the API root).
pub const ORACLE_OBSERVATIONS: &str = "/oracle/observations";

/// `/oracle/observations?expiryId=…&sourceId=…`: one source's records.
pub fn oracle_observations(expiry_id: &str, source_id: &str) -> String {
    format!(
        "{ORACLE_OBSERVATIONS}?expiryId={}&sourceId={}",
        percent_encode(expiry_id.trim()),
        percent_encode(source_id.trim())
    )
}

/// `/oracle/source-candidates?…` with the given filters.
pub fn oracle_candidates(filters: &[(&str, &str)], limit: u32) -> String {
    let mut params = vec![format!("limit={}", limit.clamp(1, 100))];
    for (key, value) in filters {
        params.push(format!("{key}={}", percent_encode(value.trim())));
    }
    format!("{ORACLE_CANDIDATES}?{}", params.join("&"))
}

/// `/oracle/source-candidates/<hash>`.
pub fn oracle_candidate(content_hash: &str) -> String {
    format!(
        "{ORACLE_CANDIDATES}/{}",
        percent_encode(content_hash.trim())
    )
}

pub(crate) fn percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                output.push(char::from(*byte));
            }
            other => output.push_str(&format!("%{other:02X}")),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_routes_are_trimmed_and_percent_encoded() {
        assert_eq!(
            oracle_candidates(&[("marketId", " ramx "), ("expiryId", "RAMX-202610")], 500),
            "/oracle/source-candidates?limit=100&marketId=ramx&expiryId=RAMX-202610"
        );
        assert_eq!(
            oracle_candidates(&[("agentId", "a b&c")], 0),
            "/oracle/source-candidates?limit=1&agentId=a%20b%26c"
        );
        assert_eq!(oracle_candidate(" ab "), "/oracle/source-candidates/ab");
        assert_eq!(
            oracle_observations("RAMX-202612", "ab"),
            "/oracle/observations?expiryId=RAMX-202612&sourceId=ab"
        );
    }
}

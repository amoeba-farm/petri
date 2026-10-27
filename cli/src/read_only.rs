//! Petri for an AI agent: `PETRI_MCP_READ_ONLY=1` (set by whoever runs
//! petri's commands for an agent) makes it read-only. It neither reads the
//! wallet's key, signs, nor updates itself. Agents trade through Amoeba's
//! hosted MCP server instead (the futures API's `/mcp`), which prepares
//! transactions for the owner's wallet and never signs.

fn read_only(value: Option<std::ffi::OsString>) -> bool {
    value.is_some_and(|v| v == "1")
}

fn restricted() -> bool {
    read_only(std::env::var_os("PETRI_MCP_READ_ONLY"))
}

/// Reading the wallet's key (its public key too) is refused.
pub fn identity_blocked() -> bool {
    restricted()
}

/// Signing is refused.
pub fn signing_blocked() -> bool {
    restricted()
}

#[cfg(test)]
mod tests {
    use super::read_only;

    #[test]
    fn only_one_turns_it_on() {
        assert!(read_only(Some("1".into())));
        for value in [None, Some("0".into()), Some("".into()), Some("true".into())] {
            assert!(!read_only(value.clone()), "{value:?}");
        }
    }
}

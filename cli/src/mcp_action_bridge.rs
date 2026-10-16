//! Resident agent transport. Authority is rebuilt and cleared for every request.
//! No shell, arbitrary CLI arguments, transaction bytes or wallet locators are accepted.
use crate::{backend::CliError, cli::Cli, mcp_actions};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};

const MAX_FRAME: usize = 26 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: u64,
    action: String,
    arguments: Value,
}

fn respond(cli: &Cli, bytes: &[u8]) -> Value {
    let request: Request = match serde_json::from_slice(bytes) {
        Ok(request) => request,
        Err(_) => {
            return json!({"id":null,"result":CliError::new("Invalid bounded agent request.").json()});
        }
    };
    let result = if request.action.len() > 160 || !request.arguments.is_object() {
        Err(CliError::new("Invalid public agent action."))
    } else {
        mcp_actions::invoke_captured(cli, &request.action, &request.arguments.to_string())
    };
    json!({"id":request.id,"result":result.unwrap_or_else(|error| error.json())})
}
pub(crate) fn serve(cli: &Cli) -> Result<(), CliError> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    loop {
        let mut bytes = Vec::new();
        let read = input
            .by_ref()
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .map_err(|_| CliError::new("Agent transport input unavailable."))?;
        if read == 0 {
            return Ok(());
        }
        if bytes.len() > MAX_FRAME {
            return Err(CliError::new("Agent transport frame exceeds its bound."));
        }
        let response = respond(cli, &bytes);
        serde_json::to_writer(&mut output, &response)
            .map_err(|_| CliError::new("Agent response unavailable."))?;
        output
            .write_all(b"\n")
            .and_then(|_| output.flush())
            .map_err(|_| CliError::new("Agent transport output unavailable."))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn request_authority_and_output_are_cleared_between_actions() {
        let cli = Cli::try_parse_from(["petri", "--json", "mcp", "serve"]).unwrap();
        for request in [
            json!({"id":1,"action":"futures.buy","arguments":{}}),
            json!({"id":2,"action":"unknown.action","arguments":{}}),
            json!({"id":3,"action":"wallet.address","arguments":{"ownerPubkey":"x"}}),
            json!({"id":4,"action":"wallet.address","arguments":{"keypair":"forbidden"}}),
            json!({"id":5,"action":"oracle.sources","arguments":{"marketId":"ramx"}}),
        ] {
            let result = respond(&cli, request.to_string().as_bytes());
            assert_eq!(result["id"], request["id"]);
            assert_eq!(result["result"]["ok"], false);
            assert!(!mcp_actions::active());
            assert!(!mcp_actions::capture_output(
                &json!({"mustNotPersist":true})
            ));
        }
        let bad = json!({"id":6,"action":"wallet.address","arguments":{},"command":"anything"});
        assert_eq!(respond(&cli, bad.to_string().as_bytes())["id"], Value::Null);
    }
}

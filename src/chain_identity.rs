use ameba_sdk::{
    CURRENT_LIGHT_TOKEN_CPI_AUTHORITY, CURRENT_LIVE_PROGRAMDATA, GovernanceGateStatusV1,
    ProtocolGovernanceGateV1,
    constants::{
        CURRENT_STATE_NAMESPACE_SEED, LIGHT_DEFAULT_ADDRESS_TREE_V2,
        LIGHT_TOKEN_COMPRESSIBLE_CONFIG, LIGHT_TOKEN_RENT_SPONSOR, VAULT_PDA_SEED,
    },
    protocol::{
        CURRENT_PROTOCOL_CLUSTER as CURRENT_CLUSTER,
        CURRENT_PROTOCOL_DEVNET_GENESIS_HASH as DEVNET_GENESIS_HASH,
    },
    state::VaultConfig,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use borsh::BorshDeserialize;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solana_program::{hash::hash, program_pack::Pack};
use solana_pubkey::Pubkey;
use solana_system_interface::program as system_program;
use spl_token::state::{Account as SplAccount, Mint};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    str::FromStr,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use crate::{
    backend::{BackendClient, CliError},
    catalog::{CatalogChainIdentity, catalog_chain_identity},
    current_release, endpoints,
    onchain::{OnchainConfig, resolve_rpc_url},
};

const IDENTITY_TIMEOUT: Duration = Duration::from_secs(15);
const IDENTITY_MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_LOCAL_CACHE_TTL: Duration = Duration::from_secs(30);
const MIN_IDENTITY_REMAINING: Duration = Duration::from_secs(2);
const PROGRAM_ID: &str = "2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw";
const PROGRAM_DATA_ADDRESS: &str = "8KR6hgcQehz32jm7CvrAriYNhvT2Bu9JuUWHce81J1oh";
const PROGRAM_LOADER_ID: &str = "BPFLoaderUpgradeab1e11111111111111111111111";
const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const LIGHT_TOKEN_PROGRAM_ID: &str = "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m";
const LIGHT_SYSTEM_PROGRAM_ID: &str = "SySTEM1eSU2p4BGQfQpimFEWWSC1XDFeun3Nqzz3rT7";
const LIGHT_ACCOUNT_COMPRESSION_PROGRAM_ID: &str = "compr6CUsB5m2jS4Y3831ztGSTnDpnKJTKS95d64XVq";
const LIGHT_COMPRESSIBLE_CONFIG_PROGRAM_ID: &str = "Lighton6oQpVkeewmo2mcPTQQp7kYHr4fWpAgJyEmDX";
const LIGHT_PROGRAM_UPGRADE_AUTHORITY: &str = "87k7P4H8dJPSEZ1mdA8gQDtiLWXRF6SiZzUMXsYJY7T1";
const LIGHT_COMPRESSIBLE_CONFIG_SIZE: usize = 310;
const LIGHT_ADDRESS_TREE_V2_SIZE: usize = 586_360;
const LIGHT_STATE_TREE_V2_SIZE: usize = 584_440;
const LIGHT_OUTPUT_QUEUE_V2_SIZE: usize = 962_536;
const LIGHT_CPI_CONTEXT_V2_SIZE: usize = 20_488;
const LIGHT_TREE_DISCRIMINATOR: &[u8; 8] = b"BatchMta";
const LIGHT_QUEUE_DISCRIMINATOR: &[u8; 8] = b"queueacc";
const LIGHT_CPI_CONTEXT_DISCRIMINATOR: &[u8; 8] = &[0x22, 0xb8, 0xb7, 0x0e, 0x64, 0x50, 0xb7, 0x7c];
const LIGHT_COMPRESSIBLE_CONFIG_DISCRIMINATOR: &[u8; 8] =
    &[0xb4, 0x04, 0xe7, 0x1a, 0xdc, 0x90, 0x37, 0xa8];
const CURRENT_NAMESPACE: &str = "ameba-spread-v2";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CurrentChainIdentityDto {
    pub state_namespace: String,
    pub cluster: String,
    pub genesis_hash: String,
    pub release_tag: String,
    pub release_commit: String,
    pub live_release_label: String,
    pub live_source_commit: Option<String>,
    pub live_read_profile_id: String,
    pub reviewed_bridge_source_commit: String,
    pub observed_slot: String,
    pub deployment_provenance: String,
    pub write_compatibility: String,
    pub program: CurrentProgramIdentityDto,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CurrentProgramIdentityDto {
    pub program_id: String,
    pub program_data_address: String,
    pub program_data_bytes: usize,
    pub upgrade_authority: String,
    pub executable: bool,
    pub deployed_slot: String,
    pub payload_bytes: usize,
    pub payload_sha256: String,
    pub raw_account_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CurrentIdentityFreshnessDto {
    source: String,
    observed_at: String,
    observed_at_slot: String,
    age_ms: u64,
    maximum_age_ms: u64,
    stale: bool,
    refresh_healthy: bool,
    read_ready: bool,
    trade_ready: bool,
    last_attempt_at: Option<String>,
    last_error_code: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ObservedAccountIdentity {
    pub owner: String,
    pub executable: bool,
}

#[derive(Clone, Debug)]
pub struct ObservedMintIdentity {
    pub owner: String,
    pub executable: bool,
    pub decimals: u8,
    pub initialized: bool,
    pub data_len: usize,
}

#[derive(Clone, Debug)]
pub struct ObservedVaultIdentity {
    pub paused: bool,
    pub owner: String,
    pub executable: bool,
    pub initialized: bool,
    pub quote_mint: String,
    pub custody_address: String,
    pub custody_owner: String,
    pub custody_mint: String,
    pub custody_authority: String,
    pub custody_initialized: bool,
    pub custody_data_len: usize,
    pub custody_amount_atomic: String,
}

#[derive(Clone, Debug)]
pub struct ObservedProgramIdentity {
    pub owner: String,
    pub executable: bool,
    pub program_account_bytes: usize,
    pub program_account_sha256: String,
    pub program_data_address: String,
    pub program_data_owner: String,
    pub program_data_executable: bool,
    pub program_data_bytes: usize,
    pub payload_bytes: usize,
    pub payload_sha256: String,
    pub program_data_account_sha256: String,
    pub deployed_slot: u64,
    pub upgrade_authority: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ObservedGovernanceIdentity {
    pub address: Pubkey,
    pub owner: Pubkey,
    pub executable: bool,
    pub account_sha256: String,
    pub bytes: Vec<u8>,
    pub gate: ProtocolGovernanceGateV1,
}

#[derive(Clone, Debug)]
pub struct ObservedLightProgramIdentity {
    pub owner: String,
    pub executable: bool,
    pub program_data_address: String,
    pub program_data_owner: String,
    pub program_data_executable: bool,
    pub program_data_bytes: usize,
    pub payload_bytes: usize,
    pub payload_sha256: String,
    pub deployed_slot: u64,
    pub upgrade_authority: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct LinkedProgramPin {
    program_id: &'static str,
    program_data_address: &'static str,
    program_data_bytes: usize,
    payload_bytes: usize,
    payload_sha256: &'static str,
    deployed_slot: u64,
}

const LINKED_PROGRAM_PINS: [LinkedProgramPin; 4] = [
    LinkedProgramPin {
        program_id: LIGHT_TOKEN_PROGRAM_ID,
        program_data_address: "GZ569jyYVcXnC6CLhA1ahfgNVZfSLtcMzrbcpjR438fy",
        program_data_bytes: 1_260_773,
        payload_bytes: 1_260_728,
        payload_sha256: "b553fa05658057b18e44e8b563b12c718f64a0dc778e3c384e4e1b80860c66fc",
        deployed_slot: 447_569_186,
    },
    LinkedProgramPin {
        program_id: LIGHT_SYSTEM_PROGRAM_ID,
        program_data_address: "Hohi6858RKfUZaS3BGTgKyX2Qt2wEDNey2aPZTXUChQz",
        program_data_bytes: 763_317,
        payload_bytes: 763_272,
        payload_sha256: "320360deb66a48c4a7a214d75e9032cb835b46b7535b9433117f460b412bb9bb",
        deployed_slot: 447_569_142,
    },
    LinkedProgramPin {
        program_id: LIGHT_ACCOUNT_COMPRESSION_PROGRAM_ID,
        program_data_address: "CyXYH8FjQgDnW32c5FiJrKsP5gwoqpaEPL6nHt5GGMMz",
        program_data_bytes: 891_845,
        payload_bytes: 891_800,
        payload_sha256: "10d40538964e25288c4134853bb09d396d80d07200e29a0f482707569336a821",
        deployed_slot: 448_021_881,
    },
    LinkedProgramPin {
        program_id: LIGHT_COMPRESSIBLE_CONFIG_PROGRAM_ID,
        program_data_address: "5GtYC3PY8YDoVvySNJez66Jpd1T3jsQwJX82A1GsudgR",
        program_data_bytes: 960_037,
        payload_bytes: 959_992,
        payload_sha256: "eec9e26044dbfecfadccf7b881d76d6463293572154b290768bda6ebb3682595",
        deployed_slot: 449_230_205,
    },
];

fn linked_program_pin(program_id: &str) -> Result<LinkedProgramPin, CliError> {
    LINKED_PROGRAM_PINS
        .iter()
        .copied()
        .find(|pin| pin.program_id == program_id)
        .ok_or_else(network_mismatch)
}

#[derive(Clone, Debug)]
pub struct ObservedNetworkIdentity {
    pub genesis_hash: String,
    pub observed_slot: u64,
    pub program: ObservedProgramIdentity,
    pub governance: ObservedGovernanceIdentity,
    pub quote_mint: ObservedMintIdentity,
    pub vault_config: Option<ObservedVaultIdentity>,
    pub light_accounts: BTreeMap<String, ObservedAccountIdentity>,
    pub light_programs: BTreeMap<String, ObservedLightProgramIdentity>,
}

#[derive(Clone, Debug)]
pub struct VerifiedNetworkIdentity {
    business_ready: bool,
    _verified: (),
    _observed_slot: u64,
}

impl VerifiedNetworkIdentity {
    pub fn require_initialized_business(&self) -> Result<(), CliError> {
        self.business_ready.then_some(()).ok_or_else(|| CliError::new(
            "Wallet changes are unavailable while Amoeba is uninitialized or paused. Nothing was prepared, signed, or sent. [CURRENT_PROGRAM_BUSINESS_UNAVAILABLE]"
        ))
    }
}

#[derive(Clone)]
struct CachedIdentity {
    expires_at: Instant,
    identity: VerifiedNetworkIdentity,
}

static IDENTITY_CACHE: OnceLock<Mutex<HashMap<String, CachedIdentity>>> = OnceLock::new();

fn network_mismatch() -> CliError {
    CliError::new(
        "Petri could not verify that this network matches the selected Amoeba deployment. Nothing was signed or sent. Choose a compatible network in Petri's configuration, then try again.",
    )
}

fn require_devnet(raw: &str) -> Result<(), CliError> {
    (raw.trim() == CURRENT_CLUSTER)
        .then_some(())
        .ok_or_else(network_mismatch)
}

fn current_program_id() -> Pubkey {
    ameba_sdk::ID
}

fn derive_vault_config_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[CURRENT_STATE_NAMESPACE_SEED, VAULT_PDA_SEED],
        &current_program_id(),
    )
}

fn sha256_hex(data: &[u8]) -> String {
    hash(data)
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical_u64(raw: &str) -> Option<u64> {
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    raw.parse().ok()
}

fn canonical_error_code(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 96
        && raw
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase())
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

pub(crate) fn validate_current_freshness(value: &Value) -> Result<(), CliError> {
    let freshness: CurrentIdentityFreshnessDto =
        serde_json::from_value(value.clone()).map_err(|_| {
            CliError::new("current snapshot freshness does not match the exact current projection")
        })?;
    if freshness.source != "redis-last-known-good"
        || canonical_u64(&freshness.observed_at_slot).is_none()
        || chrono::DateTime::parse_from_rfc3339(&freshness.observed_at).is_err()
        || freshness
            .last_attempt_at
            .as_deref()
            .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err())
        || freshness.maximum_age_ms == 0
        || freshness.stale != (freshness.age_ms > freshness.maximum_age_ms)
        || ((freshness.read_ready || freshness.trade_ready)
            && (!freshness.refresh_healthy || freshness.stale))
        || (freshness.trade_ready && !freshness.read_ready)
        || freshness
            .last_error_code
            .as_deref()
            .is_some_and(|value| !canonical_error_code(value))
    {
        return Err(CliError::new(
            "current snapshot freshness does not match the exact current projection",
        ));
    }
    Ok(())
}

fn exact_keys(value: &Value, expected: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

pub(crate) fn validate_current_protocol_data(data: &Value) -> Result<(), CliError> {
    let protocol = data.get("protocol").ok_or_else(network_mismatch)?;
    if !exact_keys(
        protocol,
        &[
            "programId",
            "namespace",
            "cluster",
            "releaseTag",
            "releaseCommit",
        ],
    ) || protocol.get("programId").and_then(Value::as_str) != Some(PROGRAM_ID)
        || protocol.get("namespace").and_then(Value::as_str) != Some(CURRENT_NAMESPACE)
        || protocol.get("cluster").and_then(Value::as_str) != Some(CURRENT_CLUSTER)
        || protocol.get("releaseTag").and_then(Value::as_str)
            != Some(current_release::READER_SEMANTIC_RELEASE)
        || protocol.get("releaseCommit").and_then(Value::as_str)
            != Some(current_release::READER_SEMANTIC_SOURCE_COMMIT)
    {
        return Err(network_mismatch());
    }
    Ok(())
}

pub(crate) fn validate_current_backend_envelope(payload: &Value) -> Result<(), CliError> {
    if payload.get("ok") != Some(&Value::Bool(true)) {
        return Err(network_mismatch());
    }
    let data = payload
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(network_mismatch)?;
    validate_current_protocol_data(data)
}

fn parse_current_identity_payload(payload: Value) -> Result<CurrentChainIdentityDto, CliError> {
    if !exact_keys(&payload, &["ok", "data"]) || payload.get("ok") != Some(&Value::Bool(true)) {
        return Err(network_mismatch());
    }
    let data = payload.get("data").ok_or_else(network_mismatch)?;
    if !exact_keys(data, &["protocol", "identity", "freshness"]) {
        return Err(network_mismatch());
    }
    validate_current_protocol_data(data)?;
    let identity: CurrentChainIdentityDto =
        serde_json::from_value(data.get("identity").cloned().ok_or_else(network_mismatch)?)
            .map_err(|_| network_mismatch())?;
    let freshness = data.get("freshness").ok_or_else(network_mismatch)?;
    validate_current_freshness(freshness).map_err(|_| network_mismatch())?;
    if freshness.get("observedAtSlot").and_then(Value::as_str)
        != Some(identity.observed_slot.as_str())
    {
        return Err(network_mismatch());
    }
    Ok(identity)
}

fn current_program_payload(program_data: &[u8]) -> Result<&[u8], CliError> {
    let selected = current_release::current_read_deployment()?;
    let payload_start = 45usize;
    let payload_end = payload_start
        .checked_add(selected.payload_bytes)
        .ok_or_else(network_mismatch)?;
    let payload = program_data
        .get(payload_start..payload_end)
        .ok_or_else(network_mismatch)?;
    let trailing_allocation = program_data
        .get(payload_end..)
        .ok_or_else(network_mismatch)?;
    if trailing_allocation.iter().any(|byte| *byte != 0) {
        return Err(network_mismatch());
    }
    Ok(payload)
}

// Payload means the whole allocated payload. ELF prefix and required padding are separate facts.
fn verify_executable_payload(
    payload: &[u8],
    artifact_bytes: usize,
    artifact_sha256: &str,
    padding_bytes: usize,
) -> Result<(), CliError> {
    if artifact_bytes == 0 || artifact_bytes.checked_add(padding_bytes) != Some(payload.len()) {
        return Err(network_mismatch());
    }
    let (artifact, padding) = payload.split_at(artifact_bytes);
    if sha256_hex(artifact) != artifact_sha256 || padding.iter().any(|byte| *byte != 0) {
        return Err(network_mismatch());
    }
    Ok(())
}

fn identity_http_client() -> Result<Client, CliError> {
    crate::backend::pinned_blocking_http_client_builder()
        .timeout(IDENTITY_TIMEOUT)
        .build()
        .map_err(|_| network_mismatch())
}

fn rpc_result(
    client: &Client,
    rpc_url: &str,
    method: &str,
    params: Value,
) -> Result<Value, CliError> {
    let mut response = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": format!("petri-current-identity:{method}"),
            "method": method,
            "params": params,
        }))
        .send()
        .map_err(|_| network_mismatch())?;
    if !response.status().is_success() || response.status().is_redirection() {
        return Err(network_mismatch());
    }
    if response
        .content_length()
        .is_some_and(|length| length > IDENTITY_MAX_RESPONSE_BYTES as u64)
    {
        return Err(network_mismatch());
    }
    let mut bytes = Vec::with_capacity(16 * 1024);
    response
        .by_ref()
        .take((IDENTITY_MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| network_mismatch())?;
    if bytes.len() > IDENTITY_MAX_RESPONSE_BYTES {
        return Err(network_mismatch());
    }
    let payload: Value = serde_json::from_slice(&bytes).map_err(|_| network_mismatch())?;
    if payload.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || payload.get("error").is_some()
    {
        return Err(network_mismatch());
    }
    payload.get("result").cloned().ok_or_else(network_mismatch)
}

fn account_value<'a>(values: &'a [Value], index: usize) -> Result<&'a Value, CliError> {
    values
        .get(index)
        .filter(|value| !value.is_null())
        .ok_or_else(network_mismatch)
}

fn account_data(value: &Value) -> Result<Vec<u8>, CliError> {
    let raw = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|data| data.first())
        .and_then(Value::as_str)
        .ok_or_else(network_mismatch)?;
    BASE64_STANDARD.decode(raw).map_err(|_| network_mismatch())
}

fn account_owner(value: &Value) -> Result<String, CliError> {
    value
        .get("owner")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(network_mismatch)
}

fn account_executable(value: &Value) -> Result<bool, CliError> {
    value
        .get("executable")
        .and_then(Value::as_bool)
        .ok_or_else(network_mismatch)
}

fn parse_pubkey_bytes(bytes: &[u8]) -> Result<String, CliError> {
    let raw: [u8; 32] = bytes.try_into().map_err(|_| network_mismatch())?;
    Ok(Pubkey::new_from_array(raw).to_string())
}

fn pubkey_at(data: &[u8], offset: usize) -> Result<String, CliError> {
    let end = offset.checked_add(32).ok_or_else(network_mismatch)?;
    parse_pubkey_bytes(data.get(offset..end).ok_or_else(network_mismatch)?)
}

fn u32_at(data: &[u8], offset: usize) -> Result<u32, CliError> {
    let end = offset.checked_add(4).ok_or_else(network_mismatch)?;
    Ok(u32::from_le_bytes(
        data.get(offset..end)
            .ok_or_else(network_mismatch)?
            .try_into()
            .map_err(|_| network_mismatch())?,
    ))
}

fn u64_at(data: &[u8], offset: usize) -> Result<u64, CliError> {
    let end = offset.checked_add(8).ok_or_else(network_mismatch)?;
    Ok(u64::from_le_bytes(
        data.get(offset..end)
            .ok_or_else(network_mismatch)?
            .try_into()
            .map_err(|_| network_mismatch())?,
    ))
}

fn validate_upgradeable_program_data(data: &[u8]) -> Result<(), CliError> {
    if data.len() < 13
        || u32_at(data, 0)? != 3
        || !matches!(data[12], 0 | 1)
        || (data[12] == 1 && data.len() < 45)
    {
        return Err(network_mismatch());
    }
    Ok(())
}

fn observe_program(
    program_value: &Value,
    program_data_value: &Value,
) -> Result<ObservedProgramIdentity, CliError> {
    let selected = current_release::current_read_deployment()?;
    let program_account_data = account_data(program_value)?;
    if account_owner(program_value)? != PROGRAM_LOADER_ID
        || !account_executable(program_value)?
        || program_account_data.len() != current_release::PROGRAM_ACCOUNT_BYTES
        || u32_at(&program_account_data, 0)? != 2
        || Pubkey::from_str(&parse_pubkey_bytes(&program_account_data[4..36])?).is_err()
    {
        return Err(network_mismatch());
    }
    let program_data_address = parse_pubkey_bytes(&program_account_data[4..36])?;
    let program_data_bytes = account_data(program_data_value)?;
    if account_owner(program_data_value)? != PROGRAM_LOADER_ID
        || account_executable(program_data_value)?
        || program_data_bytes.len() != selected.programdata_bytes
        || program_data_bytes[12] != 1
        || validate_upgradeable_program_data(&program_data_bytes).is_err()
    {
        return Err(network_mismatch());
    }
    let upgrade_authority = if program_data_bytes[12] == 1 {
        Some(parse_pubkey_bytes(&program_data_bytes[13..45])?)
    } else {
        None
    };
    let payload = current_program_payload(&program_data_bytes)?;
    verify_executable_payload(
        payload,
        selected.artifact_bytes,
        &selected.artifact_sha256,
        selected.mandatory_zero_padding_bytes,
    )?;
    Ok(ObservedProgramIdentity {
        owner: account_owner(program_value)?,
        executable: account_executable(program_value)?,
        program_account_bytes: program_account_data.len(),
        program_account_sha256: sha256_hex(&program_account_data),
        program_data_address,
        program_data_owner: account_owner(program_data_value)?,
        program_data_executable: account_executable(program_data_value)?,
        program_data_bytes: program_data_bytes.len(),
        payload_bytes: payload.len(),
        payload_sha256: sha256_hex(payload),
        program_data_account_sha256: sha256_hex(&program_data_bytes),
        deployed_slot: u64::from_le_bytes(
            program_data_bytes[4..12]
                .try_into()
                .map_err(|_| network_mismatch())?,
        ),
        upgrade_authority,
    })
}

fn observe_governance_gate(value: &Value) -> Result<ObservedGovernanceIdentity, CliError> {
    let selected = current_release::current_read_deployment()?;
    let owner = Pubkey::from_str(&account_owner(value)?).map_err(|_| network_mismatch())?;
    let executable = account_executable(value)?;
    let bytes = account_data(value)?;
    let gate = validate_selected_governance_gate(selected.gate, owner, executable, &bytes)?;
    Ok(ObservedGovernanceIdentity {
        address: selected.gate,
        owner,
        executable,
        account_sha256: sha256_hex(&bytes),
        bytes,
        gate,
    })
}

fn validate_selected_governance_gate(
    address: Pubkey,
    owner: Pubkey,
    executable: bool,
    bytes: &[u8],
) -> Result<ProtocolGovernanceGateV1, CliError> {
    if let Some(release) = current_release::selected_write_release()? {
        return ameba_sdk::validate_current_governed_gate_account_v1(
            &release,
            ameba_sdk::CurrentGovernedAccountObservationV1 {
                address,
                owner,
                executable,
                data: bytes,
            },
        )
        .map_err(|_| network_mismatch());
    }
    ameba_sdk::validate_current_governance_gate_account_v1(
        &address, &owner, executable, bytes, false,
    )
    .map_err(|_| network_mismatch())
}

fn observe_vault_config(
    client: &Client,
    rpc_url: &str,
    value: &Value,
) -> Result<Option<ObservedVaultIdentity>, CliError> {
    if value.is_null() {
        return Ok(None);
    }
    let owner = account_owner(value)?;
    let executable = account_executable(value)?;
    let data = account_data(value)?;
    if owner == system_program::ID.to_string() && !executable && data.is_empty() {
        return Ok(None);
    }
    if owner != PROGRAM_ID || executable || data.len() != VaultConfig::LEN {
        return Err(network_mismatch());
    }
    let mut remaining = data.as_slice();
    let config = VaultConfig::deserialize(&mut remaining).map_err(|_| network_mismatch())?;
    let (_, expected_bump) = derive_vault_config_pda();
    if !remaining.is_empty()
        || !config.is_initialized
        || config.bump != expected_bump
        || !config.has_current_layout()
    {
        return Err(network_mismatch());
    }
    let custody_result = rpc_result(
        client,
        rpc_url,
        "getMultipleAccounts",
        json!([[config.vault_token_account.to_string()], { "commitment": "finalized", "encoding": "base64" }]),
    )?;
    let custody_value = account_value(
        custody_result
            .get("value")
            .and_then(Value::as_array)
            .ok_or_else(network_mismatch)?,
        0,
    )?;
    let custody_data = account_data(custody_value)?;
    if account_owner(custody_value)? != TOKEN_PROGRAM_ID
        || account_executable(custody_value)?
        || custody_data.len() != SplAccount::LEN
    {
        return Err(network_mismatch());
    }
    let custody = SplAccount::unpack(&custody_data).map_err(|_| network_mismatch())?;
    Ok(Some(ObservedVaultIdentity {
        paused: config.paused,
        owner,
        executable,
        initialized: config.is_initialized,
        quote_mint: config.usdc_mint.to_string(),
        custody_address: config.vault_token_account.to_string(),
        custody_owner: custody.owner.to_string(),
        custody_mint: custody.mint.to_string(),
        custody_authority: derive_vault_config_pda().0.to_string(),
        custody_initialized: custody.state != spl_token::state::AccountState::Uninitialized,
        custody_data_len: custody_data.len(),
        custody_amount_atomic: custody.amount.to_string(),
    }))
}

fn observe_light_programs(
    client: &Client,
    rpc_url: &str,
) -> Result<BTreeMap<String, ObservedLightProgramIdentity>, CliError> {
    let ids = [
        LIGHT_TOKEN_PROGRAM_ID,
        LIGHT_SYSTEM_PROGRAM_ID,
        LIGHT_ACCOUNT_COMPRESSION_PROGRAM_ID,
        LIGHT_COMPRESSIBLE_CONFIG_PROGRAM_ID,
    ];
    let result = rpc_result(
        client,
        rpc_url,
        "getMultipleAccounts",
        json!([ids, { "commitment": "finalized", "encoding": "base64" }]),
    )?;
    let values = result
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(network_mismatch)?;
    let mut observed = BTreeMap::new();
    for (index, id) in ids.iter().enumerate() {
        let value = account_value(values, index)?;
        let data = account_data(value)?;
        if account_owner(value)? != PROGRAM_LOADER_ID
            || !account_executable(value)?
            || data.len() != 36
            || u32_at(&data, 0)? != 2
        {
            return Err(network_mismatch());
        }
        let program_data_address = parse_pubkey_bytes(&data[4..36])?;
        let pd_result = rpc_result(
            client,
            rpc_url,
            "getMultipleAccounts",
            json!([[program_data_address], { "commitment": "finalized", "encoding": "base64" }]),
        )?;
        let pd_value = account_value(
            pd_result
                .get("value")
                .and_then(Value::as_array)
                .ok_or_else(network_mismatch)?,
            0,
        )?;
        let pd_data = account_data(pd_value)?;
        let header_bytes = if pd_data.get(12) == Some(&1) { 45 } else { 13 };
        if account_owner(pd_value)? != PROGRAM_LOADER_ID
            || account_executable(pd_value)?
            || validate_upgradeable_program_data(&pd_data).is_err()
        {
            return Err(network_mismatch());
        }
        observed.insert(
            (*id).to_string(),
            ObservedLightProgramIdentity {
                owner: account_owner(value)?,
                executable: account_executable(value)?,
                program_data_address,
                program_data_owner: account_owner(pd_value)?,
                program_data_executable: account_executable(pd_value)?,
                program_data_bytes: pd_data.len(),
                payload_bytes: pd_data.len().saturating_sub(header_bytes),
                payload_sha256: sha256_hex(&pd_data[header_bytes..]),
                deployed_slot: u64_at(&pd_data, 4)?,
                upgrade_authority: if pd_data[12] == 1 {
                    Some(parse_pubkey_bytes(&pd_data[13..45])?)
                } else {
                    None
                },
            },
        );
    }
    Ok(observed)
}

fn observe_full_account(
    client: &Client,
    rpc_url: &str,
    address: &str,
) -> Result<(ObservedAccountIdentity, Vec<u8>), CliError> {
    let result = rpc_result(
        client,
        rpc_url,
        "getMultipleAccounts",
        json!([[address], { "commitment": "finalized", "encoding": "base64" }]),
    )?;
    let value = account_value(
        result
            .get("value")
            .and_then(Value::as_array)
            .ok_or_else(network_mismatch)?,
        0,
    )?;
    let data = account_data(value)?;
    Ok((
        ObservedAccountIdentity {
            owner: account_owner(value)?,
            executable: account_executable(value)?,
        },
        data,
    ))
}

fn raw_light_account<'a>(
    raw: &'a BTreeMap<String, (ObservedAccountIdentity, Vec<u8>)>,
    address: &str,
) -> Result<&'a (ObservedAccountIdentity, Vec<u8>), CliError> {
    raw.get(address).ok_or_else(network_mismatch)
}

fn exact_light_account_data<'a>(
    raw: &'a (ObservedAccountIdentity, Vec<u8>),
    owner: &str,
    length: usize,
) -> Result<&'a [u8], CliError> {
    if raw.0.owner != owner || raw.0.executable || raw.1.len() != length {
        return Err(network_mismatch());
    }
    Ok(raw.1.as_slice())
}

fn validate_light_account_layouts(
    local: &CatalogChainIdentity,
    raw: &BTreeMap<String, (ObservedAccountIdentity, Vec<u8>)>,
) -> Result<(), CliError> {
    let address_tree =
        light_account(local, "light_address_tree", 0).ok_or_else(network_mismatch)?;
    let address_data = exact_light_account_data(
        raw_light_account(raw, &address_tree)?,
        LIGHT_ACCOUNT_COMPRESSION_PROGRAM_ID,
        LIGHT_ADDRESS_TREE_V2_SIZE,
    )?;
    if address_data.get(..8) != Some(LIGHT_TREE_DISCRIMINATOR.as_slice())
        || u64_at(address_data, 8)? != 4
    {
        return Err(network_mismatch());
    }

    for index in 0..5 {
        let state_tree =
            light_account(local, "light_state_tree", index).ok_or_else(network_mismatch)?;
        let queue =
            light_account(local, "light_output_queue", index).ok_or_else(network_mismatch)?;
        let cpi_context =
            light_account(local, "light_cpi_context", index).ok_or_else(network_mismatch)?;
        let state_data = exact_light_account_data(
            raw_light_account(raw, &state_tree)?,
            LIGHT_ACCOUNT_COMPRESSION_PROGRAM_ID,
            LIGHT_STATE_TREE_V2_SIZE,
        )?;
        if state_data.get(..8) != Some(LIGHT_TREE_DISCRIMINATOR.as_slice())
            || u64_at(state_data, 8)? != 3
            || pubkey_at(state_data, 168)? != queue
        {
            return Err(network_mismatch());
        }
        let queue_data = exact_light_account_data(
            raw_light_account(raw, &queue)?,
            LIGHT_ACCOUNT_COMPRESSION_PROGRAM_ID,
            LIGHT_OUTPUT_QUEUE_V2_SIZE,
        )?;
        if queue_data.get(..8) != Some(LIGHT_QUEUE_DISCRIMINATOR.as_slice())
            || pubkey_at(queue_data, 160)? != state_tree
        {
            return Err(network_mismatch());
        }
        let cpi_data = exact_light_account_data(
            raw_light_account(raw, &cpi_context)?,
            LIGHT_SYSTEM_PROGRAM_ID,
            LIGHT_CPI_CONTEXT_V2_SIZE,
        )?;
        if cpi_data.get(..8) != Some(LIGHT_CPI_CONTEXT_DISCRIMINATOR.as_slice())
            || cpi_data
                .get(8..40)
                .is_none_or(|bytes| bytes.iter().any(|byte| *byte != 0))
            || pubkey_at(cpi_data, 40)? != state_tree
        {
            return Err(network_mismatch());
        }
    }

    let config_data = exact_light_account_data(
        raw_light_account(raw, &LIGHT_TOKEN_COMPRESSIBLE_CONFIG.to_string())?,
        LIGHT_COMPRESSIBLE_CONFIG_PROGRAM_ID,
        LIGHT_COMPRESSIBLE_CONFIG_SIZE,
    )?;
    if config_data.get(..8) != Some(LIGHT_COMPRESSIBLE_CONFIG_DISCRIMINATOR.as_slice())
        || config_data.get(8).copied() != Some(1)
        || !matches!(config_data.get(9).copied(), Some(0 | 1))
        || pubkey_at(config_data, 76)? != LIGHT_TOKEN_RENT_SPONSOR.to_string()
        || pubkey_at(config_data, 150)? != LIGHT_DEFAULT_ADDRESS_TREE_V2.to_string()
        || config_data
            .get(182..)
            .is_none_or(|bytes| bytes.iter().any(|byte| *byte != 0))
    {
        return Err(network_mismatch());
    }

    for address in [
        CURRENT_LIGHT_TOKEN_CPI_AUTHORITY.to_string(),
        LIGHT_TOKEN_RENT_SPONSOR.to_string(),
    ] {
        let identity = raw_light_account(raw, &address)?;
        if identity.0.owner != system_program::ID.to_string()
            || identity.0.executable
            || !identity.1.is_empty()
        {
            return Err(network_mismatch());
        }
    }
    Ok(())
}

fn observe_rpc_identity(
    rpc_url: &str,
    local: &CatalogChainIdentity,
) -> Result<ObservedNetworkIdentity, CliError> {
    let selected = current_release::current_read_deployment()?;
    let client = identity_http_client()?;
    let genesis_hash = rpc_result(&client, rpc_url, "getGenesisHash", json!([]))?
        .as_str()
        .map(str::to_string)
        .ok_or_else(network_mismatch)?;
    let vault_address = derive_vault_config_pda().0.to_string();
    let core_addresses = [
        local.program_id.clone(),
        CURRENT_LIVE_PROGRAMDATA.to_string(),
        selected.gate.to_string(),
        local.quote_mint.clone(),
        vault_address,
    ];
    let core = rpc_result(
        &client,
        rpc_url,
        "getMultipleAccounts",
        json!([core_addresses, { "commitment": "finalized", "encoding": "base64" }]),
    )?;
    let core_values = core
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(network_mismatch)?;
    if core_values.len() != core_addresses.len() {
        return Err(network_mismatch());
    }
    let observed_slot = core
        .get("context")
        .and_then(|value| value.get("slot"))
        .and_then(Value::as_u64)
        .ok_or_else(network_mismatch)?;
    let program_value = account_value(core_values, 0)?;
    let program_data_value = account_value(core_values, 1)?;
    let governance_value = account_value(core_values, 2)?;
    let quote_value = account_value(core_values, 3)?;
    let quote_data = account_data(quote_value)?;
    if quote_data.len() != Mint::LEN {
        return Err(network_mismatch());
    }
    let quote_mint = Mint::unpack(&quote_data).map_err(|_| network_mismatch())?;
    let vault_config = observe_vault_config(
        &client,
        rpc_url,
        core_values.get(4).ok_or_else(network_mismatch)?,
    )?;

    let mut raw_light_accounts = BTreeMap::new();
    let mut light_accounts = BTreeMap::new();
    for expected in &local.light_accounts {
        let (observed, data) = observe_full_account(&client, rpc_url, &expected.address)?;
        raw_light_accounts.insert(expected.address.clone(), (observed.clone(), data));
        light_accounts.insert(expected.address.clone(), observed);
    }
    for address in [
        LIGHT_TOKEN_COMPRESSIBLE_CONFIG.to_string(),
        CURRENT_LIGHT_TOKEN_CPI_AUTHORITY.to_string(),
        LIGHT_TOKEN_RENT_SPONSOR.to_string(),
    ] {
        let (observed, data) = observe_full_account(&client, rpc_url, &address)?;
        raw_light_accounts.insert(address, (observed, data));
    }
    validate_light_account_layouts(local, &raw_light_accounts)?;

    Ok(ObservedNetworkIdentity {
        genesis_hash,
        observed_slot,
        program: observe_program(program_value, program_data_value)?,
        governance: observe_governance_gate(governance_value)?,
        quote_mint: ObservedMintIdentity {
            owner: account_owner(quote_value)?,
            executable: account_executable(quote_value)?,
            decimals: quote_mint.decimals,
            initialized: quote_mint.is_initialized,
            data_len: quote_data.len(),
        },
        vault_config,
        light_accounts,
        light_programs: observe_light_programs(&client, rpc_url)?,
    })
}

fn light_account(local: &CatalogChainIdentity, role: &str, occurrence: usize) -> Option<String> {
    local
        .light_accounts
        .iter()
        .filter(|account| account.role == role)
        .nth(occurrence)
        .map(|account| account.address.clone())
}

fn validate_linked_program(
    expected_program_id: &str,
    observed: &ObservedNetworkIdentity,
) -> Result<(), CliError> {
    let pin = linked_program_pin(expected_program_id)?;
    let Some(actual) = observed.light_programs.get(expected_program_id) else {
        return Err(network_mismatch());
    };
    if actual.owner != PROGRAM_LOADER_ID
        || !actual.executable
        || actual.program_data_owner != PROGRAM_LOADER_ID
        || actual.program_data_executable
        || actual.program_data_address != pin.program_data_address
        || actual.program_data_bytes != pin.program_data_bytes
        || actual.payload_bytes != pin.payload_bytes
        || actual.payload_sha256 != pin.payload_sha256
        || actual.deployed_slot != pin.deployed_slot
        || actual.upgrade_authority.as_deref() != Some(LIGHT_PROGRAM_UPGRADE_AUTHORITY)
    {
        return Err(network_mismatch());
    }
    Ok(())
}

pub fn validate_network_identity(
    configured_network: &str,
    local: &CatalogChainIdentity,
    identity: &CurrentChainIdentityDto,
    observed: &ObservedNetworkIdentity,
) -> Result<(), CliError> {
    let selected = current_release::current_read_deployment()?;
    require_devnet(configured_network)?;
    let Some(identity_observed_slot) = canonical_u64(&identity.observed_slot) else {
        return Err(network_mismatch());
    };
    if local.cluster != CURRENT_CLUSTER
        || local.genesis_hash != DEVNET_GENESIS_HASH
        || local.program_id != PROGRAM_ID
        || identity.state_namespace != CURRENT_NAMESPACE
        || identity.cluster != CURRENT_CLUSTER
        || identity.genesis_hash != local.genesis_hash
        || identity.release_tag != current_release::READER_SEMANTIC_RELEASE
        || identity.release_commit != current_release::READER_SEMANTIC_SOURCE_COMMIT
        || identity.live_release_label != current_release::LIVE_RELEASE_LABEL
        || identity.live_source_commit != selected.source_commit
        || identity.live_read_profile_id != current_release::LIVE_READ_PROFILE_ID
        || identity.reviewed_bridge_source_commit != current_release::REVIEWED_BRIDGE_SOURCE_COMMIT
        || identity.deployment_provenance != current_release::DEPLOYMENT_PROVENANCE
        || identity.write_compatibility != current_release::WRITE_COMPATIBILITY
        || identity_observed_slot < selected.deployed_slot
        || identity_observed_slot > observed.observed_slot
    {
        return Err(network_mismatch());
    }
    let Some(deployed_slot) = canonical_u64(&identity.program.deployed_slot) else {
        return Err(network_mismatch());
    };
    if identity.program.program_id != PROGRAM_ID
        || identity.program.program_data_address != PROGRAM_DATA_ADDRESS
        || identity.program.program_data_bytes != selected.programdata_bytes
        || identity.program.upgrade_authority != selected.upgrade_authority
        || !identity.program.executable
        || deployed_slot != selected.deployed_slot
        || identity.program.payload_bytes != selected.payload_bytes
        || identity.program.payload_sha256 != selected.payload_sha256
        || identity.program.raw_account_sha256 != selected.programdata_sha256
        || observed.genesis_hash != identity.genesis_hash
        || observed.program.owner != PROGRAM_LOADER_ID
        || !observed.program.executable
        || observed.program.program_account_bytes != current_release::PROGRAM_ACCOUNT_BYTES
        || observed.program.program_account_sha256 != selected.program_sha256
        || observed.program.program_data_address != identity.program.program_data_address
        || observed.program.program_data_owner != PROGRAM_LOADER_ID
        || observed.program.program_data_executable
        || observed.program.program_data_bytes != identity.program.program_data_bytes
        || observed.program.payload_bytes != identity.program.payload_bytes
        || observed.program.payload_sha256 != identity.program.payload_sha256
        || observed.program.program_data_account_sha256 != identity.program.raw_account_sha256
        || observed.program.deployed_slot != deployed_slot
        || observed.program.upgrade_authority.as_deref()
            != Some(identity.program.upgrade_authority.as_str())
    {
        return Err(network_mismatch());
    }

    let decoded_gate = validate_selected_governance_gate(
        observed.governance.address,
        observed.governance.owner,
        observed.governance.executable,
        &observed.governance.bytes,
    )
    .map_err(|_| network_mismatch())?;
    if observed.governance.address != selected.gate
        || observed.governance.owner != selected.controller
        || observed.governance.executable
        || observed.governance.account_sha256 != sha256_hex(&observed.governance.bytes)
        || observed.governance.gate != decoded_gate
        || decoded_gate.epoch == 0
        || !matches!(
            decoded_gate.status,
            GovernanceGateStatusV1::Active
                | GovernanceGateStatusV1::FrozenForUpgrade
                | GovernanceGateStatusV1::EmergencyFrozen
        )
    {
        return Err(network_mismatch());
    }

    if observed.quote_mint.owner != TOKEN_PROGRAM_ID
        || observed.quote_mint.executable
        || !observed.quote_mint.initialized
        || observed.quote_mint.decimals != 6
        || observed.quote_mint.data_len != Mint::LEN
        || local.quote_decimals != 6
    {
        return Err(network_mismatch());
    }
    let expected_vault = derive_vault_config_pda().0.to_string();
    if let Some(actual) = &observed.vault_config {
        let Some(amount) = canonical_u64(&actual.custody_amount_atomic) else {
            return Err(network_mismatch());
        };
        if actual.owner != PROGRAM_ID
            || actual.executable
            || !actual.initialized
            || actual.quote_mint != local.quote_mint
            || actual.custody_owner != expected_vault
            || actual.custody_mint != local.quote_mint
            || actual.custody_authority != expected_vault
            || !actual.custody_initialized
            || actual.custody_data_len != SplAccount::LEN
            || actual.custody_amount_atomic != amount.to_string()
            || Pubkey::from_str(&actual.custody_address)
                .ok()
                .is_none_or(|address| address == Pubkey::default())
        {
            return Err(network_mismatch());
        }
    }

    validate_linked_program(LIGHT_TOKEN_PROGRAM_ID, observed)?;
    validate_linked_program(LIGHT_SYSTEM_PROGRAM_ID, observed)?;
    validate_linked_program(LIGHT_ACCOUNT_COMPRESSION_PROGRAM_ID, observed)?;
    validate_linked_program(LIGHT_COMPRESSIBLE_CONFIG_PROGRAM_ID, observed)?;
    if light_account(local, "light_address_tree", 0).as_deref()
        != Some(LIGHT_DEFAULT_ADDRESS_TREE_V2.to_string().as_str())
        || local
            .light_accounts
            .iter()
            .filter(|account| account.role == "light_state_tree")
            .count()
            != 5
    {
        return Err(network_mismatch());
    }
    if local.light_accounts.len() != observed.light_accounts.len() {
        return Err(network_mismatch());
    }
    for expected in &local.light_accounts {
        let Some(actual) = observed.light_accounts.get(&expected.address) else {
            return Err(network_mismatch());
        };
        if expected.owner != actual.owner
            || expected.executable != actual.executable
            || actual.executable
        {
            return Err(network_mismatch());
        }
    }
    Ok(())
}

fn read_current_identity(backend_url: &str) -> Result<CurrentChainIdentityDto, CliError> {
    let payload = BackendClient::new(backend_url)?.get(endpoints::chain_identity())?;
    parse_current_identity_payload(payload)
}

fn cache_key(config: &OnchainConfig, rpc_url: &str) -> String {
    hash(format!("{}|{}|{}", config.backend_url, rpc_url, config.network).as_bytes()).to_string()
}

pub fn verify_onchain_config(config: &OnchainConfig) -> Result<VerifiedNetworkIdentity, CliError> {
    let rpc_url = resolve_rpc_url(config)?;
    let key = cache_key(config, &rpc_url);
    let cache = IDENTITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock()
        && let Some(cached) = cache.get(&key)
        && cached.expires_at.saturating_duration_since(Instant::now()) > MIN_IDENTITY_REMAINING
    {
        return Ok(cached.identity.clone());
    }
    let verified = verify_onchain_config_fresh(config)?;
    let mut cache = cache.lock().map_err(|_| network_mismatch())?;
    cache.insert(
        key,
        CachedIdentity {
            expires_at: Instant::now() + MAX_LOCAL_CACHE_TTL,
            identity: verified.clone(),
        },
    );
    Ok(verified)
}

pub fn verify_onchain_config_fresh(
    config: &OnchainConfig,
) -> Result<VerifiedNetworkIdentity, CliError> {
    let rpc_url = resolve_rpc_url(config)?;
    let local = catalog_chain_identity().map_err(|_| network_mismatch())?;
    let identity = read_current_identity(&config.backend_url).map_err(|_| network_mismatch())?;
    let observed = observe_rpc_identity(&rpc_url, &local)?;
    validate_network_identity(&config.network, &local, &identity, &observed)?;
    Ok(VerifiedNetworkIdentity {
        business_ready: observed
            .vault_config
            .as_ref()
            .is_some_and(|vault| vault.initialized && !vault.paused),
        _verified: (),
        _observed_slot: observed.observed_slot,
    })
}

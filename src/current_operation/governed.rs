//! Atomic finalized deployment + complete business-account snapshots for the
//! SDK's opaque current-write admission. No caller selects a gate or appends ABI.

use super::*;
use ameba_sdk::{
    CurrentGovernedAccountObservationV1, CurrentGovernedObservationV1,
    CurrentGovernedOptionalAccountObservationV1, CurrentGovernedSigningObservationV1,
    CurrentGovernedWriteContextV1, CurrentGovernedWriteReleaseV1,
};

struct OwnedAccount {
    address: Pubkey,
    value: Option<(Pubkey, bool, Vec<u8>)>,
}

impl OwnedAccount {
    fn view(&self) -> Option<CurrentGovernedAccountObservationV1<'_>> {
        self.value.as_ref().map(
            |(owner, executable, data)| CurrentGovernedAccountObservationV1 {
                address: self.address,
                owner: *owner,
                executable: *executable,
                data,
            },
        )
    }
}

pub(super) struct FinalizedSnapshot {
    genesis: String,
    slot: u64,
    accounts: Vec<OwnedAccount>,
    business_indexes: Vec<usize>,
}

impl FinalizedSnapshot {
    pub(super) fn with_view<T>(
        &self,
        consume: impl FnOnce(CurrentGovernedSigningObservationV1<'_>) -> Result<T, CliError>,
    ) -> Result<T, CliError> {
        let required = |index: usize| {
            self.accounts[index]
                .view()
                .ok_or_else(|| operation_error("A required current deployment account is absent."))
        };
        let business = self
            .business_indexes
            .iter()
            .map(|index| {
                let account = &self.accounts[*index];
                CurrentGovernedOptionalAccountObservationV1 {
                    address: account.address,
                    account: account.view(),
                }
            })
            .collect::<Vec<_>>();
        consume(CurrentGovernedSigningObservationV1 {
            deployment: CurrentGovernedObservationV1 {
                genesis_hash: &self.genesis,
                commitment: "finalized",
                context_slot: self.slot,
                program: required(0)?,
                programdata: required(1)?,
                gate: required(2)?,
            },
            business_accounts: &business,
        })
    }
}

pub(super) fn read_snapshot(
    config: &OnchainConfig,
    release: &CurrentGovernedWriteReleaseV1,
    business: Option<&CurrentFinalizedObservation>,
    minimum_slot: u64,
    deadline: Option<u64>,
    signed_locally: bool,
) -> Result<FinalizedSnapshot, CliError> {
    let client = rpc_client()?;
    let rpc_url = onchain::resolve_rpc_url(config)?;
    let genesis = rpc_result(&client, &rpc_url, "getGenesisHash", json!([]))?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| operation_error("Invalid network genesis."))?;
    if genesis != ameba_sdk::CURRENT_FINALIZED_OBSERVATION_DEVNET_GENESIS_HASH {
        return Err(operation_error(
            "The current operation connection is not Devnet.",
        ));
    }
    let mut addresses = vec![
        release.program_id(),
        release.programdata_address(),
        release.gate_address(),
    ];
    let mut business_indexes = Vec::new();
    if let Some(observation) = business {
        ameba_sdk::validate_current_finalized_observation(observation)
            .map_err(|_| operation_error("The prepared account observation is invalid."))?;
        for account in &observation.ordered_accounts {
            let address = canonical_pubkey(&account.address, "prepared account")?;
            let index = addresses
                .iter()
                .position(|candidate| *candidate == address)
                .unwrap_or_else(|| {
                    addresses.push(address);
                    addresses.len() - 1
                });
            business_indexes.push(index);
        }
    }
    if addresses.len() > MAX_REOBSERVED_ACCOUNTS {
        return Err(operation_error(
            "The operation exceeds one atomic finalized account snapshot.",
        ));
    }
    let minimum_slot = minimum_slot.max(release.minimum_finalized_slot());
    let result = rpc_result(
        &client,
        &rpc_url,
        "getMultipleAccounts",
        json!([
            addresses.iter().map(ToString::to_string).collect::<Vec<_>>(),
            {"commitment":"finalized", "encoding":"base64", "minContextSlot":minimum_slot}
        ]),
    )?;
    let snapshot = decode_snapshot(genesis, &addresses, business_indexes, minimum_slot, &result)?;
    if let Some(observation) = business {
        let block_time = rpc_result(&client, &rpc_url, "getBlockTime", json!([snapshot.slot]))?
            .as_i64()
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| operation_error("The finalized snapshot has no valid block time."))?;
        verify_observation_age(observation, block_time, deadline, signed_locally)?;
    }
    Ok(snapshot)
}

fn decode_snapshot(
    genesis: String,
    addresses: &[Pubkey],
    business_indexes: Vec<usize>,
    minimum_slot: u64,
    result: &Value,
) -> Result<FinalizedSnapshot, CliError> {
    let slot = result
        .pointer("/context/slot")
        .and_then(Value::as_u64)
        .filter(|slot| *slot >= minimum_slot)
        .ok_or_else(|| operation_error("The current finalized snapshot regressed."))?;
    let values = result
        .get("value")
        .and_then(Value::as_array)
        .filter(|values| values.len() == addresses.len())
        .ok_or_else(|| operation_error("The current finalized snapshot is incomplete."))?;
    let accounts = addresses
        .iter()
        .zip(values)
        .map(|(address, value)| {
            let account = if value.is_null() {
                None
            } else {
                let owner = value
                    .get("owner")
                    .and_then(Value::as_str)
                    .ok_or_else(|| operation_error("The finalized account has no owner."))?;
                let owner = canonical_pubkey(owner, "finalized owner")?;
                let executable = value
                    .get("executable")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| {
                        operation_error("The finalized account has no executable flag.")
                    })?;
                Some((owner, executable, rpc_account_data(value)?))
            };
            Ok(OwnedAccount {
                address: *address,
                value: account,
            })
        })
        .collect::<Result<Vec<_>, CliError>>()?;
    Ok(FinalizedSnapshot {
        genesis,
        slot,
        accounts,
        business_indexes,
    })
}

pub(super) fn observe_context(
    config: &OnchainConfig,
) -> Result<CurrentGovernedWriteContextV1, CliError> {
    let release = ameba_sdk::current_governed_write_release_v1()
        .map_err(|_| crate::current_release::write_unavailable_error())?;
    let identity = crate::chain_identity::verify_onchain_config_fresh(config)?;
    let context = read_snapshot(
        config,
        &release,
        None,
        release.minimum_finalized_slot(),
        None,
        false,
    )?
    .with_view(|view| {
        ameba_sdk::observe_current_governed_write_context_v1(
            &release,
            view.deployment,
            release.minimum_finalized_slot(),
        )
        .map_err(|_| {
            operation_error("Wallet changes are unavailable while governance is frozen or unverifiable. Nothing was prepared, signed, or sent. [CURRENT_PROGRAM_GOVERNANCE_UNAVAILABLE]")
        })
    })?;
    identity.require_initialized_business()?;
    Ok(context)
}

//! Finalized-state re-observation and local execution of SDK-validated current operations.
//!
//! This module owns no financial formula and derives no protocol account. It
//! compares every account in a Lean/SDK plan with one fresh finalized RPC read,
//! obtains a fresh finalized blockhash, signs only the already reconstructed
//! native instructions, and delegates broadcast/confirmation to Amoeba's
//! typed prepared-plan submit/status routes.

use crate::content_hash::sha256_hex;
use std::{
    io::Read,
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use ameba_sdk::CurrentFinalizedObservation;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use light_client::{
    indexer::photon_indexer::PhotonIndexer,
    interface::{AccountSpec, create_load_instructions},
    rpc::{LightClient, LightClientConfig, Rpc},
};
use photon_api::apis::configuration::Configuration as PhotonConfiguration;
use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::{Value, json};
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_message::VersionedMessage;
use solana_pubkey::Pubkey;
use solana_transaction::{Transaction, versioned::VersionedTransaction};

use crate::{
    backend::{BackendClient, CliError},
    onchain::{self, OnchainConfig},
    wallet_signer,
};

mod governed;

pub fn observe_current_write_context(
    config: &OnchainConfig,
) -> Result<ameba_sdk::CurrentGovernedWriteContextV1, CliError> {
    governed::observe_context(config)
}

const RPC_TIMEOUT: Duration = Duration::from_secs(20);
const PHOTON_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RPC_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_REOBSERVED_ACCOUNTS: usize = 100;
const MAX_PLAN_AGE_SECONDS: u64 = 120;
const MIN_DEADLINE_REMAINING_SECONDS: u64 = 8;
const MAX_TRANSACTION_BYTES: usize = 1_232;
const OPERATION_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(45);
const OPERATION_CONFIRMATION_POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentOperationReceipt {
    pub signature: String,
    pub operation_id: String,
    pub prepared_plan_digest: String,
    pub reobserved_slot: String,
    pub recent_blockhash: String,
    pub instruction_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FreshObservationContext {
    slot: u64,
    block_time: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FreshExecutionContext {
    observation: FreshObservationContext,
    blockhash: Hash,
    last_valid_block_height: u64,
}

#[derive(Clone, Copy, Debug)]
struct SignedTransactionBinding<'a> {
    signature: &'a str,
    transaction_sha256: &'a str,
    message_sha256: &'a str,
}

enum CurrentOperationSigning {
    Legacy(ameba_sdk::CurrentGovernedSigningV1),
    Versioned(ameba_sdk::CurrentGovernedVersionedSigningV1),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationStatus {
    Prepared,
    Submitted,
    Pending,
    Confirmed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfirmationStatus {
    Pending,
    Confirmed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OperationStatusObservation {
    operation: OperationStatus,
    submission: Option<OperationStatus>,
    confirmation: Option<ConfirmationStatus>,
}

impl OperationStatus {
    fn parse(value: &str) -> Result<Self, CliError> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "submitted" => Ok(Self::Submitted),
            "pending" => Ok(Self::Pending),
            "confirmed" => Ok(Self::Confirmed),
            "failed" => Ok(Self::Failed),
            _ => Err(operation_error(
                "Amoeba returned an unknown prepared-operation status.",
            )),
        }
    }
}

#[derive(Clone, Debug)]
struct NoPdaVariant;

impl light_account::Pack<AccountMeta> for NoPdaVariant {
    type Packed = u8;

    fn pack(
        &self,
        _remaining_accounts: &mut light_account::PackedAccounts,
    ) -> Result<Self::Packed, light_account::LightSdkTypesError> {
        Ok(0)
    }
}

/// Independently resolve the one cold Flat ATA permitted by the SDK plan and
/// rebuild its official payer-bound Light load. The result is compared byte
/// for byte by `ameba_sdk::parse_flat_transfer_operation_json_with_setup`.
pub fn rebuild_flat_transfer_setup(
    config: &OnchainConfig,
    plan: &ameba_sdk::FlatTransferOperationPlan,
) -> Result<Vec<Vec<Instruction>>, CliError> {
    let proof = match (&plan.source_proof_facts, &plan.destination_proof_facts) {
        (None, None) => return Ok(Vec::new()),
        (Some(proof), None) | (None, Some(proof)) => proof,
        (Some(_), Some(_)) => {
            return Err(operation_error(
                "Two cold Flat accounts require sequential fresh preparation. Nothing was signed or sent.",
            ));
        }
    };
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    let rpc_url = onchain::resolve_rpc_url(config)?;
    require_private_provider_witness_digest(
        &proof.provider_origin_sha256,
        "cold Flat provider witness",
    )?;
    let owner = canonical_pubkey(&proof.owner, "cold Flat owner")?;
    let payer = canonical_pubkey(&proof.payer, "cold Flat payer")?;
    let mint = canonical_pubkey(&proof.mint, "cold Flat mint")?;
    let ata = canonical_pubkey(&proof.ata, "cold Flat account")?;
    let sleeve = canonical_pubkey(&plan.semantic.sleeve, "writer sleeve")?;
    let expected_mint = ameba_sdk::derive_writer_flat_mint_pda(&ameba_sdk::ID, &sleeve).0;
    if payer.to_string() != plan.semantic.owner
        || mint != expected_mint
        || proof
            .amount_atoms
            .parse::<u64>()
            .ok()
            .map(|value| value.to_string())
            .as_deref()
            != Some(proof.amount_atoms.as_str())
    {
        return Err(operation_error(
            "The cold Flat proof does not match the requested payer, sleeve mint, or canonical amount.",
        ));
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| operation_error("Could not initialize the Light account resolver."))?;
    runtime.block_on(async move {
        let photon_http = photon_reqwest::Client::builder()
            .timeout(PHOTON_TIMEOUT)
            .redirect(photon_reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| operation_error("Could not open the Amoeba cold-state connection."))?;
        let mut photon_config = PhotonConfiguration::new(rpc_url.clone());
        photon_config.client = photon_http;
        let mut light_config = LightClientConfig::new(rpc_url, None);
        light_config.fetch_active_tree = false;
        let mut rpc = LightClient::new(light_config)
            .await
            .map_err(|_| operation_error("Could not initialize the authorized Light client."))?;
        rpc.indexer = Some(PhotonIndexer::new_with_config(photon_config));
        let interface = rpc
            .get_associated_token_account_interface(&owner, &mint, None)
            .await
            .map_err(|_| operation_error("Amoeba could not resolve the cold Flat account."))?
            .value
            .ok_or_else(|| {
                operation_error("The cold Flat account is absent from Amoeba's current state.")
            })?;
        let proof_amount = proof
            .amount_atoms
            .parse::<u64>()
            .map_err(|_| operation_error("The cold Flat amount is invalid."))?;
        if interface.key != ata
            || !interface.is_cold()
            || !interface.is_ata()
            || interface.owner() != owner
            || interface.mint() != mint
            || interface.amount() != proof_amount
            || interface.is_frozen()
        {
            return Err(operation_error(
                "Amoeba's cold Flat result does not match the prepared proof facts.",
            ));
        }
        let specs = [AccountSpec::<NoPdaVariant>::Ata(Box::new(interface))];
        let indexer = rpc
            .indexer
            .as_ref()
            .ok_or_else(|| operation_error("Amoeba cold-state indexing is unavailable."))?;
        let instructions = create_load_instructions(
            &specs,
            payer,
            ameba_sdk::constants::LIGHT_TOKEN_COMPRESSIBLE_CONFIG,
            indexer,
        )
        .await
        .map_err(|_| operation_error("The official Light load could not be rebuilt."))?;
        if instructions.is_empty() || instructions.len() > 8 {
            return Err(operation_error(
                "The official Light load has an invalid transaction shape.",
            ));
        }
        Ok(vec![instructions])
    })
}

/// Independently rebuild the exact Light-account setup committed by one
/// current writer-operation plan. Hot plans need no setup. Cold Begin/Basket
/// plans resolve the authenticated Amoeba cold account and rebuild the official
/// load, while cancellation rebuilds the canonical idempotent output ATA.
pub fn rebuild_writer_operation_setup(
    config: &OnchainConfig,
    plan: &ameba_sdk::WriterOperationPlan,
) -> Result<Vec<Vec<Instruction>>, CliError> {
    let Some(mode) = plan.setup_mode else {
        return Ok(Vec::new());
    };
    if let Some(facts) = plan.classic_output_setup_facts.as_ref() {
        if !matches!(
            mode,
            ameba_sdk::WriterSetupMode::ColdLoad | ameba_sdk::WriterSetupMode::ClassicOutputCreate
        ) {
            return Err(operation_error(
                "Classic output facts do not match the writer setup mode.",
            ));
        }
        let payer = canonical_pubkey(&facts.payer, "classic output payer")?;
        let owner = canonical_pubkey(&facts.owner, "classic output owner")?;
        let mint = canonical_pubkey(&facts.mint, "classic output mint")?;
        let ata = canonical_pubkey(&facts.ata, "classic output account")?;
        if plan.semantic.get("owner").and_then(Value::as_str) != Some(facts.payer.as_str()) {
            return Err(operation_error(
                "Classic output payer differs from the requested actor.",
            ));
        }
        let token_program = canonical_pubkey(
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            "classic SPL Token program",
        )?;
        let create =
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &payer,
                &owner,
                &mint,
                &token_program,
            );
        if create.accounts.get(1).map(|meta| meta.pubkey) != Some(ata) {
            return Err(operation_error(
                "The classic refund or withdrawal destination is not canonical.",
            ));
        }
        let mut batches = if mode == ameba_sdk::WriterSetupMode::ColdLoad {
            rebuild_writer_cold_setup(
                config,
                plan.cold_account_proof_facts
                    .as_ref()
                    .ok_or_else(|| operation_error("Cold writer output plan lacks proof facts."))?,
            )?
        } else {
            let mut data = vec![2];
            data.extend_from_slice(&600_000u32.to_le_bytes());
            vec![vec![Instruction {
                program_id: canonical_pubkey(
                    "ComputeBudget111111111111111111111111111111",
                    "Compute Budget program",
                )?,
                accounts: Vec::new(),
                data,
            }]]
        };
        batches
            .last_mut()
            .ok_or_else(|| operation_error("Writer output setup is empty."))?
            .push(create);
        return Ok(batches);
    }
    match mode {
        ameba_sdk::WriterSetupMode::WriterLiquidityCompute => {
            ameba_sdk::current_writer_liquidity_compute_setup().map_err(|error| {
                operation_error(format!(
                    "Writer liquidity compute setup is invalid: {error}"
                ))
            })
        }
        ameba_sdk::WriterSetupMode::ClassicOutputCreate => {
            Err(operation_error("Classic output setup facts are missing."))
        }
        ameba_sdk::WriterSetupMode::ReleaseCompute => Err(operation_error(
            "The deployed V3 Writer release does not permit release_compute setup.",
        )),
        ameba_sdk::WriterSetupMode::ColdLoad => {
            let proof = plan.cold_account_proof_facts.as_ref().ok_or_else(|| {
                operation_error("The cold writer plan is missing authenticated proof facts.")
            })?;
            rebuild_writer_cold_setup(config, proof)
        }
        ameba_sdk::WriterSetupMode::CanonicalOutputCreate => {
            let facts = plan.canonical_output_setup_facts.as_ref().ok_or_else(|| {
                operation_error("The writer cancellation plan is missing canonical output facts.")
            })?;
            let payer = canonical_pubkey(&facts.payer, "writer cancellation payer")?;
            let owner = canonical_pubkey(&facts.owner, "writer cancellation output owner")?;
            let mint = canonical_pubkey(&facts.mint, "writer cancellation output mint")?;
            let ata = canonical_pubkey(&facts.ata, "writer cancellation output account")?;
            if plan.semantic.get("owner").and_then(Value::as_str) != Some(facts.payer.as_str()) {
                return Err(operation_error(
                    "The writer cancellation setup payer does not match the attached actor.",
                ));
            }
            let create =
                light_token::instruction::CreateAssociatedTokenAccount::new(payer, owner, mint)
                    .idempotent()
                    .instruction()
                    .map_err(|_| {
                        operation_error(
                            "The official Light cancellation output could not be rebuilt.",
                        )
                    })?;
            if create.accounts.get(3).map(|meta| meta.pubkey) != Some(ata) {
                return Err(operation_error(
                    "The writer cancellation output is not the canonical Light account.",
                ));
            }
            let mut compute_data = vec![2];
            compute_data.extend_from_slice(&600_000u32.to_le_bytes());
            let compute = Instruction {
                program_id: canonical_pubkey(
                    "ComputeBudget111111111111111111111111111111",
                    "Compute Budget program",
                )?,
                accounts: Vec::new(),
                data: compute_data,
            };
            Ok(vec![vec![compute, create]])
        }
    }
}

fn rebuild_writer_cold_setup(
    config: &OnchainConfig,
    proof: &ameba_sdk::WriterColdAccountProofFacts,
) -> Result<Vec<Vec<Instruction>>, CliError> {
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    let rpc_url = onchain::resolve_rpc_url(config)?;
    require_private_provider_witness_digest(
        &proof.provider_origin_sha256,
        "cold writer provider witness",
    )?;
    let owner = canonical_pubkey(&proof.owner, "cold writer owner")?;
    let payer = canonical_pubkey(&proof.payer, "cold writer payer")?;
    let mint = canonical_pubkey(&proof.mint, "cold writer mint")?;
    let ata = canonical_pubkey(&proof.ata, "cold writer account")?;
    let amount = proof
        .amount_atoms
        .parse::<u64>()
        .map_err(|_| operation_error("The cold writer amount is invalid."))?;
    let minimum = proof
        .minimum_amount_atoms
        .parse::<u64>()
        .map_err(|_| operation_error("The cold writer minimum amount is invalid."))?;
    if amount.to_string() != proof.amount_atoms
        || minimum.to_string() != proof.minimum_amount_atoms
        || amount < minimum
        || !proof.includes_cold_balance
    {
        return Err(operation_error(
            "The cold writer proof has invalid balance or minimum facts.",
        ));
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| operation_error("Could not initialize the Light account resolver."))?;
    runtime.block_on(async move {
        let photon_http = photon_reqwest::Client::builder()
            .timeout(PHOTON_TIMEOUT)
            .redirect(photon_reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| operation_error("Could not open the Amoeba cold-state connection."))?;
        let mut photon_config = PhotonConfiguration::new(rpc_url.clone());
        photon_config.client = photon_http;
        let mut light_config = LightClientConfig::new(rpc_url, None);
        light_config.fetch_active_tree = false;
        let mut rpc = LightClient::new(light_config)
            .await
            .map_err(|_| operation_error("Could not initialize the authorized Light client."))?;
        rpc.indexer = Some(PhotonIndexer::new_with_config(photon_config));
        let interface = rpc
            .get_associated_token_account_interface(&owner, &mint, None)
            .await
            .map_err(|_| operation_error("Amoeba could not resolve the cold writer account."))?
            .value
            .ok_or_else(|| {
                operation_error("The cold writer account is absent from Amoeba's current state.")
            })?;
        if interface.key != ata
            || !interface.is_cold()
            || !interface.is_ata()
            || interface.owner() != owner
            || interface.mint() != mint
            || interface.amount() != amount
            || interface.is_frozen()
        {
            return Err(operation_error(
                "Amoeba's cold writer result does not match the prepared proof facts.",
            ));
        }
        let specs = [AccountSpec::<NoPdaVariant>::Ata(Box::new(interface))];
        let indexer = rpc
            .indexer
            .as_ref()
            .ok_or_else(|| operation_error("Amoeba cold-state indexing is unavailable."))?;
        let instructions = create_load_instructions(
            &specs,
            payer,
            ameba_sdk::constants::LIGHT_TOKEN_COMPRESSIBLE_CONFIG,
            indexer,
        )
        .await
        .map_err(|_| operation_error("The official Light writer load could not be rebuilt."))?;
        if instructions.is_empty() || instructions.len() > 8 {
            return Err(operation_error(
                "The official Light writer load has an invalid transaction shape.",
            ));
        }
        Ok(vec![instructions])
    })
}

fn with_final_admitted_signer<T>(
    config: &OnchainConfig,
    expected_signer: &Pubkey,
    continuation: impl FnOnce(&dyn solana_signer::Signer) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let signer = wallet_signer::load_signer(config)?;
    wallet_signer::require_admitted_signer(signer.as_ref(), expected_signer)?;
    continuation(signer.as_ref())
}

/// Re-observe the complete plan snapshot before the final signing-source load,
/// then repeat fresh identity, snapshot, deadline, and original-blockhash
/// validity checks immediately before relaying exactly the signed SDK plan.
pub fn sign_submit_validated_operation(
    config: &OnchainConfig,
    backend: &BackendClient,
    submit_path: &str,
    status_path: &str,
    expected_operation: &str,
    admitted: &ameba_sdk::CurrentGovernedOperationV1,
    deadline_ts: Option<u64>,
) -> Result<CurrentOperationReceipt, CliError> {
    crate::current_release::require_current_write_release()?;
    let [instructions] = admitted.execution_instruction_batches() else {
        return Err(operation_error(
            "The admitted operation requires unsupported sequential batches.",
        ));
    };
    let observation = admitted.observation();
    let operation_id = admitted.operation_id();
    let prepared_plan_digest = admitted.prepared_plan_digest();
    let expected_signer = admitted.payer();
    crate::mcp_actions::authorize_execution(
        operation_id,
        prepared_plan_digest,
        &expected_signer.to_string(),
    )?;
    let _submission_claim = crate::operation_journal::reserve(backend, admitted)?;
    if instructions.is_empty() || instructions.len() > 32 {
        return Err(operation_error(
            "The admitted operation has an invalid instruction count. Nothing was signed or sent.",
        ));
    }

    // This verifies the configured genesis, exact Spread ProgramData payload,
    // authority, collateral identity, and linked Light programs. It is fresh,
    // intentionally bypassing the short read cache.
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    let mut execution = if admitted.swap_operation().is_some() {
        let fresh = reobserve_finalized_state(
            &rpc_client()?,
            &onchain::resolve_rpc_url(config)?,
            observation,
            deadline_ts,
            None,
            false,
        )?;
        // A placeholder is used only for unsigned SDK preflight. The backend-owned
        // lifetime is acquired after account validation and signer resolution.
        FreshExecutionContext {
            observation: fresh,
            blockhash: Hash::default(),
            last_valid_block_height: 0,
        }
    } else {
        reobserve_and_get_blockhash(config, observation, deadline_ts)?
    };
    let release = ameba_sdk::current_governed_write_release_v1()
        .map_err(|_| crate::current_release::write_unavailable_error())?;
    let signing_snapshot = governed::read_snapshot(
        config,
        &release,
        Some(observation),
        execution.observation.slot,
        deadline_ts,
        false,
    )?;
    let prepare_signing = |blockhash| {
        signing_snapshot.with_view(|view| {
            let prepared = if admitted.requires_versioned_signing() {
                ameba_sdk::prepare_current_governed_versioned_signing_v1(
                    admitted, &release, view, 0, blockhash,
                )
                .map(CurrentOperationSigning::Versioned)
            } else {
                ameba_sdk::prepare_current_governed_signing_v1(
                    admitted, &release, view, 0, blockhash,
                )
                .map(CurrentOperationSigning::Legacy)
            };
            prepared.map_err(|_| {
                operation_error("The current operation changed before signing. Prepare it again.")
            })
        })
    };
    let mut signing = prepare_signing(execution.blockhash)?;

    // The signing-source load used to produce the signature is deliberately
    // after both deployment verification and full finalized re-observation.
    // Request preparation may already have resolved the same source's public
    // key so Lean can bind the actor; equality is checked again here.
    with_final_admitted_signer(config, &expected_signer, |signer| {
        if admitted.swap_operation().is_some() {
            let response = backend.post_json(crate::endpoints::dlmm_trade_lifetime(), &json!({
                "operationId":operation_id,"preparedPlanDigest":prepared_plan_digest,"owner":expected_signer.to_string()
            }))?;
            let data = operation_data(&response, "trade transaction lifetime")?;
            let lifetime = data
                .get("transactionLifetime")
                .ok_or_else(|| operation_error("Trade lifetime is missing."))?;
            if lifetime["schemaVersion"] != 1
                || lifetime["operationId"] != operation_id
                || lifetime["preparedPlanDigest"] != prepared_plan_digest
                || lifetime["commitment"] != "finalized"
            {
                return Err(operation_error(
                    "Trade lifetime does not match the reviewed operation.",
                ));
            }
            let slot = lifetime["contextSlot"]
                .as_u64()
                .filter(|s| *s >= execution.observation.slot)
                .ok_or_else(|| operation_error("Trade lifetime observation is stale."))?;
            let acquired = lifetime["acquiredAt"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .ok_or_else(|| operation_error("Trade lifetime has no valid observation time."))?;
            let age = chrono::Utc::now()
                .signed_duration_since(acquired)
                .num_seconds();
            if !(0..=30).contains(&age) {
                return Err(operation_error(
                    "Trade lifetime expired before signing. Prepare a fresh review.",
                ));
            }
            execution.blockhash = lifetime["blockhash"]
                .as_str()
                .and_then(|s| Hash::from_str(s).ok())
                .ok_or_else(|| operation_error("Trade lifetime blockhash is invalid."))?;
            execution.last_valid_block_height = lifetime["lastValidBlockHeight"]
                .as_u64()
                .filter(|h| *h > 0)
                .ok_or_else(|| operation_error("Trade lifetime height is invalid."))?;
            execution.observation.slot = slot;
            signing = prepare_signing(execution.blockhash)?;
        }
        let message = match &signing {
            CurrentOperationSigning::Legacy(signing) => {
                VersionedMessage::Legacy(signing.message().clone())
            }
            CurrentOperationSigning::Versioned(signing) => signing.message().clone(),
        };
        let unsigned = VersionedTransaction {
            signatures: vec![Default::default(); message.header().num_required_signatures as usize],
            message,
        };
        let prepared_bytes = bincode::serialize(&unsigned)
            .map_err(|_| operation_error("The prepared operation could not be encoded."))?;
        if prepared_bytes.len() > MAX_TRANSACTION_BYTES {
            return Err(operation_error(
                "The admitted operation exceeds Solana's transaction packet limit. Nothing was submitted.",
            ));
        }
        let prepared = BASE64_STANDARD.encode(prepared_bytes);
        let transaction = VersionedTransaction::try_new(unsigned.message, &[signer])
            .map_err(|_| operation_error("The attached wallet could not sign this operation."))?;
        let message_bytes =
            verify_final_signed_versioned_transaction(&transaction, expected_signer)?;
        let encoded_bytes = bincode::serialize(&transaction)
            .map_err(|_| operation_error("The signed operation could not be encoded."))?;
        if encoded_bytes.len() > MAX_TRANSACTION_BYTES {
            return Err(operation_error(
                "The admitted operation exceeds Solana's transaction packet limit. Nothing was submitted.",
            ));
        }
        let transaction_sha256 = sha256_hex(&encoded_bytes);
        let message_sha256 = sha256_hex(&message_bytes);
        let encoded = BASE64_STANDARD.encode(&encoded_bytes);
        let signature = transaction
            .signatures
            .first()
            .map(ToString::to_string)
            .ok_or_else(|| operation_error("The signed operation has no transaction signature."))?;
        let signed_binding = SignedTransactionBinding {
            signature: &signature,
            transaction_sha256: &transaction_sha256,
            message_sha256: &message_sha256,
        };
        let submission = json!({
            "operationId": operation_id,
            "preparedPlanDigest": prepared_plan_digest,
            "owner": expected_signer.to_string(),
            "batchIndex": 0,
            "serializedTransactionBase64": prepared,
            "signedTransactionBase64": encoded,
        });

        // Hardware-wallet approval can take long enough for deployment identity,
        // finalized accounts, the admitted deadline, or the signed blockhash to
        // change. Re-run every fail-closed read after signing, while retaining and
        // checking the exact lastValidBlockHeight of the blockhash in the message.
        crate::chain_identity::verify_onchain_config_fresh(config)?;
        let relay_observation = reobserve_before_relay(
            config,
            observation,
            deadline_ts,
            execution.observation,
            execution.last_valid_block_height,
        )?;
        governed::read_snapshot(
            config, &release, Some(observation), relay_observation.slot, deadline_ts, true,
        )?.with_view(|view| {
            let result = match (&signing, &transaction.message) {
                (CurrentOperationSigning::Legacy(signing), VersionedMessage::Legacy(message)) => {
                    let legacy = Transaction { signatures: transaction.signatures.clone(), message: message.clone() };
                    ameba_sdk::revalidate_current_governed_signed_transaction_v1(signing, &release, view, &legacy)
                }
                (CurrentOperationSigning::Versioned(signing), VersionedMessage::V0(_)) => {
                    ameba_sdk::revalidate_current_governed_signed_versioned_transaction_v1(signing, &release, view, &transaction)
                }
                _ => return Err(operation_error("Signed message version differs from the admitted transport.")),
            };
            result.map_err(|_| operation_error("The approved operation or current governance changed. The transaction was signed locally but not submitted."))
        })?;
        crate::operation_journal::before_relay(
            backend,
            admitted,
            expected_operation,
            &signature,
            &transaction_sha256,
            &message_sha256,
        )?;
        let submit_error = submission_result_error(
            backend.post_json(submit_path, &submission),
            operation_id,
            prepared_plan_digest,
            signed_binding,
        );
        let confirmation = wait_for_typed_operation_confirmation(
            backend,
            status_path,
            operation_id,
            prepared_plan_digest,
            expected_operation,
            signed_binding,
            submit_error,
        );
        if let Err(error) = confirmation {
            if let Ok(record) = crate::operation_journal::load(operation_id) {
                match recover_finalized_signature(backend, &record) {
                    Ok(Some(("confirmed", _))) => {}
                    Ok(Some(("failed_on_chain", _))) => {
                        crate::operation_journal::failed_on_chain(operation_id)?;
                        return Err(CliError::confirmed_failure(
                            "The original transaction finalized with an execution error. Inspect its operation receipt before preparing a new action.",
                            operation_id,
                            &signature,
                        ));
                    }
                    _ => {
                        return Err(CliError::uncertain(
                            format!(
                                "{error} Operation {operation_id}; signature {signature}. Use petri operations resume {operation_id}; do not resubmit."
                            ),
                            operation_id,
                            &signature,
                        ));
                    }
                }
            } else {
                return Err(CliError::uncertain(
                    format!(
                        "{error} Operation {operation_id}; signature {signature}. Use petri operations resume {operation_id}; do not resubmit."
                    ),
                    operation_id,
                    &signature,
                ));
            }
        }
        crate::operation_journal::confirmed(operation_id).map_err(|error|CliError::uncertain(
            format!("The transaction was confirmed, but its local receipt could not be saved: {error}. Recover this operation; do not resubmit."),operation_id,&signature))?;
        Ok(CurrentOperationReceipt {
            signature,
            operation_id: operation_id.to_owned(),
            prepared_plan_digest: prepared_plan_digest.to_owned(),
            reobserved_slot: relay_observation.slot.to_string(),
            recent_blockhash: execution.blockhash.to_string(),
            instruction_count: instructions.len(),
        })
    })
}

pub fn validate_recovered_operation(
    record: &crate::operation_journal::OperationRecord,
    response: &Value,
) -> Result<&'static str, CliError> {
    if let (Some(signature), Some(transaction), Some(message)) = (
        &record.signature,
        &record.transaction_sha256,
        &record.message_sha256,
    ) {
        let status = validate_operation_status_response(
            response,
            &record.operation_id,
            &record.prepared_plan_digest,
            &record.operation,
            SignedTransactionBinding {
                signature,
                transaction_sha256: transaction,
                message_sha256: message,
            },
        )?;
        return Ok(match status.operation {
            OperationStatus::Confirmed => "confirmed",
            OperationStatus::Failed => "rejected",
            OperationStatus::Pending => "pending",
            OperationStatus::Submitted => "submitted",
            OperationStatus::Prepared => "transport_uncertain",
        });
    }
    let data = operation_data(response, "operation recovery")?;
    validate_operation_binding(data, &record.operation_id, &record.prepared_plan_digest)?;
    if data["status"] == "prepared"
        && data["operation"] == record.operation
        && data["nextBatchIndex"] == 0
        && data["batchCount"] == 1
        && data["submissions"] == json!([])
    {
        return Ok("prepared");
    }
    // Without a local signature binding, a server status must not certify a payment.
    Ok("needs_fresh_preparation")
}

pub fn read_trade_market(
    config: &OnchainConfig,
    series: &str,
) -> Result<(Pubkey, ameba_sdk::state::Market, u64), CliError> {
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    if !series.is_ascii() || series.len() > 32 {
        return Err(operation_error("Series label is invalid."));
    }
    let mut series_id = [0u8; 32];
    series_id[..series.len()].copy_from_slice(series.as_bytes());
    let address = ameba_sdk::derive_market_pda(&ameba_sdk::ID, &series_id).0;
    let client = rpc_client()?;
    let url = onchain::resolve_rpc_url(config)?;
    let result = rpc_result(
        &client,
        &url,
        "getAccountInfo",
        json!([address.to_string(), {"encoding":"base64","commitment":"finalized"}]),
    )?;
    let slot = result
        .pointer("/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| operation_error("Market observation has no finalized slot."))?;
    let value = &result["value"];
    let owner = canonical_pubkey(value["owner"].as_str().unwrap_or_default(), "market owner")?;
    let bytes = rpc_account_data(value)?;
    let market = ameba_sdk::decode_current_market(
        ameba_sdk::CurrentAccountData {
            address,
            owner,
            executable: value["executable"]
                .as_bool()
                .ok_or_else(|| operation_error("Market executable flag is missing."))?,
            data: &bytes,
        },
        &ameba_sdk::ID,
    )
    .map_err(|_| operation_error("The exact series does not match the current market contract."))?;
    if market.market_id != series_id {
        return Err(operation_error("Market and selected series differ."));
    }
    let time = rpc_result(&client, &url, "getBlockTime", json!([slot]))?
        .as_u64()
        .ok_or_else(|| operation_error("Market observation time is unavailable."))?;
    Ok((address, market, time))
}

/// Read-only recovery never loads a signer and never creates a new transaction.
pub fn recover_finalized_signature(
    backend: &BackendClient,
    record: &crate::operation_journal::OperationRecord,
) -> Result<Option<(&'static str, Value)>, CliError> {
    record.require_scope(backend, &record.owner)?;
    let signature = record
        .signature
        .as_deref()
        .ok_or_else(|| operation_error("No original signature recorded."))?;
    let owner = canonical_pubkey(&record.owner, "operation owner")?;
    let config = OnchainConfig {
        network: "devnet".into(),
        backend_url: backend.base_url().into(),
        commitment: Some("finalized".into()),
        keypair_path: None,
        allow_insecure_keypair: false,
    };
    crate::chain_identity::verify_onchain_config_fresh(&config)?;
    let client = rpc_client()?;
    let url = onchain::resolve_rpc_url(&config)?;
    let result = rpc_result(
        &client,
        &url,
        "getTransaction",
        json!([signature,
        {"encoding":"base64","commitment":"finalized","maxSupportedTransactionVersion":0}]),
    )?;
    if result.is_null() {
        return Ok(None);
    }
    let encoded = result["transaction"]
        .as_array()
        .filter(|a| a.len() == 2 && a[1] == "base64")
        .and_then(|a| a[0].as_str())
        .filter(|s| s.len() <= MAX_TRANSACTION_BYTES * 2)
        .ok_or_else(|| operation_error("Finalized receipt has no bounded transaction packet."))?;
    let bytes = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| operation_error("Finalized packet is malformed."))?;
    if bytes.len() > MAX_TRANSACTION_BYTES
        || record.transaction_sha256.as_deref() != Some(sha256_hex(&bytes).as_str())
    {
        return Err(operation_error(
            "Finalized transaction differs from the original signed packet.",
        ));
    }
    let tx: VersionedTransaction = bincode::deserialize(&bytes)
        .map_err(|_| operation_error("Finalized transaction cannot be decoded."))?;
    let message = verify_final_signed_versioned_transaction(&tx, owner)?;
    if tx.signatures.first().map(ToString::to_string).as_deref() != Some(signature)
        || record.message_sha256.as_deref() != Some(sha256_hex(&message).as_str())
    {
        return Err(operation_error(
            "Finalized signature or message differs from the original operation.",
        ));
    }
    let slot = result["slot"]
        .as_u64()
        .filter(|s| *s > 0)
        .ok_or_else(|| operation_error("Finalized receipt has no slot."))?;
    let error = result
        .pointer("/meta/err")
        .ok_or_else(|| operation_error("Finalized receipt has no execution result."))?;
    let state = if error.is_null() {
        "confirmed"
    } else {
        "failed_on_chain"
    };
    Ok(Some((
        state,
        json!({"source":"finalized_chain","signature":signature,"slot":slot.to_string(),
        "commitment":"finalized","state":state,"error":error,"retryAuthorized":false}),
    )))
}

fn verify_final_signed_versioned_transaction(
    transaction: &VersionedTransaction,
    expected_signer: Pubkey,
) -> Result<Vec<u8>, CliError> {
    if transaction.message.header().num_required_signatures != 1
        || transaction.signatures.len() != 1
        || transaction.message.static_account_keys().first() != Some(&expected_signer)
    {
        return Err(operation_error(
            "The final operation does not have the exact admitted signer set. Nothing was submitted.",
        ));
    }
    transaction
        .sanitize()
        .map_err(|_| operation_error("The signed transaction shape is invalid."))?;
    transaction.verify_and_hash_message().map_err(|_| {
        operation_error("The signed operation does not verify against its exact message.")
    })?;
    Ok(transaction.message.serialize())
}

fn operation_data<'a>(response: &'a Value, label: &str) -> Result<&'a Value, CliError> {
    if !has_exact_keys(response, &["ok", "data"]) || response.get("ok") != Some(&Value::Bool(true))
    {
        return Err(operation_error(format!(
            "Amoeba returned an invalid {label} envelope."
        )));
    }
    let data = response
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| operation_error(format!("Amoeba omitted {label} data.")))?;
    crate::chain_identity::validate_current_protocol_data(data).map_err(|_| {
        operation_error(format!(
            "Amoeba returned {label} for a different protocol release."
        ))
    })?;
    Ok(data)
}

fn has_exact_keys(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
    })
}

fn is_canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn is_canonical_u64(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|parsed| parsed.to_string() == value)
}

fn required_text<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a str, CliError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| operation_error(format!("Amoeba omitted {label}.")))
}

fn validate_operation_binding(
    data: &Value,
    operation_id: &str,
    prepared_plan_digest: &str,
) -> Result<(), CliError> {
    if required_text(data, "operationId", "operation id")? != operation_id
        || required_text(data, "preparedPlanDigest", "prepared-plan digest")?
            != prepared_plan_digest
    {
        return Err(operation_error(
            "Amoeba returned status for a different prepared operation.",
        ));
    }
    Ok(())
}

fn validate_submission_response(
    response: &Value,
    operation_id: &str,
    prepared_plan_digest: &str,
    signed: SignedTransactionBinding<'_>,
) -> Result<(), CliError> {
    let data = operation_data(response, "operation submission")?;
    if !has_exact_keys(
        data,
        &[
            "protocol",
            "operationId",
            "preparedPlanDigest",
            "batchIndex",
            "batchCount",
            "submission",
            "confirmation",
        ],
    ) {
        return Err(operation_error(
            "Amoeba returned an invalid operation-submission shape.",
        ));
    }
    validate_operation_binding(data, operation_id, prepared_plan_digest)?;
    if data.get("batchIndex").and_then(Value::as_u64) != Some(0)
        || data.get("batchCount").and_then(Value::as_u64) != Some(1)
    {
        return Err(operation_error(
            "Amoeba returned a submission for a different operation batch.",
        ));
    }
    let submission = data.get("submission").unwrap_or(&Value::Null);
    if !has_exact_keys(submission, &["signature"])
        || required_text(submission, "signature", "submission signature")? != signed.signature
    {
        return Err(operation_error(
            "Amoeba returned a different transaction signature.",
        ));
    }
    validate_current_confirmation(
        data.get("confirmation").unwrap_or(&Value::Null),
        false,
        signed,
    )?;
    Ok(())
}

fn submission_result_error(
    result: Result<Value, CliError>,
    operation_id: &str,
    prepared_plan_digest: &str,
    signed: SignedTransactionBinding<'_>,
) -> Option<CliError> {
    match result {
        Ok(response) => {
            validate_submission_response(&response, operation_id, prepared_plan_digest, signed)
                .err()
        }
        Err(error) => Some(error),
    }
}

fn validate_current_confirmation(
    value: &Value,
    require_confirmed: bool,
    signed: SignedTransactionBinding<'_>,
) -> Result<ConfirmationStatus, CliError> {
    let status = required_text(value, "status", "transaction confirmation status")?;
    match status {
        "pending" | "failed" if !require_confirmed => {
            if !has_exact_keys(value, &["status", "reason"])
                || !matches!(
                    required_text(value, "reason", "transaction confirmation reason")?,
                    "signature_not_found"
                        | "transaction_failed"
                        | "not_confirmed"
                        | "transaction_not_available"
                )
            {
                return Err(operation_error(
                    "Amoeba returned an invalid pending transaction confirmation.",
                ));
            }
            Ok(if status == "failed" {
                ConfirmationStatus::Failed
            } else {
                ConfirmationStatus::Pending
            })
        }
        "confirmed" => {
            if !has_exact_keys(
                value,
                &[
                    "status",
                    "confirmationStatus",
                    "confirmedSlot",
                    "confirmedTransactionSha256",
                    "messageSha256",
                    "blockTime",
                ],
            ) || !matches!(
                required_text(value, "confirmationStatus", "confirmation commitment")?,
                "confirmed" | "finalized"
            ) || !value
                .get("confirmedSlot")
                .and_then(Value::as_str)
                .is_some_and(is_canonical_u64)
                || !value
                    .get("confirmedTransactionSha256")
                    .and_then(Value::as_str)
                    .is_some_and(|digest| {
                        is_canonical_sha256(digest) && digest == signed.transaction_sha256
                    })
                || !value
                    .get("messageSha256")
                    .and_then(Value::as_str)
                    .is_some_and(|digest| {
                        is_canonical_sha256(digest) && digest == signed.message_sha256
                    })
                || !value.get("blockTime").is_some_and(|block_time| {
                    block_time.is_null() || block_time.as_str().is_some_and(is_canonical_u64)
                })
            {
                return Err(operation_error(
                    "Amoeba returned an invalid confirmed transaction proof.",
                ));
            }
            Ok(ConfirmationStatus::Confirmed)
        }
        _ => Err(operation_error(
            "Amoeba returned an unknown transaction confirmation status.",
        )),
    }
}

fn validate_operation_status_response(
    response: &Value,
    operation_id: &str,
    prepared_plan_digest: &str,
    expected_operation: &str,
    signed: SignedTransactionBinding<'_>,
) -> Result<OperationStatusObservation, CliError> {
    let data = operation_data(response, "operation status")?;
    if !has_exact_keys(
        data,
        &[
            "protocol",
            "operationId",
            "preparedPlanDigest",
            "operation",
            "status",
            "nextBatchIndex",
            "batchCount",
            "submissions",
            "lastError",
        ],
    ) {
        return Err(operation_error(
            "Amoeba returned an invalid operation-status shape.",
        ));
    }
    validate_operation_binding(data, operation_id, prepared_plan_digest)?;
    if required_text(data, "operation", "operation kind")? != expected_operation
        || data.get("batchCount").and_then(Value::as_u64) != Some(1)
        || data
            .get("nextBatchIndex")
            .and_then(Value::as_u64)
            .is_none_or(|index| index > 1)
    {
        return Err(operation_error(
            "Amoeba returned status for a different operation or batch plan.",
        ));
    }
    let status = OperationStatus::parse(required_text(data, "status", "operation status")?)?;
    let submissions = data
        .get("submissions")
        .and_then(Value::as_array)
        .ok_or_else(|| operation_error("Amoeba omitted operation submissions."))?;
    if submissions.len() > 1 {
        return Err(operation_error(
            "Amoeba returned too many operation submissions.",
        ));
    }
    let mut submission_status = None;
    let mut confirmation_status = None;
    for submission in submissions {
        if !has_exact_keys(
            submission,
            &[
                "batchIndex",
                "signature",
                "transactionSha256",
                "messageSha256",
                "status",
                "confirmation",
            ],
        ) || submission.get("batchIndex").and_then(Value::as_u64) != Some(0)
            || required_text(submission, "signature", "status signature")? != signed.signature
            || !submission
                .get("transactionSha256")
                .and_then(Value::as_str)
                .is_some_and(|digest| {
                    is_canonical_sha256(digest) && digest == signed.transaction_sha256
                })
            || !submission
                .get("messageSha256")
                .and_then(Value::as_str)
                .is_some_and(|digest| {
                    is_canonical_sha256(digest) && digest == signed.message_sha256
                })
        {
            return Err(operation_error(
                "Amoeba returned an invalid operation batch status.",
            ));
        }
        let parsed_submission_status =
            OperationStatus::parse(required_text(submission, "status", "batch status")?)?;
        if parsed_submission_status == OperationStatus::Prepared {
            return Err(operation_error(
                "Amoeba returned an invalid operation batch status.",
            ));
        }
        submission_status = Some(parsed_submission_status);
        if let Some(confirmation) = submission
            .get("confirmation")
            .filter(|value| !value.is_null())
        {
            confirmation_status = Some(validate_current_confirmation(confirmation, false, signed)?);
        }
        let confirmation_matches_submission = match parsed_submission_status {
            OperationStatus::Submitted => matches!(
                confirmation_status,
                None | Some(ConfirmationStatus::Pending)
            ),
            OperationStatus::Pending => confirmation_status == Some(ConfirmationStatus::Pending),
            OperationStatus::Confirmed => {
                confirmation_status == Some(ConfirmationStatus::Confirmed)
            }
            OperationStatus::Failed => {
                matches!(confirmation_status, None | Some(ConfirmationStatus::Failed))
            }
            OperationStatus::Prepared => false,
        };
        if !confirmation_matches_submission {
            return Err(operation_error(
                "Amoeba returned contradictory submission and confirmation states.",
            ));
        }
    }
    let last_error = data.get("lastError").unwrap_or(&Value::Null);
    if !last_error.is_null()
        && last_error
            .as_str()
            .is_none_or(|message| message.trim().is_empty())
    {
        return Err(operation_error(
            "Amoeba returned an invalid operation failure reason.",
        ));
    }
    match status {
        OperationStatus::Prepared
            if data.get("nextBatchIndex").and_then(Value::as_u64) != Some(0)
                || !submissions.is_empty()
                || !last_error.is_null() =>
        {
            Err(operation_error(
                "Amoeba returned a contradictory prepared operation status.",
            ))
        }
        OperationStatus::Confirmed => {
            let submission = submissions
                .first()
                .ok_or_else(|| operation_error("Amoeba omitted the confirmed transaction."))?;
            if data.get("nextBatchIndex").and_then(Value::as_u64) != Some(1)
                || submission.get("status").and_then(Value::as_str) != Some("confirmed")
                || !last_error.is_null()
            {
                return Err(operation_error(
                    "Amoeba returned a contradictory confirmed operation status.",
                ));
            }
            let confirmation = validate_current_confirmation(
                submission.get("confirmation").unwrap_or(&Value::Null),
                true,
                signed,
            )?;
            Ok(OperationStatusObservation {
                operation: status,
                submission: submission_status,
                confirmation: Some(confirmation),
            })
        }
        OperationStatus::Submitted
            if submission_status != Some(OperationStatus::Submitted) || !last_error.is_null() =>
        {
            Err(operation_error(
                "Amoeba returned a contradictory submitted operation status.",
            ))
        }
        OperationStatus::Pending
            if !matches!(
                submission_status,
                Some(OperationStatus::Submitted | OperationStatus::Pending)
            ) || !last_error.is_null() =>
        {
            Err(operation_error(
                "Amoeba returned a contradictory pending operation status.",
            ))
        }
        OperationStatus::Failed
            if last_error.is_null()
                || submission_status.is_some_and(|nested| nested != OperationStatus::Failed) =>
        {
            Err(operation_error(
                "Amoeba returned a contradictory failed operation status.",
            ))
        }
        _ => Ok(OperationStatusObservation {
            operation: status,
            submission: submission_status,
            confirmation: confirmation_status,
        }),
    }
}

fn validate_operation_status_transition(
    previous: Option<OperationStatusObservation>,
    current: OperationStatusObservation,
) -> Result<(), CliError> {
    let monotone_operation = match previous.map(|observation| observation.operation) {
        None => true,
        Some(OperationStatus::Prepared) => true,
        Some(OperationStatus::Submitted) => !matches!(current.operation, OperationStatus::Prepared),
        Some(OperationStatus::Pending) => matches!(
            current.operation,
            OperationStatus::Pending | OperationStatus::Confirmed | OperationStatus::Failed
        ),
        Some(OperationStatus::Confirmed) => current.operation == OperationStatus::Confirmed,
        Some(OperationStatus::Failed) => current.operation == OperationStatus::Failed,
    };
    let monotone_submission = match (
        previous.and_then(|value| value.submission),
        current.submission,
    ) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(OperationStatus::Submitted), Some(nested)) => {
            !matches!(nested, OperationStatus::Prepared)
        }
        (Some(OperationStatus::Pending), Some(nested)) => matches!(
            nested,
            OperationStatus::Pending | OperationStatus::Confirmed | OperationStatus::Failed
        ),
        (Some(OperationStatus::Confirmed), Some(nested)) => nested == OperationStatus::Confirmed,
        (Some(OperationStatus::Failed), Some(nested)) => nested == OperationStatus::Failed,
        (Some(OperationStatus::Prepared), _) => false,
    };
    let monotone_confirmation = match (
        previous.and_then(|value| value.confirmation),
        current.confirmation,
    ) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(ConfirmationStatus::Pending), Some(_)) => true,
        (Some(ConfirmationStatus::Confirmed), Some(nested)) => {
            nested == ConfirmationStatus::Confirmed
        }
        (Some(ConfirmationStatus::Failed), Some(nested)) => nested == ConfirmationStatus::Failed,
    };
    if !monotone_operation || !monotone_submission || !monotone_confirmation {
        return Err(operation_error(
            "Amoeba returned a non-monotone operation lifecycle. Submission state is ambiguous; query the prepared operation before attempting another submission.",
        ));
    }
    Ok(())
}

fn wait_for_typed_operation_confirmation(
    backend: &BackendClient,
    status_path: &str,
    operation_id: &str,
    prepared_plan_digest: &str,
    expected_operation: &str,
    signed: SignedTransactionBinding<'_>,
    submit_error: Option<CliError>,
) -> Result<(), CliError> {
    let started = Instant::now();
    let mut previous_status = None;
    loop {
        match backend.get(status_path) {
            Ok(response) => {
                let observation = validate_operation_status_response(
                    &response,
                    operation_id,
                    prepared_plan_digest,
                    expected_operation,
                    signed,
                )?;
                validate_operation_status_transition(previous_status, observation)?;
                previous_status = Some(observation);
                match observation.operation {
                    OperationStatus::Confirmed => return Ok(()),
                    OperationStatus::Failed => {
                        let reason = response["data"]
                            .get("lastError")
                            .and_then(Value::as_str)
                            .unwrap_or("transaction_failed");
                        return Err(operation_error(format!(
                            "Amoeba reports that the prepared operation failed: {reason}."
                        )));
                    }
                    OperationStatus::Prepared
                    | OperationStatus::Submitted
                    | OperationStatus::Pending => {}
                }
            }
            Err(error) if started.elapsed() >= OPERATION_CONFIRMATION_TIMEOUT => {
                return Err(operation_error(format!(
                    "The Amoeba status route could not resolve the submitted operation: {error}"
                )));
            }
            Err(_) => {}
        }
        if started.elapsed() >= OPERATION_CONFIRMATION_TIMEOUT {
            let prefix = submit_error
                .as_ref()
                .map(|error| format!("The one-time Amoeba submit returned: {error}. "))
                .unwrap_or_default();
            return Err(operation_error(format!(
                "{prefix}Confirmation is still unresolved; query the operation status before attempting another submission."
            )));
        }
        thread::sleep(OPERATION_CONFIRMATION_POLL);
    }
}

fn reobserve_and_get_blockhash(
    config: &OnchainConfig,
    observation: &CurrentFinalizedObservation,
    deadline_ts: Option<u64>,
) -> Result<FreshExecutionContext, CliError> {
    let rpc_url = onchain::resolve_rpc_url(config)?;
    let client = rpc_client()?;
    let fresh_observation =
        reobserve_finalized_state(&client, &rpc_url, observation, deadline_ts, None, false)?;
    let latest = rpc_result(
        &client,
        &rpc_url,
        "getLatestBlockhash",
        json!([{"commitment": "finalized", "minContextSlot": fresh_observation.slot}]),
    )?;
    let blockhash_slot = latest
        .pointer("/context/slot")
        .and_then(Value::as_u64)
        .filter(|context_slot| *context_slot >= fresh_observation.slot)
        .ok_or_else(|| operation_error("The finalized blockhash context is stale."))?;
    let blockhash = latest
        .pointer("/value/blockhash")
        .and_then(Value::as_str)
        .and_then(|value| Hash::from_str(value).ok())
        .ok_or_else(|| operation_error("The selected connection returned an invalid blockhash."))?;
    let last_valid_block_height = latest
        .pointer("/value/lastValidBlockHeight")
        .and_then(Value::as_u64)
        .ok_or_else(|| operation_error("The selected connection omitted blockhash validity."))?;
    let current_block_height = rpc_result(
        &client,
        &rpc_url,
        "getBlockHeight",
        json!([{"commitment": "finalized", "minContextSlot": blockhash_slot}]),
    )?
    .as_u64()
    .ok_or_else(|| operation_error("The selected connection returned an invalid block height."))?;
    if !blockhash_is_valid_at_height(current_block_height, last_valid_block_height) {
        return Err(operation_error(
            "The finalized blockhash expired before signing. Nothing was signed or sent.",
        ));
    }
    Ok(FreshExecutionContext {
        observation: fresh_observation,
        blockhash,
        last_valid_block_height,
    })
}

fn reobserve_before_relay(
    config: &OnchainConfig,
    observation: &CurrentFinalizedObservation,
    deadline_ts: Option<u64>,
    minimum_observation: FreshObservationContext,
    original_last_valid_block_height: u64,
) -> Result<FreshObservationContext, CliError> {
    let rpc_url = onchain::resolve_rpc_url(config)?;
    let client = rpc_client()?;
    let fresh_observation = reobserve_finalized_state(
        &client,
        &rpc_url,
        observation,
        deadline_ts,
        Some(minimum_observation),
        true,
    )?;
    let current_block_height = rpc_result(
        &client,
        &rpc_url,
        "getBlockHeight",
        json!([{"commitment": "finalized", "minContextSlot": fresh_observation.slot}]),
    )?
    .as_u64()
    .ok_or_else(|| operation_error("The selected connection returned an invalid block height."))?;
    if !blockhash_is_valid_at_height(current_block_height, original_last_valid_block_height) {
        return Err(operation_error(
            "The signed transaction's blockhash expired before relay. Nothing was submitted.",
        ));
    }
    Ok(fresh_observation)
}

fn reobserve_finalized_state(
    client: &Client,
    rpc_url: &str,
    observation: &CurrentFinalizedObservation,
    deadline_ts: Option<u64>,
    minimum_observation: Option<FreshObservationContext>,
    signed_locally: bool,
) -> Result<FreshObservationContext, CliError> {
    ameba_sdk::validate_current_finalized_observation(observation).map_err(|_| {
        operation_error(format!(
            "The admitted finalized observation is invalid. {}",
            local_signature_outcome(signed_locally)
        ))
    })?;
    if observation.ordered_accounts.len() > MAX_REOBSERVED_ACCOUNTS {
        return Err(operation_error(
            "The admitted operation requires too many accounts for one atomic finalized re-observation.",
        ));
    }
    let genesis = rpc_result(client, rpc_url, "getGenesisHash", json!([]))?;
    if genesis.as_str() != Some(ameba_sdk::CURRENT_FINALIZED_OBSERVATION_DEVNET_GENESIS_HASH) {
        return Err(operation_error(format!(
            "The selected network does not match the admitted operation. {}",
            local_signature_outcome(signed_locally)
        )));
    }

    let slot = rpc_result(
        client,
        rpc_url,
        "getSlot",
        json!([{"commitment": "finalized"}]),
    )?
    .as_u64()
    .ok_or_else(|| {
        operation_error("The selected connection returned an invalid finalized slot.")
    })?;
    if minimum_observation.is_some_and(|minimum| slot < minimum.slot) {
        return Err(operation_error(
            "The finalized slot moved backwards after signing. Nothing was submitted.",
        ));
    }
    let addresses = observation
        .ordered_accounts
        .iter()
        .map(|account| account.address.as_str())
        .collect::<Vec<_>>();
    let accounts_result = rpc_result(
        client,
        rpc_url,
        "getMultipleAccounts",
        json!([addresses, {
            "commitment": "finalized",
            "encoding": "base64",
            "minContextSlot": slot,
        }]),
    )?;
    let accounts_slot = accounts_result
        .pointer("/context/slot")
        .and_then(Value::as_u64)
        .filter(|context_slot| *context_slot >= slot)
        .ok_or_else(|| operation_error("The finalized account snapshot is stale."))?;
    let values = accounts_result
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| operation_error("The finalized account snapshot is malformed."))?;
    verify_account_values(observation, values, signed_locally)?;
    let block_time = rpc_result(client, rpc_url, "getBlockTime", json!([accounts_slot]))?
        .as_i64()
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| {
            operation_error("The selected connection returned no finalized block time.")
        })?;
    if minimum_observation.is_some_and(|minimum| block_time < minimum.block_time) {
        return Err(operation_error(
            "Finalized chain time moved backwards after signing. Nothing was submitted.",
        ));
    }
    verify_observation_age(observation, block_time, deadline_ts, signed_locally)?;
    Ok(FreshObservationContext {
        slot: accounts_slot,
        block_time,
    })
}

fn blockhash_is_valid_at_height(current_block_height: u64, last_valid_block_height: u64) -> bool {
    current_block_height <= last_valid_block_height
}

fn local_signature_outcome(signed_locally: bool) -> &'static str {
    if signed_locally {
        "The transaction was signed locally but not submitted."
    } else {
        "Nothing was signed or sent."
    }
}

fn verify_observation_age(
    observation: &CurrentFinalizedObservation,
    current_block_time: u64,
    deadline_ts: Option<u64>,
    signed_locally: bool,
) -> Result<(), CliError> {
    let observed_block_time = observation
        .observed_block_time_unix_seconds
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| operation_error("The admitted operation has no finalized block time."))?;
    if current_block_time < observed_block_time
        || current_block_time - observed_block_time > MAX_PLAN_AGE_SECONDS
    {
        return Err(operation_error(format!(
            "The admitted operation is stale. Prepare it again. {}",
            local_signature_outcome(signed_locally)
        )));
    }
    if let Some(deadline) = deadline_ts
        && current_block_time
            .checked_add(MIN_DEADLINE_REMAINING_SECONDS)
            .is_none_or(|minimum| minimum >= deadline)
    {
        return Err(operation_error(format!(
            "The trade deadline is too close or has expired. Prepare it again. {}",
            local_signature_outcome(signed_locally)
        )));
    }
    Ok(())
}

fn verify_account_values(
    observation: &CurrentFinalizedObservation,
    values: &[Value],
    signed_locally: bool,
) -> Result<(), CliError> {
    if values.len() != observation.ordered_accounts.len() {
        return Err(operation_error(
            "The finalized account snapshot has the wrong number of accounts.",
        ));
    }
    for (expected, actual) in observation.ordered_accounts.iter().zip(values) {
        match (
            &expected.owner,
            &expected.executable,
            &expected.data_length,
            &expected.data_sha256,
        ) {
            (None, None, None, None) if actual.is_null() => continue,
            (Some(owner), Some(executable), Some(length), Some(digest)) => {
                let data = rpc_account_data(actual)?;
                let actual_digest = sha256_hex(&data);
                if actual.get("owner").and_then(Value::as_str) != Some(owner.as_str())
                    || actual.get("executable").and_then(Value::as_bool) != Some(*executable)
                    || length.parse::<usize>().ok() != Some(data.len())
                    || actual_digest != *digest
                {
                    return Err(operation_error(format!(
                        "Finalized account state changed after preparation. Prepare the operation again. {}",
                        local_signature_outcome(signed_locally)
                    )));
                }
            }
            _ => {
                return Err(operation_error(format!(
                    "The admitted account snapshot is incomplete. {}",
                    local_signature_outcome(signed_locally)
                )));
            }
        }
    }
    Ok(())
}

fn rpc_account_data(value: &Value) -> Result<Vec<u8>, CliError> {
    let encoded = value
        .get("data")
        .and_then(Value::as_array)
        .filter(|parts| parts.len() == 2)
        .and_then(|parts| {
            (parts.get(1).and_then(Value::as_str) == Some("base64"))
                .then(|| parts.first().and_then(Value::as_str))
                .flatten()
        })
        .ok_or_else(|| operation_error("The finalized account data is not canonical base64."))?;
    let data = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| operation_error("The finalized account data is invalid base64."))?;
    if BASE64_STANDARD.encode(&data) != encoded {
        return Err(operation_error(
            "The finalized account data is not canonical base64.",
        ));
    }
    Ok(data)
}

fn require_private_provider_witness_digest(raw: &str, label: &str) -> Result<(), CliError> {
    if raw.len() != 64
        || !raw
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(operation_error(&format!(
            "The {label} is not a canonical SHA-256 digest. Nothing was signed or sent."
        )));
    }
    Ok(())
}

fn canonical_pubkey(raw: &str, label: &str) -> Result<Pubkey, CliError> {
    let parsed = Pubkey::from_str(raw)
        .map_err(|_| operation_error(&format!("The {label} is not a canonical Solana address.")))?;
    if parsed.to_string() != raw {
        return Err(operation_error(&format!(
            "The {label} is not a canonical Solana address."
        )));
    }
    Ok(parsed)
}

fn rpc_client() -> Result<Client, CliError> {
    crate::backend::pinned_blocking_http_client_builder()
        .timeout(RPC_TIMEOUT)
        .build()
        .map_err(|_| operation_error("Could not open the Amoeba chain-read gateway."))
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
            "id": format!("petri-current-operation:{method}"),
            "method": method,
            "params": params,
        }))
        .send()
        .map_err(|_| {
            operation_error("The Amoeba chain-read gateway is temporarily unavailable.")
        })?;
    if !response.status().is_success() || response.status().is_redirection() {
        return Err(operation_error(
            "The Amoeba chain-read gateway rejected a finalized-state request.",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RPC_RESPONSE_BYTES as u64)
    {
        return Err(operation_error("The Solana response is too large."));
    }
    let mut bytes = Vec::with_capacity(16 * 1024);
    response
        .by_ref()
        .take((MAX_RPC_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| operation_error("The Solana response could not be read."))?;
    if bytes.len() > MAX_RPC_RESPONSE_BYTES {
        return Err(operation_error("The Solana response is too large."));
    }
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|_| operation_error("The Solana response is malformed."))?;
    if payload.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || payload.get("error").is_some()
    {
        return Err(operation_error(
            "The Amoeba chain-read gateway could not prove the required finalized state.",
        ));
    }
    payload
        .get("result")
        .cloned()
        .ok_or_else(|| operation_error("The Amoeba chain-read response omitted its result."))
}

fn operation_error(message: impl Into<String>) -> CliError {
    CliError::new(message)
}

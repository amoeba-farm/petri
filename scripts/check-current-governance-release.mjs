#!/usr/bin/env node

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (relative) => readFileSync(path.join(root, relative), "utf8");
const status = JSON.parse(read("release/current-governance-status.json"));

const expected = Object.freeze({
  sdk: "a21b324a7a64da87046c7650355b80ea20c47540",
  program: "2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw",
  programData: "8KR6hgcQehz32jm7CvrAriYNhvT2Bu9JuUWHce81J1oh",
  programAccountSha256: "83be84ac424d864aacf03e489ff8fe3f0a6957e8726c8f9c48f572df6b522e75",
  programDataAccountSha256: "df5e426d74dadcca84c42e999489ad3089421fe99c4ad5fc718e4b18eee5e610",
  payloadSha256: "903c58504f44e8820e5c9f99765b26be15a97504ec5fdd0855f3668f29927fee",
  controller: "8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx",
  gate: "Cdym9p7FvtxEAjF8XuCqSrishB7LmBDXaZGDMMgWczu",
  generationTwoGate: "CiszKKUZAUAb4DrZF8FBUJf136SVLY73d6J2MdyrinLL",
  compatibility: "governance-gate-v1",
  errorCode: "CURRENT_PROGRAM_WRITE_ABI_UNAVAILABLE",
});

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

assert(status.schema === "ameba.petri.current-governance-status.v1", "wrong status schema");
assert(status.packageSource?.candidateCommit === null, "self-referential candidate commit must stay null");
assert(status.sdk?.commit === expected.sdk, "wrong SDK candidate");
assert(status.liveDeployment?.identityGeneration === 3, "live generation must be three");
assert(status.liveDeployment?.sourceCommit === "b1931fecff5229da232f06e9c5c43b1d6328806d", "V3 artifact source mismatch");
assert(status.liveDeployment?.provenance === "finalized-rpc-account" &&
  status.liveDeployment?.minimumContextSlot === 497359082 &&
  status.liveDeployment?.finalizedObservationSlot === 497359082,
  "release metadata must bind the dated finalized identity capture");
assert(status.liveDeployment?.programId === expected.program, "wrong live program");
assert(status.liveDeployment?.programDataAddress === expected.programData, "wrong ProgramData");
assert(
  status.liveDeployment?.programAccountSha256 === expected.programAccountSha256,
  "wrong Program account hash",
);
assert(
  status.liveDeployment?.programDataAccountSha256 === expected.programDataAccountSha256,
  "wrong ProgramData account hash",
);
assert(
  status.liveDeployment?.programDataPayloadSha256 === expected.payloadSha256,
  "wrong ProgramData payload hash",
);
assert(status.liveGovernance?.controllerProgramId === expected.controller, "wrong live controller");
assert(status.liveGovernance?.protocolGatePda === expected.gate, "wrong live gate");
assert(
  status.reviewedGeneration2Candidate?.protocolGatePda === expected.generationTwoGate &&
    status.reviewedGeneration2Candidate?.live === false,
  "generation two must remain distinct and non-live",
);
assert(
  status.liveDeployment?.writeCompatibility === expected.compatibility &&
    status.writeRelease?.available === true &&
    status.writeRelease?.governedSpreadReleaseCommit === status.sourceHeads.spreadTools &&
    status.writeRelease?.governedInstructionManifestSha256 === "1aed01b9e8b251a01996ac880511d84fd42db48d259d385557a38de662387736",
  "write package must bind the exact deployed V3 manifest",
);
assert(status.runtimePermission?.available === false && status.runtimePermission?.revalidateBeforeEveryWrite === true, "recorded state cannot grant runtime permission");
for (const field of [
  "deploymentEnabled",
  "upgradeAuthorized",
  "authorityHandoffAuthorized",
  "productionEnabled",
  "mainnetEnabled",
]) {
  assert(status.authorization?.[field] === false, `authorization.${field} must remain false`);
}

const cargo = read("Cargo.toml");
const lock = read("Cargo.lock");
assert(cargo.includes(`rev = "${expected.sdk}"`), "Cargo.toml does not pin the SDK candidate");
assert(
  lock.includes(`ameba_sdk.git?rev=${expected.sdk}#${expected.sdk}`),
  "Cargo.lock does not resolve the SDK candidate",
);

const releaseSource = read("src/current_release.rs");
for (const value of [
  expected.sdk,
  expected.programAccountSha256,
  expected.programDataAccountSha256,
  expected.payloadSha256,
  expected.compatibility,
  expected.errorCode,
  "assert_current_write_release_available",
]) {
  assert(releaseSource.includes(value), `current_release.rs is missing ${value}`);
}

const identitySource = read("src/chain_identity.rs");
for (const value of [
  "validate_current_governance_gate_account_v1",
  "validate_current_governed_gate_account_v1",
  "current_release::selected_write_release()?",
  "validate_selected_governance_gate(selected.gate, owner, executable, &bytes)?",
  "program_account_sha256",
  "program_data_account_sha256",
  "identity_observed_slot > observed.observed_slot",
]) {
  assert(identitySource.includes(value), `chain identity is missing ${value}`);
}
const runtimeIdentitySource = identitySource.split("#[cfg(any())]", 1)[0];
const runtimeReleaseSource = releaseSource.split("#[cfg(test)]", 1)[0];
assert(
  !runtimeIdentitySource.includes(expected.generationTwoGate) &&
    !runtimeReleaseSource.includes(expected.generationTwoGate),
  "generation-two identity leaked into the runtime live allowlist",
);

const guardedFunctions = [
  ["src/main.rs", "run_collective_trade_prepare_command"],
  ["src/main.rs", "run_collective_trade_submit_command"],
  ["src/main.rs", "submit_writer_operation"],
  ["src/main.rs", "submit_flat_transfer"],
  ["src/main.rs", "run_writer_command"],
  ["src/current_operation.rs", "sign_submit_validated_operation"],
  ["src/staking.rs", "require_typed_staking_submission"],
  ["src/lab/trade.rs", "begin_confirmed_trade_submit"],
  ["src/lab/writers.rs", "activate_writer_confirmation"],
  ["src/lab/staking.rs", "activate_staking_confirmation"],
];
if (cargo.includes("developer-ops = []")) {
  guardedFunctions.push(
    ["src/onchain.rs", "typed_operator_submission_unavailable"],
    ["src/main.rs", "run_settlement_signer_governance_command"],
    ["src/main.rs", "run_oracle_economics_configure"],
    ["src/main.rs", "run_vault_authority_rotation_command"],
  );
}
for (const [relative, functionName] of guardedFunctions) {
  const source = read(relative);
  const start = source.indexOf(`fn ${functionName}`);
  assert(start >= 0, `missing guarded function ${functionName}`);
  assert(
    source.slice(start, start + 1_200).includes("require_current_write_release"),
    `${functionName} does not reach the static release gate before side effects`,
  );
}

const rpcSource = read("src/solana_rpc.rs");
const operationSource = read("src/current_operation.rs");
for (const boundary of ["CurrentGovernedOperationV1", "prepare_current_governed_signing_v1",
  "revalidate_current_governed_signed_transaction_v1", "signing.message().clone()"])
  assert(operationSource.includes(boundary), `missing current signing boundary ${boundary}`);
assert(!operationSource.includes("sign_submit_validated_instructions"), "raw instruction-only signing boundary remains");
assert(!rpcSource.includes("sendTransaction"), "a generic Solana broadcaster is present");
assert(
  rpcSource.includes("require_current_write_release"),
  "the retired submission facade is not fail-closed",
);
assert(read("src/cli.rs").includes(expected.errorCode), "CLI help omits the stable refusal code");
const publicCi = read(".github/workflows/ci.yml");
assert(
  publicCi.includes("cargo test --locked -j1 -- --test-threads=1"),
  "public CI does not compile its locked Rust test profile",
);
assert(
  publicCi.includes("check-current-governance-release.mjs") &&
    publicCi.includes("check-current-write-refusal.mjs"),
  "public CI does not enforce the static and runtime governance boundary",
);

console.log(
  `Current Petri governance release verified: V3 package capability with finalized runtime refusal; writes ${expected.compatibility}.`,
);

for (const name of ["run_collective_trade_prepare_command", "run_collective_trade_submit_command", "submit_writer_operation", "submit_flat_transfer"]) {
  const source = read("src/main.rs");
  const start = source.indexOf(`fn ${name}(`);
  const end = source.indexOf("\nfn ", start + 3);
  const body = source.slice(start, end < 0 ? undefined : end);
  const gate = body.indexOf("observe_current_write_context");
  assert(gate >= 0, `${name} omits finalized runtime preflight`);
  for (const boundary of ["signer_pubkey", "collective_trade_request", "post_json_with_current_state_retry"]) {
    const position = body.indexOf(boundary);
    assert(position < 0 || position > gate, `${name} crosses ${boundary} before runtime preflight`);
  }
}
assert(operationSource.includes("WriterSetupMode::ReleaseCompute => Err(operation_error("), "undeployed Writer V2 setup is not rejected");

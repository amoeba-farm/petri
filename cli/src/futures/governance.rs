//! Authenticated gate epochs for locally built and sponsor-prepared
//! instructions (`futures/docs/interfaces.md` §6): every instruction of a
//! governed target carries that target's gate account and signed epoch. The
//! futures program and a governed oracle are separate targets.
use super::{
    error::{Error, Result},
    market::{Deployment, pubkey},
    rpc::Rpc,
    settings::DEV_GENESIS,
};
use ameba_sdk_futures::futures::governance::{self as gate, GateBinding};
use serde::Deserialize;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_transaction::Transaction;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct GovernanceDeployment {
    pub program: String,
    pub targets: Vec<String>,
}

impl GovernanceDeployment {
    fn controller(&self) -> Result<Pubkey> {
        pubkey(&self.program, "governance controller")
    }

    fn targets(&self) -> Result<Vec<Pubkey>> {
        self.targets
            .iter()
            .map(|t| pubkey(t, "governed target"))
            .collect()
    }
}

/// A governed deployment pins a known controller and lists the futures
/// program it gates. A custom controller is accepted only on the dev chain's
/// genesis, and only in the dev profile (`dev`).
pub fn validate_deployment(
    g: Option<&GovernanceDeployment>,
    futures: Option<&Pubkey>,
    genesis: Option<&str>,
    dev: bool,
) -> Result<()> {
    let Some(g) = g else {
        return Ok(());
    };
    let controller = g.controller()?;
    let dev_chain = genesis == Some(DEV_GENESIS);
    if controller != gate::PRODUCTION_CONTROLLER_PROGRAM_ID && !(dev && dev_chain) {
        return Err(Error::msg(if dev_chain {
            "unrecognized governance controller: the dev chain's own controller needs --profile dev"
        } else {
            "unrecognized governance controller"
        }));
    }
    if controller == gate::LOCALNET_CONTROLLER_PROGRAM_ID && !dev_chain {
        return Err(Error::msg(
            "localnet governance requires the trusted localnet genesis",
        ));
    }
    let targets = g.targets()?;
    if let Some(futures) = futures
        && !targets.contains(futures)
    {
        return Err(Error::msg(
            "governed deployment must include the futures program",
        ));
    }
    Ok(())
}

/// The gate of `target`, read fresh from chain: `None` when the deployment
/// does not govern it. A missing, substituted or frozen gate is refused.
pub fn gate(dep: &Deployment, rpc: &Rpc, target: &Pubkey) -> Result<Option<GateBinding>> {
    Ok(bindings(dep, rpc, std::slice::from_ref(target))?
        .into_iter()
        .next())
}

/// The gates of the governed targets among `wanted`.
pub fn bindings(dep: &Deployment, rpc: &Rpc, wanted: &[Pubkey]) -> Result<Vec<GateBinding>> {
    let Some(g) = &dep.governance_controller else {
        return Ok(Vec::new());
    };
    let controller = g.controller()?;
    let mut targets = g.targets()?;
    targets.retain(|t| wanted.contains(t));
    targets.dedup();
    let keys = targets
        .iter()
        .map(|t| gate::gate_address(&controller, t).0)
        .collect::<Vec<_>>();
    let accounts = rpc.accounts(&keys)?;
    targets
        .iter()
        .zip(keys)
        .zip(accounts)
        .map(|((target, address), account)| {
            let a = account.ok_or_else(|| Error::msg("governance gate account is missing"))?;
            GateBinding::from_account(controller, *target, address, a.owner, &a.data)
                .map_err(|e| Error::msg(format!("governance gate refused: {e}")))
        })
        .collect()
}

fn bind(ix: Instruction, bindings: &[GateBinding]) -> Result<Instruction> {
    match bindings.iter().find(|b| b.target == ix.program_id) {
        Some(binding) => binding.bind(ix).map_err(|e| Error::msg(e.to_string())),
        None => Ok(ix),
    }
}

/// Binds every instruction of a governed target to its current gate.
pub fn bind_instructions(
    dep: &Deployment,
    rpc: &Rpc,
    instructions: Vec<Instruction>,
) -> Result<Vec<Instruction>> {
    let wanted = instructions
        .iter()
        .map(|ix| ix.program_id)
        .collect::<Vec<_>>();
    let bindings = bindings(dep, rpc, &wanted)?;
    instructions
        .into_iter()
        .map(|ix| bind(ix, &bindings))
        .collect()
}

/// Validate canonical readonly gate accounts and exact freshly read epochs,
/// then return a copy with raw instructions for the existing intent checker.
pub fn verify_prepared(dep: &Deployment, rpc: &Rpc, tx: &Transaction) -> Result<Transaction> {
    let wanted = tx
        .message
        .instructions
        .iter()
        .filter_map(|ix| {
            tx.message
                .account_keys
                .get(usize::from(ix.program_id_index))
        })
        .copied()
        .collect::<Vec<_>>();
    let bindings = bindings(dep, rpc, &wanted)?;
    verify_with_bindings(tx, &bindings)
}

fn verify_with_bindings(tx: &Transaction, bindings: &[GateBinding]) -> Result<Transaction> {
    let mut raw_tx = tx.clone();
    let msg = &tx.message;
    let signer_count = usize::from(msg.header.num_required_signatures);
    if signer_count > msg.account_keys.len()
        || usize::from(msg.header.num_readonly_signed_accounts) > signer_count
        || usize::from(msg.header.num_readonly_unsigned_accounts)
            > msg.account_keys.len() - signer_count
    {
        return Err(Error::msg("invalid prepared message privileges"));
    }
    let writable = |i: usize| {
        if i < signer_count {
            i < signer_count - usize::from(msg.header.num_readonly_signed_accounts)
        } else {
            i < msg.account_keys.len() - usize::from(msg.header.num_readonly_unsigned_accounts)
        }
    };
    for (raw, ci) in raw_tx
        .message
        .instructions
        .iter_mut()
        .zip(&msg.instructions)
    {
        let program = *msg
            .account_keys
            .get(usize::from(ci.program_id_index))
            .ok_or_else(|| Error::msg("invalid instruction program"))?;
        if !bindings.iter().any(|b| b.target == program) {
            continue;
        }
        let data_len = ci
            .data
            .len()
            .checked_sub(gate::GOVERNANCE_TAIL_LEN)
            .ok_or_else(|| Error::msg("missing signed governance epoch"))?;
        let account_len = ci
            .accounts
            .len()
            .checked_sub(1)
            .ok_or_else(|| Error::msg("missing governance gate account"))?;
        let keys = ci.accounts[..account_len]
            .iter()
            .map(|i| {
                let n = usize::from(*i);
                let key = *msg
                    .account_keys
                    .get(n)
                    .ok_or_else(|| Error::msg("invalid instruction account"))?;
                Ok(AccountMeta {
                    pubkey: key,
                    is_signer: n < signer_count,
                    is_writable: writable(n),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let ix = Instruction {
            program_id: program,
            accounts: keys,
            data: ci.data[..data_len].to_vec(),
        };
        let expected = bind(ix, bindings)?;
        if expected.data != ci.data || expected.accounts.len() != ci.accounts.len() {
            return Err(Error::msg(
                "prepared governance epoch does not match current gate state",
            ));
        }
        for (meta, i) in expected.accounts[account_len..]
            .iter()
            .zip(&ci.accounts[account_len..])
        {
            let n = usize::from(*i);
            if msg.account_keys.get(n) != Some(&meta.pubkey) || n < signer_count || writable(n) {
                return Err(Error::msg(
                    "prepared governance gate is substituted, writable or a signer",
                ));
            }
        }
        raw.data.truncate(data_len);
        raw.accounts.truncate(account_len);
    }
    Ok(raw_tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::{hash::Hash, message::Message};

    fn fixture() -> (GovernanceDeployment, Vec<GateBinding>, Instruction, Pubkey) {
        let futures = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let controller = gate::LOCALNET_CONTROLLER_PROGRAM_ID;
        let governance = GovernanceDeployment {
            program: controller.to_string(),
            targets: vec![futures.to_string()],
        };
        let bindings = vec![GateBinding {
            controller,
            target: futures,
            address: gate::gate_address(&controller, &futures).0,
            epoch: 7,
        }];
        let raw = Instruction {
            program_id: futures,
            data: vec![3, 1, 2],
            accounts: vec![
                AccountMeta::new(owner, true),
                AccountMeta::new(Pubkey::new_unique(), false),
                AccountMeta::new_readonly(Pubkey::new_unique(), false),
            ],
        };
        (governance, bindings, raw, owner)
    }

    #[test]
    fn a_custom_controller_requires_the_actual_localnet_genesis() {
        let (g, _, raw, _) = fixture();
        let futures = raw.program_id;
        assert!(
            validate_deployment(Some(&g), Some(&futures), Some("another-chain"), true).is_err()
        );
        validate_deployment(Some(&g), Some(&futures), Some(DEV_GENESIS), true).unwrap();
        // A governed deployment must gate its futures program.
        assert!(
            validate_deployment(
                Some(&g),
                Some(&Pubkey::new_unique()),
                Some(DEV_GENESIS),
                true
            )
            .is_err()
        );
        validate_deployment(None, Some(&futures), None, false).unwrap();
    }

    #[test]
    fn the_dev_chains_controller_needs_the_dev_profile() {
        let (g, _, raw, _) = fixture();
        let futures = raw.program_id;
        let e =
            validate_deployment(Some(&g), Some(&futures), Some(DEV_GENESIS), false).unwrap_err();
        assert!(e.message.contains("--profile dev"), "{}", e.message);
        // The production controller is accepted in either profile.
        let production = GovernanceDeployment {
            program: gate::PRODUCTION_CONTROLLER_PROGRAM_ID.to_string(),
            targets: vec![futures.to_string()],
        };
        for dev in [false, true] {
            validate_deployment(
                Some(&production),
                Some(&futures),
                Some(crate::futures::settings::MAINNET_GENESIS),
                dev,
            )
            .unwrap();
        }
    }

    #[test]
    fn prepared_gate_validation_rejects_stale_substituted_and_writable_gates() {
        let (_, mut bindings, raw, owner) = fixture();
        let bound = bind(raw.clone(), &bindings).unwrap();
        assert_eq!(bound.accounts.len(), raw.accounts.len() + 1);
        assert_eq!(bound.data.len(), raw.data.len() + gate::GOVERNANCE_TAIL_LEN);
        let tx = Transaction::new_unsigned(Message::new_with_blockhash(
            std::slice::from_ref(&bound),
            Some(&owner),
            &Hash::default(),
        ));
        let checked = verify_with_bindings(&tx, &bindings).unwrap();
        assert_eq!(checked.message.instructions[0].data, raw.data);
        assert_eq!(
            checked.message.instructions[0].accounts.len(),
            raw.accounts.len()
        );
        bindings[0].epoch += 1;
        assert!(verify_with_bindings(&tx, &bindings).is_err());
        bindings[0].epoch -= 1;
        let mut altered = bound.clone();
        altered.accounts.last_mut().unwrap().pubkey = Pubkey::new_unique();
        let altered = Transaction::new_unsigned(Message::new(&[altered], Some(&owner)));
        assert!(verify_with_bindings(&altered, &bindings).is_err());
        let mut altered = bound;
        altered.accounts.last_mut().unwrap().is_writable = true;
        let altered = Transaction::new_unsigned(Message::new(&[altered], Some(&owner)));
        assert!(verify_with_bindings(&altered, &bindings).is_err());
        let raw = Transaction::new_unsigned(Message::new(&[raw], Some(&owner)));
        assert!(verify_with_bindings(&raw, &bindings).is_err());
    }

    #[test]
    fn instructions_of_ungoverned_programs_stay_raw() {
        let (_, bindings, _, owner) = fixture();
        let other = Instruction {
            program_id: Pubkey::new_unique(),
            data: vec![1],
            accounts: vec![AccountMeta::new(owner, true)],
        };
        assert_eq!(bind(other.clone(), &bindings).unwrap(), other);
        let tx = Transaction::new_unsigned(Message::new(&[other], Some(&owner)));
        assert_eq!(verify_with_bindings(&tx, &bindings).unwrap(), tx);
    }
}

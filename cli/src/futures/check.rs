//! Checks a sponsor-prepared futures transaction before the owner signs it:
//! the sponsor pays the fee, only the compute budget, the owner's own USDC
//! account creation, the trusted futures program and decompressions of the
//! owner's compressed USDC into their own USDC account (`light`) are called,
//! every futures step targets this market with the owner as signer and the
//! owner's own token account, and the steps do exactly what was asked.

use ameba_sdk_futures::futures::{FuturesInstruction, NO_CLAIM, SubmitArgs};
use borsh::BorshDeserialize;
use solana_program::pubkey::Pubkey;
use solana_transaction::Transaction;

use super::{
    error::{Error, Result, bail},
    light,
    txbuild::{ATA_PROGRAM, COMPUTE_BUDGET_PROGRAM},
    units::USDC_DECIMALS,
};

/// What the owner asked for.
#[derive(Clone, Debug)]
pub enum Intent {
    /// `deposit` may top up at most `max_deposit`; `withdraw` sweeps all
    /// free cash back afterwards.
    Submit {
        args: SubmitArgs,
        withdraw: bool,
        max_deposit: u128,
    },
    Deposit {
        amount: u64,
    },
    /// `u64::MAX` withdraws all free cash.
    Withdraw {
        amount: u64,
    },
    Cancel {
        slot: u16,
        seq: u64,
        withdraw: bool,
    },
    /// A transaction that only moves the owner's compressed USDC into their
    /// USDC account, ahead of the one that spends it (the API's `before`).
    Decompress,
}

/// The accounts the transaction must stay within.
pub struct Scope<'a> {
    pub owner: &'a Pubkey,
    pub sponsor: &'a Pubkey,
    pub program: &'a Pubkey,
    pub market: &'a Pubkey,
    pub owner_token: &'a Pubkey,
    pub mint: &'a Pubkey,
}

pub fn check(tx: &Transaction, s: &Scope, intent: &Intent) -> Result<()> {
    let msg = &tx.message;
    let keys = &msg.account_keys;
    let signers = usize::from(msg.header.num_required_signatures);
    let payer = keys
        .first()
        .ok_or_else(|| Error::msg("prepared transaction has no fee payer"))?;
    if payer == s.owner {
        bail("refusing: the prepared transaction makes you the fee payer")?;
    }
    if payer != s.sponsor {
        bail(format!(
            "refusing: the fee payer {payer} is not the sponsor {}",
            s.sponsor
        ))?;
    }
    if signers > 2 || !keys[..signers.min(keys.len())].contains(s.owner) {
        bail("refusing: the prepared transaction asks for unexpected signers")?;
    }
    let mut deposited: u128 = 0;
    let mut decompressed: u128 = 0;
    let mut did = (false, false, false); // (main step, withdraw, ata)
    for (i, ci) in msg.instructions.iter().enumerate() {
        let program = keys
            .get(usize::from(ci.program_id_index))
            .ok_or_else(|| Error::msg("prepared instruction names no program"))?;
        let account = |n: usize| ci.accounts.get(n).and_then(|a| keys.get(usize::from(*a)));
        if *program == COMPUTE_BUDGET_PROGRAM {
            continue;
        }
        if *program == ATA_PROGRAM {
            if ci.data != [1] || account(2) != Some(s.owner) || account(3) != Some(s.mint) {
                bail(format!(
                    "refusing: step {i} is not an idempotent creation of your USDC account"
                ))?;
            }
            did.2 = true;
            continue;
        }
        if *program == light::PROGRAM {
            let expect = light::Expect {
                owner: s.owner,
                sponsor: s.sponsor,
                mint: s.mint,
                decimals: USDC_DECIMALS as u8,
                destination: s.owner_token,
            };
            decompressed += u128::from(light::check(keys, ci, i, &expect)?);
            continue;
        }
        if program != s.program {
            bail(format!(
                "refusing: step {i} calls unexpected program {program}"
            ))?;
        }
        if account(0) != Some(s.market) {
            bail(format!("refusing: step {i} targets another market"))?;
        }
        if account(1) != Some(s.owner) {
            bail(format!("refusing: step {i} acts for another owner"))?;
        }
        let ix = FuturesInstruction::try_from_slice(&ci.data)
            .map_err(|_| Error::msg(format!("refusing: step {i} is not a futures instruction")))?;
        let token_ok = account(2) == Some(s.owner_token);
        let ok = match (&ix, intent) {
            (FuturesInstruction::Deposit { amount }, Intent::Deposit { amount: want }) => {
                did.0 = true;
                token_ok && amount == want
            }
            (FuturesInstruction::Deposit { amount }, Intent::Submit { .. }) => {
                deposited = deposited.saturating_add(u128::from(*amount));
                token_ok
            }
            (FuturesInstruction::Withdraw { amount }, Intent::Withdraw { amount: want }) => {
                did.0 = true;
                token_ok && amount == want
            }
            (
                FuturesInstruction::Withdraw { amount },
                Intent::Submit { withdraw: true, .. } | Intent::Cancel { withdraw: true, .. },
            ) => {
                did.1 = true;
                token_ok && *amount == u64::MAX
            }
            (
                FuturesInstruction::Submit {
                    side,
                    lo,
                    hi,
                    lots,
                    limit,
                    min_fill,
                    mode,
                    close_claim,
                    close_seq,
                    ..
                },
                Intent::Submit { args, .. },
            ) => {
                did.0 = true;
                let (claim, seq) = args.close.unwrap_or((NO_CLAIM, 0));
                *side == args.side as u8
                    && *lo == args.lo
                    && *hi == args.hi
                    && *lots == args.lots
                    && *limit == args.limit
                    && *mode == args.mode
                    && *min_fill >= args.min_fill
                    && *min_fill <= args.lots
                    && *close_claim == claim
                    && (args.close.is_none() || *close_seq == seq)
            }
            (
                FuturesInstruction::Cancel { slot, seq },
                Intent::Cancel {
                    slot: s0, seq: q, ..
                },
            ) => {
                did.0 = true;
                slot == s0 && seq == q
            }
            _ => false,
        };
        if !ok {
            bail(format!(
                "refusing: step {i} ({}) is not what you asked for",
                name(&ix)
            ))?;
        }
    }
    if matches!(intent, Intent::Decompress) {
        if decompressed == 0 {
            bail("refusing: the prepared transaction moves none of your compressed USDC")?;
        }
        return Ok(());
    }
    if !did.0 {
        bail("refusing: the prepared transaction leaves out the requested step")?;
    }
    if let Intent::Submit { max_deposit, .. } = intent
        && deposited > *max_deposit
    {
        bail(format!(
            "refusing: the prepared transaction deposits {deposited} USDC atoms, more than the order can need ({max_deposit})"
        ))?;
    }
    let wants_withdraw = matches!(
        intent,
        Intent::Submit { withdraw: true, .. } | Intent::Cancel { withdraw: true, .. }
    );
    if wants_withdraw != did.1 {
        bail("refusing: the prepared transaction's withdrawal is not what you asked for")?;
    }
    Ok(())
}

fn name(ix: &FuturesInstruction) -> &'static str {
    match ix {
        FuturesInstruction::Deposit { .. } => "deposit",
        FuturesInstruction::Withdraw { .. } => "withdraw",
        FuturesInstruction::Submit { .. } => "submit",
        FuturesInstruction::Cancel { .. } => "cancel",
        _ => "another instruction",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ameba_sdk_futures::futures::{MODE_IOC, Market, Side};
    use solana_program::{hash::Hash, instruction::Instruction};

    use crate::futures::txbuild::{ata, compute_unit_limit, create_ata_idempotent};

    struct Fx {
        owner: Pubkey,
        sponsor: Pubkey,
        mint: Pubkey,
        market: Market,
    }

    fn fx() -> Fx {
        let program = Pubkey::new_unique();
        Fx {
            owner: Pubkey::new_unique(),
            sponsor: Pubkey::new_unique(),
            mint: Pubkey::new_unique(),
            market: Market {
                program,
                market: Pubkey::new_unique(),
                registry: Pubkey::new_unique(),
                authority: Pubkey::new_unique(),
                vault: Pubkey::new_unique(),
                period: Pubkey::new_unique(),
                mint: Pubkey::new_unique(),
            },
        }
    }

    fn tx(payer: &Pubkey, ixs: Vec<Instruction>) -> Transaction {
        let mut t = Transaction::new_with_payer(&ixs, Some(payer));
        t.message.recent_blockhash = Hash::default();
        t
    }

    fn args() -> SubmitArgs {
        SubmitArgs {
            side: Side::Long,
            lo: 89,
            hi: 120,
            lots: 100,
            limit: 145_965,
            min_fill: 1,
            mode: MODE_IOC,
            close: None,
        }
    }

    fn run(f: &Fx, t: &Transaction, intent: &Intent) -> Result<()> {
        let token = ata(&f.owner, &f.mint);
        check(
            t,
            &Scope {
                owner: &f.owner,
                sponsor: &f.sponsor,
                program: &f.market.program,
                market: &f.market.market,
                owner_token: &token,
                mint: &f.mint,
            },
            intent,
        )
    }

    #[test]
    fn a_faithful_submit_passes_and_tampering_is_refused() {
        let f = fx();
        let token = ata(&f.owner, &f.mint);
        let intent = Intent::Submit {
            args: args(),
            withdraw: false,
            max_deposit: 14_596_500,
        };
        let good = vec![
            compute_unit_limit(1_400_000),
            f.market.deposit(&f.owner, &token, 14_310_200),
            f.market.submit(&f.owner, &args(), vec![1, 2, 3]),
        ];
        run(&f, &tx(&f.sponsor, good.clone()), &intent).unwrap();
        // The owner as fee payer.
        assert!(run(&f, &tx(&f.owner, good.clone()), &intent).is_err());
        // A larger deposit than the order can need.
        let mut big = good.clone();
        big[1] = f.market.deposit(&f.owner, &token, 20_000_000);
        assert!(run(&f, &tx(&f.sponsor, big), &intent).is_err());
        // A worse limit.
        let mut worse = args();
        worse.limit += 1;
        let mut t = good.clone();
        t[2] = f.market.submit(&f.owner, &worse, vec![]);
        assert!(run(&f, &tx(&f.sponsor, t), &intent).is_err());
        // Another market.
        let mut other = f.market;
        other.market = Pubkey::new_unique();
        let mut t = good.clone();
        t[2] = other.submit(&f.owner, &args(), vec![]);
        assert!(run(&f, &tx(&f.sponsor, t), &intent).is_err());
        // An unasked withdrawal.
        let mut t = good.clone();
        t.push(f.market.withdraw(&f.owner, &token, u64::MAX));
        assert!(run(&f, &tx(&f.sponsor, t), &intent).is_err());
        // A withdrawal to someone else's token account.
        let intent_w = Intent::Submit {
            args: args(),
            withdraw: true,
            max_deposit: 14_596_500,
        };
        let mut t = good.clone();
        t.push(create_ata_idempotent(&f.sponsor, &f.owner, &f.mint));
        t.push(f.market.withdraw(&f.owner, &token, u64::MAX));
        run(&f, &tx(&f.sponsor, t.clone()), &intent_w).unwrap();
        let thief = Pubkey::new_unique();
        let last = t.len() - 1;
        t[last] = f.market.withdraw(&f.owner, &thief, u64::MAX);
        assert!(run(&f, &tx(&f.sponsor, t), &intent_w).is_err());
    }

    #[test]
    fn deposits_withdrawals_and_cancels_match_exactly() {
        let f = fx();
        let token = ata(&f.owner, &f.mint);
        let dep = vec![
            compute_unit_limit(1_400_000),
            f.market.deposit(&f.owner, &token, 5),
        ];
        run(
            &f,
            &tx(&f.sponsor, dep.clone()),
            &Intent::Deposit { amount: 5 },
        )
        .unwrap();
        assert!(run(&f, &tx(&f.sponsor, dep), &Intent::Deposit { amount: 4 }).is_err());
        let wd = vec![f.market.withdraw(&f.owner, &token, u64::MAX)];
        run(
            &f,
            &tx(&f.sponsor, wd.clone()),
            &Intent::Withdraw { amount: u64::MAX },
        )
        .unwrap();
        assert!(run(&f, &tx(&f.sponsor, wd), &Intent::Withdraw { amount: 7 }).is_err());
        let cancel = vec![f.market.cancel(&f.owner, 3, 17)];
        let intent = Intent::Cancel {
            slot: 3,
            seq: 17,
            withdraw: false,
        };
        run(&f, &tx(&f.sponsor, cancel), &intent).unwrap();
        let wrong = vec![f.market.cancel(&f.owner, 4, 17)];
        assert!(run(&f, &tx(&f.sponsor, wrong), &intent).is_err());
        // An empty transaction does nothing asked for.
        assert!(run(&f, &tx(&f.sponsor, vec![compute_unit_limit(1)]), &intent).is_err());
    }

    #[test]
    fn your_compressed_usdc_may_be_moved_into_your_own_account_only() {
        let f = fx();
        let owner_token = ata(&f.owner, &f.mint);
        let step = |to: &Pubkey| crate::futures::light::api_step(&f.sponsor, &f.owner, &f.mint, to);
        let scope = Scope {
            owner: &f.owner,
            sponsor: &f.sponsor,
            program: &f.market.program,
            market: &f.market.market,
            owner_token: &owner_token,
            mint: &f.mint,
        };
        let deposit = f.market.deposit(&f.owner, &owner_token, 12);
        // In the transaction that spends it, ahead of the deposit.
        let main = tx(
            &f.sponsor,
            vec![
                compute_unit_limit(400_000),
                step(&owner_token),
                deposit.clone(),
            ],
        );
        assert!(check(&main, &scope, &Intent::Deposit { amount: 12 }).is_ok());
        // In a transaction of its own, ahead of the order.
        let before = tx(
            &f.sponsor,
            vec![
                create_ata_idempotent(&f.sponsor, &f.owner, &f.mint),
                step(&owner_token),
            ],
        );
        assert!(check(&before, &scope, &Intent::Decompress).is_ok());
        // Paid into anyone else's account: refused.
        let elsewhere = tx(
            &f.sponsor,
            vec![step(&Pubkey::new_unique()), deposit.clone()],
        );
        let e = check(&elsewhere, &scope, &Intent::Deposit { amount: 12 }).unwrap_err();
        assert!(
            e.message.contains("not a decompression of your own USDC"),
            "{}",
            e.message
        );
        // A decompression transaction that also does something else, or
        // nothing: refused.
        let sneaky = tx(&f.sponsor, vec![step(&owner_token), deposit]);
        assert!(check(&sneaky, &scope, &Intent::Decompress).is_err());
        let empty = tx(&f.sponsor, vec![compute_unit_limit(400_000)]);
        assert!(check(&empty, &scope, &Intent::Decompress).is_err());
    }
}

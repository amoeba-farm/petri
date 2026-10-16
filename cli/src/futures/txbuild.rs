//! Named transaction plans and the instructions every futures and oracle
//! write shares: the compute-unit limit and the owner's USDC token account.

use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey,
    pubkey::Pubkey,
};

pub const COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
pub const COMPUTE_BUDGET_PROGRAM: Pubkey = pubkey!("ComputeBudget111111111111111111111111111111");
pub const ATA_PROGRAM: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const SYSTEM_PROGRAM: Pubkey = pubkey!("11111111111111111111111111111111");
/// Solana's packet limit for a serialized transaction.
pub const PACKET_SIZE: usize = 1232;

/// `SetComputeUnitLimit(units)`.
pub fn compute_unit_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: COMPUTE_BUDGET_PROGRAM,
        accounts: vec![],
        data,
    }
}

/// The owner's associated token account for `mint`.
pub fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), spl_token::id().as_ref(), mint.as_ref()],
        &ATA_PROGRAM,
    )
    .0
}

/// Idempotent creation of `owner`'s ATA, paid by `payer`.
pub fn create_ata_idempotent(payer: &Pubkey, owner: &Pubkey, mint: &Pubkey) -> Instruction {
    Instruction {
        program_id: ATA_PROGRAM,
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(ata(owner, mint), false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: vec![1],
    }
}

/// Named instructions, starting with the compute-unit limit.
#[derive(Clone, Debug)]
pub struct Plan {
    pub steps: Vec<(&'static str, Instruction)>,
}

impl Default for Plan {
    fn default() -> Self {
        Self::new()
    }
}

impl Plan {
    pub fn new() -> Self {
        Self {
            steps: vec![("computeBudget", compute_unit_limit(COMPUTE_UNIT_LIMIT))],
        }
    }

    pub fn push(&mut self, name: &'static str, ix: Instruction) -> &mut Self {
        self.steps.push((name, ix));
        self
    }

    pub fn instructions(&self) -> Vec<Instruction> {
        self.steps.iter().map(|(_, ix)| ix.clone()).collect()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.steps.iter().map(|(n, _)| *n).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_budget_and_ata_match_their_programs() {
        let ix = compute_unit_limit(1_400_000);
        assert_eq!(ix.data, vec![2, 0xc0, 0x5c, 0x15, 0x00]);
        assert!(ix.accounts.is_empty());
        let owner = Pubkey::new_from_array([7; 32]);
        let mint = Pubkey::new_from_array([6; 32]);
        let create = create_ata_idempotent(&owner, &owner, &mint);
        assert_eq!(create.data, vec![1]);
        assert_eq!(create.accounts[1].pubkey, ata(&owner, &mint));
        assert_eq!(create.accounts[4].pubkey, Pubkey::default());
        // The dev market maker's USDC account, as the futures API reports it.
        let owner = Pubkey::from_str_const("C6B4a71wjn8nF4rBa3VWGppoi9EXsgvx1JGVZdj8VAK3");
        let usdc = Pubkey::from_str_const("GSUW4ohqFyJnJvC7GiQPwsseFEYXY6uLkPjYUYQNRmbZ");
        assert_eq!(
            ata(&owner, &usdc).to_string(),
            "3gb6Vad553spfDGWoHdy5a1UY9ox1VmFsYav4jDCTeki"
        );
    }

    #[test]
    fn plans_start_with_the_compute_limit() {
        let mut plan = Plan::new();
        let owner = Pubkey::new_from_array([7; 32]);
        let mint = Pubkey::new_from_array([6; 32]);
        plan.push("createAta", create_ata_idempotent(&owner, &owner, &mint));
        assert_eq!(plan.names(), ["computeBudget", "createAta"]);
        assert_eq!(
            plan.instructions()[0],
            compute_unit_limit(COMPUTE_UNIT_LIMIT)
        );
    }
}

//! Oracle v2's accounts read from chain: each decoded and checked exactly as the program loads
//! it (`ameba_oracle::heads_view::decode`: owner, exact length, discriminator, version, field
//! domains, zero tail, invariants, canonical PDA), and bound to the account it belongs to.
//! Scans go by layout (size and discriminator) and, where a read wants one owner's accounts, by
//! that owner's key in the layout; through the futures API's kept scans for a read
//! (`/v1/oracle/accounts`, `Oracle::reads`), on the RPC otherwise.

use std::collections::HashMap;

use ameba_commodity_spec::Product;
use ameba_oracle::{
    heads_view::{self as hv, BoardAccounts, Weights},
    instruction::heads::HeadAddresses,
    state::heads::{
        Delegation, Head, HeadChallenge, HeadClaim, HeadsAccount, HeadsBoard, HeadsConfig,
        HeadsContract, HeadsContractStatus, HeadsEpoch, HeadsStake, find_heads_board_address,
        find_heads_config_address, find_heads_epoch_address, find_heads_stake_address,
        find_heads_weights_address,
    },
};
use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::{
    futures::{
        ctx::Ctx,
        error::{Error, Result, bail},
        rpc::{Account, Filter},
    },
    oracle::{chain::Oracle, prints::ChainNow, skus, writes::OBJECT_HEADER},
};

/// Where an owner's key sits in each layout (the flat codec lays the fields out in order after
/// the discriminator, version and bump: `tests::offsets_are_the_layouts`).
pub mod offset {
    pub const HEAD_BOARD: usize = 10;
    pub const DELEGATION_OWNER: usize = 42;
    pub const DELEGATION_BOARD: usize = 74;
    pub const CLAIM_HEAD: usize = 10;
    pub const CLAIM_FETCHER: usize = 80;
    pub const CHALLENGE_TARGET: usize = 10;
    pub const CHALLENGE_HEAD: usize = 42;
    pub const CHALLENGE_CHALLENGER: usize = 74;
    pub const CONTRACT_BOARD: usize = 42;
}

/// The oracle program's heads accounts, through [`Oracle`]'s reads.
pub struct Chain<'a> {
    pub oracle: Oracle<'a>,
}

/// A product's board: its symbol, id, address and account.
pub struct BoardRef {
    pub product: Product,
    pub id: [u8; 32],
    pub address: Pubkey,
    pub board: Box<HeadsBoard>,
}

impl BoardRef {
    pub fn symbol(&self) -> &'static str {
        self.product.symbol()
    }

    pub fn params(&self) -> ameba_oracle_heads::Params {
        self.board.params.to_params()
    }

    pub fn head_addresses(&self, program: &Pubkey, head: u32) -> HeadAddresses {
        HeadAddresses::derive(program, &self.id, head)
    }

    /// `min_post` of the board now, as `PostHead` computes it: `n` its live sources that carry
    /// weight (`thresholds::min_post`).
    pub fn min_post(&self) -> u64 {
        let b = &self.board;
        let live = usize::from(b.live_count).min(b.live_flags.len());
        let weighted = b.live_flags[..live]
            .iter()
            .filter(|f| *f & ameba_oracle::state::heads::LIVE_DUPLICATE == 0)
            .count();
        let p = self.params();
        ameba_oracle_heads::thresholds::min_post(
            p.floor_post,
            p.alpha_bps,
            b.total_delegation,
            weighted,
        )
    }

    /// The activation threshold now (`thresholds::activation_threshold` over the live sources
    /// that carry weight).
    pub fn activation(&self) -> u64 {
        let b = &self.board;
        let live = usize::from(b.live_count).min(b.live_flags.len());
        let mut delegations: Vec<u64> = (0..live)
            .filter(|&i| b.live_flags[i] & ameba_oracle::state::heads::LIVE_DUPLICATE == 0)
            .map(|i| b.live_delegations[i])
            .collect();
        let p = self.params();
        ameba_oracle_heads::thresholds::activation_threshold(
            p.floor_activate,
            p.beta_bps,
            &mut delegations,
        )
    }
}

/// The product of a `--product` value: RAMX, NANDX or BATX, any case.
pub fn product(text: &str) -> Result<Product> {
    Product::from_symbol(&text.trim().to_ascii_uppercase()).ok_or_else(|| {
        Error::msg(format!(
            "`{text}` is not a product with a heads board: use RAMX, NANDX or BATX"
        ))
    })
}

fn filters<T: HeadsAccount>(memcmp: &[(usize, &Pubkey)]) -> Vec<Filter> {
    let mut out = vec![
        Filter::DataSize(T::LEN as u64),
        Filter::Memcmp {
            offset: 0,
            bytes: T::DISCRIMINATOR.to_vec(),
        },
    ];
    out.extend(memcmp.iter().map(|(offset, key)| Filter::Memcmp {
        offset: *offset,
        bytes: key.to_bytes().to_vec(),
    }));
    out
}

impl<'a> Chain<'a> {
    /// For a write: every account read from the RPC.
    pub fn new(ctx: &'a Ctx) -> Result<Self> {
        Ok(Self {
            oracle: Oracle::new(ctx)?,
        })
    }

    /// For a read: scans from the futures API while no RPC is set (`Oracle::reads`).
    pub fn reads(ctx: &'a Ctx) -> Result<Self> {
        Ok(Self {
            oracle: Oracle::reads(ctx)?,
        })
    }

    pub fn ctx(&self) -> &'a Ctx {
        self.oracle.ctx
    }

    pub fn program(&self) -> Pubkey {
        self.oracle.program
    }

    /// The chain's clock.
    pub fn now(&self) -> Result<ChainNow> {
        ChainNow::read(self.ctx())
    }

    fn decode<T: HeadsAccount>(&self, key: &Pubkey, account: Option<&Account>) -> Option<Box<T>> {
        let account = account?;
        hv::decode::<T>(&self.oracle.program, key, &account.owner, &account.data)
    }

    /// The account at `key`, `None` when it does not exist; one that exists and fails the
    /// program's checks is refused.
    pub fn maybe<T: HeadsAccount>(&self, key: &Pubkey, what: &str) -> Result<Option<Box<T>>> {
        let account = self.ctx().rpc.account(key)?;
        match account {
            None => Ok(None),
            Some(a) => self.decode::<T>(key, Some(&a)).map(Some).ok_or_else(|| {
                Error::msg(format!("{what} {key} is not a valid oracle heads account"))
            }),
        }
    }

    /// Several accounts by address in one read; those missing or invalid are `None`.
    pub fn many<T: HeadsAccount>(&self, keys: &[Pubkey]) -> Result<Vec<Option<Box<T>>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(keys.len());
        for chunk in keys.chunks(100) {
            let found = self.ctx().rpc.accounts(chunk)?;
            out.extend(
                chunk
                    .iter()
                    .zip(found)
                    .map(|(k, a)| self.decode::<T>(k, a.as_ref())),
            );
        }
        Ok(out)
    }

    /// Every account of layout `T` whose bytes at each offset are the key given, decoded (those
    /// that fail the program's checks are left out), by address.
    pub fn scan<T: HeadsAccount>(
        &self,
        memcmp: &[(usize, &Pubkey)],
    ) -> Result<Vec<(Pubkey, Box<T>)>> {
        let found = self.oracle.program_accounts(&filters::<T>(memcmp))?;
        let mut out: Vec<(Pubkey, Box<T>)> = found
            .iter()
            .filter_map(|(k, a)| self.decode::<T>(k, Some(a)).map(|v| (*k, v)))
            .collect();
        out.sort_by_key(|(k, _)| *k);
        Ok(out)
    }

    /// The heads config: oracle v2 is open on this network.
    pub fn config(&self) -> Result<Box<HeadsConfig>> {
        let key = find_heads_config_address(&self.oracle.program).0;
        self.maybe::<HeadsConfig>(&key, "heads config")?
            .ok_or_else(|| Error::msg("Oracle staking isn't open on this network yet."))
    }

    pub fn board_address(&self, product: Product) -> Result<([u8; 32], Pubkey)> {
        let id = skus::encode_label(product.symbol())?;
        Ok((id, find_heads_board_address(&self.oracle.program, &id).0))
    }

    /// A product's board.
    pub fn board(&self, product: Product) -> Result<BoardRef> {
        let (id, address) = self.board_address(product)?;
        let board = self
            .maybe::<HeadsBoard>(&address, "board")?
            .ok_or_else(|| {
                Error::msg(format!(
                    "{} has no heads board on this network yet.",
                    product.symbol()
                ))
            })?;
        Ok(BoardRef {
            product,
            id,
            address,
            board,
        })
    }

    /// Head `id` of `board`.
    pub fn head(&self, board: &BoardRef, id: u32) -> Result<(Pubkey, Box<Head>)> {
        let key = board.head_addresses(&self.oracle.program, id).head;
        let head = self.maybe::<Head>(&key, "head")?.ok_or_else(|| {
            Error::msg(format!(
                "{} has no head {id}: its heads are 0 to {}.",
                board.symbol(),
                board.board.head_count.saturating_sub(1)
            ))
        })?;
        if head.board != board.address || head.head_id != id {
            bail(format!("head {key} is not head {id} of {}", board.symbol()))?;
        }
        Ok((key, head))
    }

    pub fn stake_address(&self, owner: &Pubkey) -> Pubkey {
        find_heads_stake_address(&self.oracle.program, owner).0
    }

    /// `owner`'s stake account, if they ever staked.
    pub fn stake(&self, owner: &Pubkey) -> Result<Option<Box<HeadsStake>>> {
        let stake = self.maybe::<HeadsStake>(&self.stake_address(owner), "stake account")?;
        if stake.as_ref().is_some_and(|s| s.owner != *owner) {
            bail(format!("the stake account of {owner} names another owner"))?;
        }
        Ok(stake)
    }

    /// `owner`'s free stake (0 without a stake account).
    pub fn free(&self, owner: &Pubkey) -> Result<u64> {
        Ok(self.stake(owner)?.map_or(0, |s| s.free))
    }

    /// `owner`'s delegation on the head at `head`.
    pub fn delegation(
        &self,
        a: &HeadAddresses,
        owner: &Pubkey,
    ) -> Result<Option<(Pubkey, Box<Delegation>)>> {
        let key = a.delegation(&self.oracle.program, owner);
        let d = self.maybe::<Delegation>(&key, "delegation")?;
        if d.as_ref()
            .is_some_and(|d| d.head != a.head || d.owner != *owner || d.board != a.board)
        {
            bail(format!(
                "delegation {key} does not belong to {owner} on head {}",
                a.head
            ))?;
        }
        Ok(d.map(|d| (key, d)))
    }

    /// Claim `seq` of the head.
    pub fn claim(
        &self,
        a: &HeadAddresses,
        head_id: u32,
        seq: u32,
    ) -> Result<(Pubkey, Box<HeadClaim>)> {
        let key = a.claim(&self.oracle.program, seq);
        let claim = self
            .maybe::<HeadClaim>(&key, "claim")?
            .ok_or_else(|| Error::msg(format!("Head {head_id} has no claim {seq}.")))?;
        if claim.head != a.head || claim.seq != seq {
            bail(format!("claim {key} is not claim {seq} of head {head_id}"))?;
        }
        Ok((key, claim))
    }

    /// The challenges of a claim or of a head's source case (`target`), by their index.
    pub fn challenges_of(
        &self,
        head: &Pubkey,
        target: &Pubkey,
    ) -> Result<Vec<(Pubkey, Box<HeadChallenge>)>> {
        let mut found: Vec<(Pubkey, Box<HeadChallenge>)> = self.scan::<HeadChallenge>(&[
            (offset::CHALLENGE_TARGET, target),
            (offset::CHALLENGE_HEAD, head),
        ])?;
        found.sort_by_key(|(_, c)| c.index);
        Ok(found)
    }

    /// A prize epoch of `board`, if it exists.
    pub fn epoch(&self, board: &Pubkey, month: u32) -> Result<Option<Box<HeadsEpoch>>> {
        let key = find_heads_epoch_address(&self.oracle.program, board, month).0;
        self.maybe::<HeadsEpoch>(&key, "prize epoch")
    }

    /// Everything [`hv::view`] reads for one board: its heads, their delegations, its contracts
    /// and the weights of those past their freeze.
    pub fn board_accounts(&self, board: &BoardRef) -> Result<BoardAccounts> {
        let heads = self.scan::<Head>(&[(offset::HEAD_BOARD, &board.address)])?;
        let delegations = self.scan::<Delegation>(&[(offset::DELEGATION_BOARD, &board.address)])?;
        let contracts = self.scan::<HeadsContract>(&[(offset::CONTRACT_BOARD, &board.address)])?;
        let weights = self.weights(&contracts)?;
        Ok(BoardAccounts {
            address: board.address,
            board: board.board.clone(),
            heads,
            delegations,
            contracts,
            weights,
        })
    }

    /// The weights accounts of `contracts` past their freeze (by address: they have no fixed
    /// size).
    fn weights(
        &self,
        contracts: &[(Pubkey, Box<HeadsContract>)],
    ) -> Result<Vec<(Pubkey, Weights)>> {
        let program = self.oracle.program;
        let keys: Vec<Pubkey> = contracts
            .iter()
            .filter(|(_, k)| k.status != HeadsContractStatus::Accumulating)
            .flat_map(|(key, k)| {
                (0..k.component_count).map(move |c| find_heads_weights_address(&program, key, c).0)
            })
            .collect();
        let mut out = Vec::new();
        for chunk in keys.chunks(100) {
            let found = self.ctx().rpc.accounts(chunk)?;
            for (key, account) in chunk.iter().zip(found) {
                if let Some(w) =
                    account.and_then(|a| hv::decode_weights(&program, key, &a.owner, &a.data))
                {
                    out.push((*key, w));
                }
            }
        }
        Ok(out)
    }

    /// The texts of sealed kind-2 objects by commitment (`sha256(text)`, UTF-8): heads' specs,
    /// a challenge's reason. Objects missing, unsealed or not matching are left out.
    pub fn kind2_texts(&self, wanted: &[(Pubkey, [u8; 32])]) -> Result<HashMap<[u8; 32], String>> {
        let mut out = HashMap::new();
        let keys: Vec<Pubkey> = wanted.iter().map(|(k, _)| *k).collect();
        for (chunk, wants) in keys.chunks(100).zip(wanted.chunks(100)) {
            let found = self.ctx().rpc.accounts(chunk)?;
            for ((_, hash), account) in wants.iter().zip(found) {
                if let Some(text) = account.and_then(|a| kind2_text(&self.oracle.program, &a, hash))
                {
                    out.insert(*hash, text);
                }
            }
        }
        Ok(out)
    }
}

/// The text of a sealed kind-2 object committing to `hash`: `sha256(bytes) == hash`, UTF-8.
pub fn kind2_text(program: &Pubkey, account: &Account, hash: &[u8; 32]) -> Option<String> {
    let d = &account.data;
    if account.owner != *program
        || d.len() < OBJECT_HEADER
        || d[..5] != *b"OEB\x01\x01"
        || d[38] != 2
        || d[39..71] != *hash
        || d[75] != 1
    {
        return None;
    }
    let total = usize::from(u16::from_le_bytes([d[71], d[72]]));
    let bytes = d.get(OBJECT_HEADER..OBJECT_HEADER + total)?;
    (hashv(&[bytes]).to_bytes() == *hash)
        .then(|| String::from_utf8(bytes.to_vec()).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ameba_oracle::state::heads::zeroed_box;

    fn packed<T: HeadsAccount>(value: &T) -> Vec<u8> {
        let mut data = vec![0u8; T::LEN];
        value.pack(&mut data).unwrap();
        data
    }

    fn at(data: &[u8], offset: usize) -> Pubkey {
        Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
    }

    /// The scans' offsets are where the program's own codec puts each key.
    #[test]
    fn offsets_are_the_layouts() {
        let (a, b, c) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let mut stake = zeroed_box::<HeadsStake>();
        stake.owner = a;
        assert_eq!(at(&packed(stake.as_ref()), 10), a);
        let mut head = zeroed_box::<Head>();
        head.board = a;
        assert_eq!(at(&packed(head.as_ref()), offset::HEAD_BOARD), a);
        let mut d = zeroed_box::<Delegation>();
        (d.head, d.owner, d.board) = (a, b, c);
        let data = packed(d.as_ref());
        assert_eq!(
            (
                at(&data, 10),
                at(&data, offset::DELEGATION_OWNER),
                at(&data, offset::DELEGATION_BOARD)
            ),
            (a, b, c)
        );
        let mut claim = zeroed_box::<HeadClaim>();
        (claim.head, claim.fetcher) = (a, b);
        let data = packed(claim.as_ref());
        assert_eq!(
            (
                at(&data, offset::CLAIM_HEAD),
                at(&data, offset::CLAIM_FETCHER)
            ),
            (a, b)
        );
        let mut ch = zeroed_box::<HeadChallenge>();
        (ch.target, ch.head, ch.challenger) = (a, b, c);
        let data = packed(ch.as_ref());
        assert_eq!(
            (
                at(&data, offset::CHALLENGE_TARGET),
                at(&data, offset::CHALLENGE_HEAD),
                at(&data, offset::CHALLENGE_CHALLENGER)
            ),
            (a, b, c)
        );
        let mut k = zeroed_box::<HeadsContract>();
        k.board = a;
        assert_eq!(at(&packed(k.as_ref()), offset::CONTRACT_BOARD), a);
    }

    #[test]
    fn products_are_the_three_boards() {
        assert_eq!(product(" ramx ").unwrap(), Product::Ramx);
        assert_eq!(product("BATX").unwrap(), Product::Batx);
        assert!(
            product("DRAMX")
                .unwrap_err()
                .message
                .contains("RAMX, NANDX or BATX")
        );
    }

    #[test]
    fn a_kind2_text_is_its_sealed_bytes() {
        let program = Pubkey::new_unique();
        let text = "Mouser DDR5. USD per chip";
        let hash = hashv(&[text.as_bytes()]).to_bytes();
        let mut data = b"OEB\x01\x01".to_vec();
        data.resize(OBJECT_HEADER, 0);
        data[38] = 2;
        data[39..71].copy_from_slice(&hash);
        data[71..73].copy_from_slice(&(text.len() as u16).to_le_bytes());
        data[73..75].copy_from_slice(&(text.len() as u16).to_le_bytes());
        data[75] = 1;
        data.extend_from_slice(text.as_bytes());
        let account = Account {
            lamports: 1,
            owner: program,
            data,
            executable: false,
        };
        assert_eq!(kind2_text(&program, &account, &hash).as_deref(), Some(text));
        assert_eq!(kind2_text(&program, &account, &[0; 32]), None);
        assert_eq!(kind2_text(&Pubkey::new_unique(), &account, &hash), None);
    }
}

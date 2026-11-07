//! The production (mainnet) artifact from an empty address, the way the oracle reboot takes it
//! (`/opt/ameba-dev/sources/oracle-reboot-plan-2026-11-07.md`, steps 4.12 to 4.15): loaded alone
//! at exact size, the controller's per-target PDA as its upgrade authority, its gate Active, the
//! council's Config in place. `target/deploy/ameba_oracle.so`, or `AMEBA_ORACLE_PRODUCTION_SO`;
//! `scripts/check.sh oracle` builds it before these tests.
//!
//! This is the proof that nothing needs what the artifact refuses. Only the controller's two
//! accounts are installed: every account of the oracle's own comes from an instruction sent to the
//! artifact, so a dependency on 110, 154 or any other refused byte fails here.
//!
//! - The setup: 0 (three council seats; the controller's PDA cannot sign), 128, 90 and 129, with
//!   a USDC-like mint that has a freeze authority, and the production signer rotation delay.
//! - Heads: the config, a RAMX board on the mainnet parameters
//!   (`futures/ops/fixtures/heads-mainnet-params.proposed.json`), every sentence sealed through
//!   30/14, a draft closed through 30/15, and RAMX Q4 26 and Q1 27 through selector 28.
//! - Collateral in and out: 9, 10, 11, 140.
//! - A contract's life, as `futures-ops` lists it (`FixedMixReference`, three origins on all
//!   twelve components): stake, heads, delegation, fills, a challenge ruled by the controller's
//!   council, the listing, updates in the settlement window, every component settled live, the
//!   contract settled at the shared crate's value, prizes, delegators' rewards, withdrawal and
//!   unstaking.
//! - The admin paths (2, 109, 129) and 79 on the quarter that never listed.
//! - Every byte the reboot drops is refused as unassigned, on the address it just booted.
//!
//! Timing below comes from the artifact's own (real) calendar and the board's parameters: the
//! `test-sbf` constants this test crate compiles with are the shortened ones, and apply to nothing
//! here.
#![cfg(all(
    feature = "test-sbf",
    feature = "heads",
    not(feature = "localnet-dev-authority"),
    not(feature = "localnet-governance")
))]

mod common;

use ameba_commodity_spec::{molds, Product};
use ameba_oracle::{
    constants::{
        MAX_SETTLEMENT_SIGNER_COUNT, PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
    },
    error::OracleError,
    governance_gate::PINNED_CONTROLLER_PROGRAM_ID,
    instruction::{
        self,
        heads::{self as hx, heads_claim_commitment, heads_reason_hash},
        InitializeSettlementSignerRegistryParams, OracleCarryForwardActionV1 as Action,
        RotateVaultAuthoritiesV2Params, TerminalCleanupParams,
    },
    state::{
        derive_settlement_signer_registry_pda, derive_settlement_signer_set_pda,
        find_config_address,
        heads::{
            find_head_address, find_head_challenge_address, find_head_claim_address,
            find_heads_amba_vault_address, find_heads_authority_address, find_heads_board_address,
            find_heads_config_address, find_heads_contract_address, find_heads_stake_address,
            heads_settlement_digest, Head, HeadAccessClass, HeadChallengeReason, HeadClaimKind,
            HeadStatus, HeadsAccount, HeadsBoard, HeadsChannel, HeadsConfig, HeadsContract,
            HeadsContractStatus, HeadsMixBasis, HeadsParams, HeadsSettlementTarget, HeadsStake,
            COMPONENT_LIVE, COMPONENT_SETTLED,
        },
        OracleEconomicParams,
    },
};
use ameba_oracle_heads::{channels, median, rewards, settle, time};
use ameba_oracle_interface::{find_period_address, PeriodStatus};
use borsh::BorshSerialize;
use common::{cfm::Council, production::*, *};
use governance_controller_v3::state::target_authority_pda;
use solana_program_test::BanksClientError;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};
use spl_token_2022::extension::ExtensionType;

const AMBA: u64 = 1_000_000_000;
const USDC: u64 = 1_000_000;
const DAY: u64 = 86_400;
/// AMBA's transfer fee.
const FEE_BPS: u16 = 25;
/// RAMX Q4 26: its period expires as the quarter ends.
const Q4_EXPIRY: u64 = JAN_01_2027;
/// RAMX Q1 27, which never lists.
const Q1_EXPIRY: u64 = APR_01_2027;
/// Three heads of independent origins on every component (`min_live_sources`).
const ORIGINS: usize = 3;

fn t22() -> Pubkey {
    spl_token_2022::id()
}

/// `futures/ops/fixtures/heads-mainnet-params.proposed.json`, the parameters the reboot
/// recreates; the rest is the spec's, as there.
fn mainnet_params() -> HeadsParams {
    let mut p = HeadsParams::spec_defaults();
    p.claim_bond = 5 * USDC;
    p.challenge_bond = 5 * USDC;
    p.min_delegators = 1;
    p.min_live_sources = ORIGINS as u16;
    p.min_settle_weight = 100_000 * AMBA;
    p.alpha_bps = 0;
    p.floor_post = 50_000 * AMBA;
    p.floor_activate = 100_000 * AMBA;
    p
}

#[derive(Clone, Copy)]
struct Pending {
    id: u32,
    seq: u32,
}

struct Oracle {
    h: Harness,
    council: Council,
    /// The USDC-like mint's freeze authority, which tag 129 pins.
    freeze_authority: Pubkey,
    mint: Pubkey,
    mint_authority: Keypair,
    product: [u8; 32],
    board: Pubkey,
    params: HeadsParams,
    now: u64,
}

impl Oracle {
    /// Sends with a full heap frame and compute limit, as clients of the big accounts do.
    async fn send(
        &mut self,
        ix: Instruction,
        signers: &[&Keypair],
    ) -> Result<(), BanksClientError> {
        self.h
            .send_many(
                vec![
                    ComputeBudgetInstruction::request_heap_frame(256 * 1024),
                    ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                    ix,
                ],
                signers,
            )
            .await
    }

    async fn warp(&mut self, to: u64) {
        assert!(to >= self.now);
        self.now = to;
        set_unix_timestamp(&self.h.ctx, to).await;
        self.h.advance_slot().await;
    }

    async fn read<T: HeadsAccount>(&mut self, key: Pubkey) -> Box<T> {
        let account = self.h.account(key).await.expect("account exists");
        assert_eq!(account.owner, program());
        T::unpack(&account.data).expect("valid heads account")
    }

    fn head_key(&self, id: u32) -> Pubkey {
        find_head_address(&program(), &self.board, id).0
    }

    async fn board(&mut self) -> Box<HeadsBoard> {
        let key = self.board;
        self.read(key).await
    }

    async fn head(&mut self, id: u32) -> Box<Head> {
        let key = self.head_key(id);
        self.read(key).await
    }

    async fn contract(&mut self, expiry: u64) -> Box<HeadsContract> {
        let period = find_period_address(&program(), &self.product, expiry).0;
        self.read(find_heads_contract_address(&program(), &period).0)
            .await
    }

    async fn stake_of(&mut self, owner: &Pubkey) -> Box<HeadsStake> {
        self.read(find_heads_stake_address(&program(), owner).0)
            .await
    }

    async fn usdc(&mut self, owner: &Pubkey) -> u64 {
        self.h.collateral(owner).await.available_balance
    }

    async fn amba_account(&mut self, owner: &Pubkey, amount: u64) -> Pubkey {
        let create =
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &self.h.ctx.payer.pubkey(),
                owner,
                &self.mint,
                &t22(),
            );
        self.h.send(create, &[]).await.unwrap();
        let ata = spl_associated_token_account::get_associated_token_address_with_program_id(
            owner,
            &self.mint,
            &t22(),
        );
        if amount > 0 {
            let mint = spl_token_2022::instruction::mint_to(
                &t22(),
                &self.mint,
                &ata,
                &self.mint_authority.pubkey(),
                &[],
                amount,
            )
            .unwrap();
            let authority = self.mint_authority.insecure_clone();
            self.h.send(mint, &[&authority]).await.unwrap();
        }
        ata
    }

    async fn amba_balance(&mut self, token_account: Pubkey) -> u64 {
        let account = self.h.account(token_account).await.unwrap();
        u64::from_le_bytes(account.data[64..72].try_into().unwrap())
    }

    /// A key with SOL, `amba` AMBA staked into the family (what arrives, the fee off) and `usdc`
    /// USDC in its `UserCollateral`, through 9 and 10.
    async fn user(&mut self, amba: u64, usdc: u64) -> Keypair {
        let key = Keypair::new();
        self.h.fund(&key.pubkey(), 5_000_000_000).await;
        if amba > 0 {
            let ata = self.amba_account(&key.pubkey(), amba).await;
            let ix = hx::stake(
                &program(),
                &key.pubkey(),
                &key.pubkey(),
                &ata,
                &self.mint,
                &t22(),
                amba,
            );
            self.send(ix, &[&key]).await.unwrap();
        }
        let token = self.h.create_ata(&key.pubkey()).await;
        self.h
            .send(
                instruction::init_user_collateral(&program(), &key.pubkey()),
                &[&key],
            )
            .await
            .unwrap();
        if usdc > 0 {
            self.h.mint_usdc(&token, usdc).await;
            let ix = instruction::deposit_collateral(
                &program(),
                &key.pubkey(),
                &token,
                &self.h.usdc_mint,
                usdc,
            );
            self.h.send(ix, &[&key]).await.unwrap();
        }
        key
    }

    /// Writes and seals a kind-2 evidence object through 30/14; returns its commitment and
    /// address.
    async fn object(&mut self, payer: &Keypair, bytes: &[u8]) -> ([u8; 32], Pubkey) {
        let hash = solana_program::hash::hashv(&[bytes]).to_bytes();
        for (index, chunk) in bytes.chunks(192).enumerate() {
            let ix = instruction::write_evidence(
                &program(),
                &payer.pubkey(),
                2,
                hash,
                bytes.len() as u16,
                (index * 192) as u16,
                chunk.to_vec(),
            );
            self.h.send(ix, &[payer]).await.unwrap();
        }
        (
            hash,
            instruction::find_evidence_object_address(&program(), &payer.pubkey(), 2, &hash),
        )
    }

    /// Posts a head of `component` (A's floor) and backs it (B, the majority) until it is Active.
    async fn active_head(&mut self, a: &Keypair, b: &Keypair, component: u8) -> u32 {
        let id = self.board().await.head_count;
        let spec = format!("head {id} on component {component}: a distributor's monthly table");
        let (hash, object) = self.object(a, spec.as_bytes()).await;
        let ix = hx::post_head(
            &program(),
            &a.pubkey(),
            &a.pubkey(),
            &self.product,
            id,
            component,
            HeadAccessClass::PublicPage,
            self.params.floor_post,
            &format!("source {id}"),
            hash,
            &object,
        )
        .unwrap();
        self.send(ix, &[a]).await.unwrap();
        let ix = hx::delegate(
            &program(),
            &b.pubkey(),
            &b.pubkey(),
            &self.product,
            id,
            self.params.floor_activate - self.params.floor_post + 10_000 * AMBA,
        );
        self.send(ix, &[b]).await.unwrap();
        assert_eq!(self.head(id).await.status, HeadStatus::Active);
        id
    }

    /// Commits a claim and reveals it in the next slot; returns where it is.
    async fn claim(
        &mut self,
        fetcher: &Keypair,
        id: u32,
        kind: HeadClaimKind,
        value: u64,
        origin: [u8; 32],
    ) -> Pending {
        let seq = self.head(id).await.claim_seq;
        let reveal = hx::RevealClaimParams {
            value,
            source_ts: self.now,
            origin_id: origin,
            lta: Default::default(),
            salt: [seq as u8 ^ 0x5a; 32],
            evidence_kind: 0,
            evidence_commitment: [0; 32],
        };
        let hash = heads_claim_commitment(&self.head_key(id), seq, &fetcher.pubkey(), &reveal);
        let ix = hx::commit_claim(
            &program(),
            &fetcher.pubkey(),
            &fetcher.pubkey(),
            &self.product,
            id,
            seq,
            kind,
            hash,
            self.params.claim_bond,
        );
        self.send(ix, &[fetcher]).await.unwrap();
        self.h.advance_slot().await;
        let ix = hx::reveal_claim(
            &program(),
            &fetcher.pubkey(),
            &self.product,
            id,
            seq,
            reveal,
            None,
        )
        .unwrap();
        self.send(ix, &[fetcher]).await.unwrap();
        Pending { id, seq }
    }

    async fn finalize(&mut self, p: Pending, fetcher: &Pubkey) {
        let ix = hx::finalize_claim(
            &program(),
            &self.h.ctx.payer.pubkey(),
            &self.product,
            p.id,
            p.seq,
            fetcher,
            time::month_id(self.now),
        );
        self.send(ix, &[]).await.unwrap();
    }

    /// The quarter's mixes from the molds, which its sealed terms document commits to.
    fn listing_mixes(expiry: u64) -> Vec<hx::HeadsListedMix> {
        molds::listing_mixes(Product::Ramx, expiry as i64)
            .into_iter()
            .map(|(mix_bps, basis)| hx::HeadsListedMix {
                mix_bps,
                mix_basis: HeadsMixBasis::from_byte(basis.code()).unwrap(),
            })
            .collect()
    }

    async fn create_quarter_period(&mut self, planned: u64, expiry: u64) {
        let oracle = self.h.oracle_authority.insecure_clone();
        let ix = hx::create_quarter_period(
            &program(),
            &self.h.ctx.payer.pubkey(),
            &oracle.pubkey(),
            &self.product,
            planned,
            expiry,
        )
        .unwrap();
        self.send(ix, &[&oracle]).await.unwrap();
    }
}

/// An instruction with `tag` and `payload`, no accounts of its own (the harness adds the governed
/// envelope): whatever the payload, a refused byte never looks at it.
fn probe(tag: u8, payload: Vec<u8>) -> Instruction {
    Instruction {
        program_id: program(),
        accounts: vec![],
        data: [vec![tag], payload].concat(),
    }
}

/// The empty address with what `RegisterTarget` and the controller leave there, and the setup:
/// 0, 128, 90 and 129.
async fn boot() -> Option<Oracle> {
    let elf = production_elf()?;
    let controller = PINNED_CONTROLLER_PROGRAM_ID;
    let mut h = start_deployed(&elf, target_authority_pda(&controller, &program()).0).await;
    let (config_key, _) = find_config_address(&program());
    assert!(h.account(config_key).await.is_none(), "an empty address");
    let council = h.launch_council().await;
    // USDC has a freeze authority; tag 129 pins it.
    let freeze_authority = Pubkey::new_unique();
    let usdc_authority = h.usdc_mint_authority.pubkey();
    h.usdc_mint = h
        .create_mint(&usdc_authority, 6, Some(&freeze_authority))
        .await;

    // Tag 0: the loader authority is the controller's PDA, so three seats authorize the admin.
    let payer = h.payer();
    let admin = h.admin.pubkey();
    let mint = h.usdc_mint;
    let seats = council.three();
    let signers: Vec<&Keypair> = council.seats[..3].iter().collect();
    let outsiders: [Keypair; 3] = core::array::from_fn(|_| Keypair::new());
    let wrong = instruction::initialize_config_with_council(
        &program(),
        &payer.pubkey(),
        &controller,
        &outsiders.each_ref().map(Signer::pubkey),
        admin,
        &mint,
    );
    assert_oracle_error(
        h.send(wrong, &outsiders.each_ref()).await,
        OracleError::InvalidUpgradeAuthority,
    );
    let init = instruction::initialize_config_with_council(
        &program(),
        &payer.pubkey(),
        &controller,
        &seats,
        admin,
        &mint,
    );
    h.send(init, &signers).await.unwrap();
    let config = h.config().await;
    assert!(config.paused);
    assert_eq!(
        (config.admin, config.oracle_authority, config.usdc_mint),
        (admin, admin, mint)
    );
    assert_eq!(config.bond_vault, h.bond_vault());
    assert_eq!(h.token_balance(h.bond_vault()).await, 0);

    // Tag 128 splits the oracle authority off the admin, before the signer registry exists.
    h.split_oracle_authority().await;
    let oracle = h.oracle_authority.insecure_clone();
    assert_eq!(h.config().await.oracle_authority, oracle.pubkey());

    // Tag 90: five signers, threshold 3, and the production rotation delay (the test crate's
    // constant is the short one). The set is never used by heads; 129 and 109 read it.
    let mut keys: Vec<Pubkey> = (0..5).map(|_| Pubkey::new_unique()).collect();
    keys.sort_unstable();
    let mut members = [Pubkey::default(); MAX_SETTLEMENT_SIGNER_COUNT];
    members[..5].copy_from_slice(&keys);
    let recovery = h.recovery_authority.insecure_clone();
    let admin_key = h.admin.insecure_clone();
    let ix = instruction::initialize_settlement_signer_registry(
        &program(),
        &admin,
        &oracle.pubkey(),
        &recovery.pubkey(),
        InitializeSettlementSignerRegistryParams {
            signer_set_version: 1,
            threshold: 3,
            signer_count: 5,
            signers: members,
            rotation_delay_slots: PRODUCTION_MIN_SETTLEMENT_SIGNER_ROTATION_DELAY_SLOTS,
            recovery_authority: recovery.pubkey(),
        },
    );
    h.send(ix, &[&admin_key, &oracle, &recovery]).await.unwrap();
    assert!(h
        .account(derive_settlement_signer_registry_pda(&program()).0)
        .await
        .is_some());
    let set = derive_settlement_signer_set_pda(&program(), 1).0;

    // Tag 129: the only unpause, against the real custody.
    let ix = instruction::activate_vault_v2(&program(), &admin, &set, &mint, None);
    assert_oracle_error(h.send(ix, &[&admin_key]).await, OracleError::InvalidMint);
    let ix =
        instruction::activate_vault_v2(&program(), &admin, &set, &mint, Some(freeze_authority));
    h.send(ix, &[&admin_key]).await.unwrap();
    assert!(!h.config().await.paused);

    // AMBA: a Token-2022 mint with its 25 bp transfer fee.
    let mint_authority = Keypair::new();
    let amba = Keypair::new();
    let space = ExtensionType::try_calculate_account_len::<spl_token_2022::state::Mint>(&[
        ExtensionType::TransferFeeConfig,
    ])
    .unwrap();
    let rent = h.ctx.banks_client.get_rent().await.unwrap();
    let ixs = vec![
        solana_system_interface::instruction::create_account(
            &h.ctx.payer.pubkey(),
            &amba.pubkey(),
            rent.minimum_balance(space),
            space as u64,
            &t22(),
        ),
        spl_token_2022::extension::transfer_fee::instruction::initialize_transfer_fee_config(
            &t22(),
            &amba.pubkey(),
            Some(&mint_authority.pubkey()),
            Some(&mint_authority.pubkey()),
            FEE_BPS,
            u64::MAX,
        )
        .unwrap(),
        spl_token_2022::instruction::initialize_mint2(
            &t22(),
            &amba.pubkey(),
            &mint_authority.pubkey(),
            None,
            9,
        )
        .unwrap(),
    ];
    h.send_many(ixs, &[&amba]).await.unwrap();
    let product_id = product("RAMX");
    Some(Oracle {
        h,
        council,
        freeze_authority,
        mint: amba.pubkey(),
        mint_authority,
        product: product_id,
        board: find_heads_board_address(&program(), &product_id).0,
        params: mainnet_params(),
        now: NOW,
    })
}

/// The heads config and the RAMX board from the molds, every sentence sealed.
async fn heads_setup(oracle: &mut Oracle) {
    let admin = oracle.h.admin.insecure_clone();
    let payer = oracle.h.payer();
    let authority = find_heads_authority_address(&program()).0;
    let vault = oracle.amba_account(&authority, 0).await;
    assert_eq!(
        vault,
        find_heads_amba_vault_address(&program(), &oracle.mint, &t22())
    );
    let ix = hx::init_heads_config(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        &oracle.mint,
        &t22(),
    );
    oracle.send(ix, &[&admin]).await.unwrap();

    let mold = Product::Ramx.mold();
    let items = mold
        .items
        .iter()
        .map(|item| hx::HeadsItemParams {
            bucket_id: item.bucket.id32(),
            basket_weight_bps: item.basket_weight_bps,
            mix_bps: item.mix_bps,
            mix_basis: HeadsMixBasis::Assumed,
        })
        .collect();
    let ix = hx::init_board(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        oracle.product,
        oracle.params,
        items,
    )
    .unwrap();
    oracle.send(ix, &[&admin]).await.unwrap();

    // 30/15: a draft nobody finishes closes back to its payer.
    let draft = b"a sentence the admin abandoned";
    let hash = solana_program::hash::hashv(&[draft]).to_bytes();
    let ix = instruction::write_evidence(
        &program(),
        &admin.pubkey(),
        2,
        hash,
        draft.len() as u16,
        0,
        draft[..8].to_vec(),
    );
    oracle.h.send(ix, &[&admin]).await.unwrap();
    let object = instruction::find_evidence_object_address(&program(), &admin.pubkey(), 2, &hash);
    assert!(oracle.h.account(object).await.is_some());
    let ix = instruction::close_evidence_draft(&program(), &admin.pubkey(), &object);
    oracle.h.send(ix, &[&admin]).await.unwrap();
    assert!(oracle.h.account(object).await.is_none());

    // 30/14 seals each sentence; SetComponent binds it to the board.
    for c in mold.components() {
        let (hash, object) = oracle.object(&admin, c.sentence.as_bytes()).await;
        let ix = hx::set_component(
            &program(),
            &admin.pubkey(),
            &oracle.product,
            c.index,
            c.item,
            HeadsChannel::from_byte(c.channel.code()).unwrap(),
            hash,
            &object,
        )
        .unwrap();
        oracle.send(ix, &[&admin]).await.unwrap();
    }
    let board = oracle.board().await;
    for c in mold.components() {
        assert_eq!(
            board.component_sentence_hashes[usize::from(c.index)],
            c.sentence_sha256
        );
    }
    let config: Box<HeadsConfig> = oracle.read(find_heads_config_address(&program()).0).await;
    assert_eq!((config.amba_mint, config.amba_decimals), (oracle.mint, 9));
}

#[tokio::test]
async fn a_fresh_address_boots_and_settles_a_contract_on_the_production_artifact() {
    let Some(mut oracle) = boot().await else {
        return;
    };
    heads_setup(&mut oracle).await;
    let usdc_mint = oracle.h.usdc_mint;
    let admin = oracle.h.admin.insecure_clone();
    let payer = oracle.h.payer();

    // Collateral in and out: 9 and 10 (`user`), 11, then 140 with the admin.
    let holder = oracle.user(0, 100 * USDC).await;
    let token =
        spl_associated_token_account::get_associated_token_address(&holder.pubkey(), &usdc_mint);
    let ix = instruction::withdraw_collateral(
        &program(),
        &holder.pubkey(),
        &token,
        &usdc_mint,
        40 * USDC,
    );
    oracle.h.send(ix, &[&holder]).await.unwrap();
    let ix = instruction::admin_assisted_withdraw_collateral(
        &program(),
        &admin.pubkey(),
        &holder.pubkey(),
        &token,
        &usdc_mint,
        10 * USDC,
    );
    oracle.h.send(ix, &[&admin, &holder]).await.unwrap();
    assert_eq!(oracle.h.token_balance(token).await, 50 * USDC);
    assert_eq!(oracle.usdc(&holder.pubkey()).await, 50 * USDC);

    // Stake, heads and delegation: three heads of independent origins on all twelve components.
    let a = oracle.user(2_000_000 * AMBA, 0).await;
    let b = oracle.user(3_000_000 * AMBA, 0).await;
    let filler = oracle.user(0, 1_000 * USDC).await;
    let updater = oracle.user(0, 1_000 * USDC).await;
    let challenger = oracle.user(0, 100 * USDC).await;
    let funder = oracle.user(0, 2_000 * USDC).await;
    let mold = Product::Ramx.mold();
    let components = 3 * mold.items.len();
    let mut heads: Vec<Vec<u32>> = Vec::new();
    for component in 0..components {
        let mut ids = Vec::new();
        for _ in 0..ORIGINS {
            ids.push(oracle.active_head(&a, &b, component as u8).await);
        }
        heads.push(ids);
    }

    // Fills, the first one challenged and ruled by three seats of the controller's Config.
    let opening = |c: usize, j: usize| 1_000_000 * (c as u64 + 2) + 10_000 * j as u64;
    let publisher =
        |c: usize, j: usize| molds::origin_id(&format!("publisher-{c}-{j}"), "monthly table");
    let mut fills = Vec::new();
    for (c, ids) in heads.iter().enumerate() {
        for (j, &id) in ids.iter().enumerate() {
            let p = oracle
                .claim(
                    &filler,
                    id,
                    HeadClaimKind::Fill,
                    opening(c, j),
                    publisher(c, j),
                )
                .await;
            fills.push(p);
        }
    }
    let disputed = fills[0];
    let ix = hx::challenge_claim(
        &program(),
        &challenger.pubkey(),
        &challenger.pubkey(),
        &oracle.product,
        disputed.id,
        disputed.seq,
        hx::ChallengeClaimParams {
            bond: oracle.params.challenge_bond,
            reason: HeadChallengeReason::ValueOrOther,
            alternative_value: 0,
            alternative_source_ts: 0,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            reason_hash: heads_reason_hash(b"the table shows another number"),
        },
        None,
    )
    .unwrap();
    oracle.send(ix, &[&challenger]).await.unwrap();
    oracle.warp(oracle.now + DAY).await;
    let ix = hx::rule_claim_challenge(
        &program(),
        &oracle.council.config,
        &oracle.council.three(),
        &oracle.product,
        disputed.id,
        disputed.seq,
        &challenger.pubkey(),
        false,
        1,
    );
    let seats: Vec<Keypair> = oracle.council.seats[..3]
        .iter()
        .map(Keypair::insecure_clone)
        .collect();
    oracle
        .send(ix, &seats.iter().collect::<Vec<_>>())
        .await
        .unwrap();
    let before = oracle.usdc(&filler.pubkey()).await;
    for &p in &fills {
        oracle.finalize(p, &filler.pubkey()).await;
    }
    // Every fill's bond is back, and the challenger's lost bond went to the filler.
    assert_eq!(
        oracle.usdc(&filler.pubkey()).await,
        before + fills.len() as u64 * oracle.params.claim_bond + oracle.params.challenge_bond
    );
    let ix = hx::settle_claim_challenge(
        &program(),
        &oracle.product,
        disputed.id,
        disputed.seq,
        &challenger.pubkey(),
        &challenger.pubkey(),
    );
    oracle.send(ix, &[]).await.unwrap();
    let claim = find_head_claim_address(&program(), &oracle.head_key(disputed.id), disputed.seq).0;
    assert!(oracle
        .h
        .account(find_head_challenge_address(&program(), &claim, &challenger.pubkey()).0)
        .await
        .is_none());
    let board = oracle.board().await;
    assert_eq!(usize::from(board.live_count), ORIGINS * components);
    assert!(board.component_live_origins[..components]
        .iter()
        .all(|&origins| usize::from(origins) == ORIGINS));

    // October's fills share a funded prize pool; the rest of the funding reaches the delegators.
    let october = time::month_id(oracle.now);
    let ix = hx::fund_board(
        &program(),
        &funder.pubkey(),
        &funder.pubkey(),
        &oracle.product,
        october,
        1_000 * USDC,
    );
    oracle.send(ix, &[&funder]).await.unwrap();

    // Selector 28: RAMX Q4 26 lists at once, Q1 27 is planned and never lists. Then the listing,
    // as `futures-ops heads list` sends it: the oracle authority, the fixed-mix target, the
    // quarter's mixes, the sealed terms document.
    let planned = oracle.now;
    oracle.create_quarter_period(planned, Q4_EXPIRY).await;
    oracle.create_quarter_period(Q4_EXPIRY, Q1_EXPIRY).await;
    let document = molds::listing_terms_document(Product::Ramx, Q4_EXPIRY as i64);
    let terms = oracle.object(&admin, &document).await;
    let base = 100 * USDC;
    let authority = oracle.h.oracle_authority.insecure_clone();
    let target = HeadsSettlementTarget::FixedMixReference;
    let ix = hx::list_contract(
        &program(),
        &payer.pubkey(),
        &authority.pubkey(),
        &oracle.product,
        Q4_EXPIRY,
        base,
        target,
        &Oracle::listing_mixes(Q4_EXPIRY),
        terms.0,
        &terms.1,
    )
    .unwrap();
    oracle.send(ix, &[&authority]).await.unwrap();
    let period = oracle.h.period(oracle.product, Q4_EXPIRY).await;
    assert_eq!(
        (period.status, period.effective_listing_ts),
        (PeriodStatus::Live, planned)
    );
    let contract = oracle.contract(Q4_EXPIRY).await;
    assert_eq!(contract.settlement_target, target);
    assert_eq!(usize::from(contract.listed_count), ORIGINS * components);
    assert_eq!(
        contract.terms_hash,
        molds::listing_terms_sha256(Product::Ramx, Q4_EXPIRY as i64)
    );

    // Updates inside the settlement window: each source moves by its own amount, in percent.
    let moves = |c: usize, j: usize| (c as i64 * 7 + j as i64 * 5) % 23 - 11;
    let level = |c: usize, j: usize| {
        (opening(c, j) as i64 + opening(c, j) as i64 * moves(c, j) / 100) as u64
    };
    let freeze = time::settlement_window_start(Q4_EXPIRY, oracle.params.settlement_business_days);
    assert_eq!(contract.freeze_ts, freeze);
    oracle.warp(freeze + DAY).await;
    let mut updates = Vec::new();
    for (c, ids) in heads.iter().enumerate() {
        for (j, &id) in ids.iter().enumerate() {
            updates.push(
                oracle
                    .claim(
                        &updater,
                        id,
                        HeadClaimKind::Update,
                        level(c, j),
                        publisher(c, j),
                    )
                    .await,
            );
        }
    }
    oracle.warp(oracle.now + DAY).await;
    for &p in &updates {
        oracle.finalize(p, &updater.pubkey()).await;
    }

    // SettleComponent on every component, at the cutoff, then the contract: no signers, no month.
    oracle
        .warp(Q4_EXPIRY + oracle.params.challenge_window_secs + 1)
        .await;
    for (component, ids) in heads.iter().enumerate() {
        let ix = hx::settle_component(
            &program(),
            &payer.pubkey(),
            &oracle.product,
            Q4_EXPIRY,
            component as u8,
            0,
            ids,
        )
        .unwrap();
        oracle.send(ix, &[]).await.unwrap();
    }
    let ix = hx::finalize_contract(&program(), &oracle.product, Q4_EXPIRY);
    oracle.send(ix, &[]).await.unwrap();

    // The value is the shared crate's: every component's weighted median (equal weights), each
    // item's channels weighted by the listed mix, the basket over items.
    let contract = oracle.contract(Q4_EXPIRY).await;
    let mixes = molds::listing_mixes(Product::Ramx, Q4_EXPIRY as i64);
    let mut expected_items = Vec::new();
    for (item, mold_item) in mold.items.iter().enumerate() {
        let mut readings = [channels::ChannelReading::default(); 3];
        for (channel, reading) in readings.iter_mut().enumerate() {
            let c = item * 3 + channel;
            let mut pairs: Vec<(i64, u64)> = (0..ORIGINS)
                .map(|j| (settle::change_ppm(opening(c, j), level(c, j)).unwrap(), 1))
                .collect();
            let change = median::weighted_median(&mut pairs).unwrap();
            assert_eq!(
                contract.component_flags[c] & (COMPONENT_SETTLED | COMPONENT_LIVE),
                COMPONENT_SETTLED | COMPONENT_LIVE,
                "component {c}"
            );
            assert_eq!(contract.component_change_ppm[c], change, "component {c}");
            *reading = channels::ChannelReading {
                change_ppm: change,
                degraded: false,
            };
        }
        expected_items.push(channels::ItemSettlement {
            basket_weight_bps: mold_item.basket_weight_bps,
            contract_change_ppm: readings[1].change_ppm,
            reference: channels::reference(&mixes[item].0, channels::MixBasis::Assumed, &readings),
        });
    }
    let index = channels::stage2_change_ppm(&expected_items).unwrap();
    let value = settle::settlement_value(base, index).unwrap();
    assert_eq!(contract.status, HeadsContractStatus::Settled);
    assert_eq!((contract.index_change_ppm, contract.value), (index, value));
    assert_eq!(contract.digest, heads_settlement_digest(&contract));
    let settled = oracle.h.period(oracle.product, Q4_EXPIRY).await;
    assert_eq!(settled.status, PeriodStatus::Settled);
    assert_eq!(
        (settled.settlement_value, settled.settlement_digest),
        (value, contract.digest)
    );

    // Payouts: a fill's prize, a delegator's reward, and both leave as USDC through 11.
    let before = oracle.usdc(&filler.pubkey()).await;
    let first = fills[1];
    let ix = hx::claim_prize(
        &program(),
        &filler.pubkey(),
        &oracle.product,
        first.id,
        first.seq,
        october,
    );
    oracle.send(ix, &[&filler]).await.unwrap();
    let fill_prize = rewards::fill_prize(&oracle.params.to_params(), 110_000 * AMBA);
    let paid = oracle.usdc(&filler.pubkey()).await - before;
    assert!(paid > 0 && paid <= fill_prize, "{paid} of {fill_prize}");
    let ix = hx::claim_rewards(&program(), &b.pubkey(), &oracle.product, heads[0][0]);
    oracle.send(ix, &[&b]).await.unwrap();
    let reward = oracle.usdc(&b.pubkey()).await;
    assert!(reward > 0);
    let out = oracle.usdc(&filler.pubkey()).await;
    let filler_token =
        spl_associated_token_account::get_associated_token_address(&filler.pubkey(), &usdc_mint);
    let ix = instruction::withdraw_collateral(
        &program(),
        &filler.pubkey(),
        &filler_token,
        &usdc_mint,
        out,
    );
    oracle.h.send(ix, &[&filler]).await.unwrap();
    assert_eq!(oracle.h.token_balance(filler_token).await, out);
    assert_eq!(oracle.usdc(&filler.pubkey()).await, 0);

    // AMBA gets out too: what B never delegated, then a delegation after its cooldown.
    let b_ata = spl_associated_token_account::get_associated_token_address_with_program_id(
        &b.pubkey(),
        &oracle.mint,
        &t22(),
    );
    let free = oracle.stake_of(&b.pubkey()).await.free;
    assert!(free > 0);
    let ix = hx::unstake(&program(), &b.pubkey(), &b_ata, &oracle.mint, &t22(), free);
    oracle.send(ix, &[&b]).await.unwrap();
    assert!(oracle.amba_balance(b_ata).await > 0);
    let id = heads[0][0];
    let ix = hx::undelegate(&program(), &b.pubkey(), &oracle.product, id, 10_000 * AMBA);
    oracle.send(ix, &[&b]).await.unwrap();
    oracle.warp(oracle.now + oracle.params.cooldown_secs).await;
    let ix = hx::sync_delegation(&program(), &oracle.product, id, &b.pubkey(), &b.pubkey());
    oracle.send(ix, &[]).await.unwrap();
    assert_eq!(
        oracle.stake_of(&b.pubkey()).await.free,
        10_000 * AMBA,
        "the cooldown's end frees it"
    );

    // 79 fails the quarter nobody listed: the period has no month, and needs none.
    oracle.warp(Q1_EXPIRY).await;
    let ix = instruction::fail_period(&program(), &payer.pubkey(), &oracle.product, Q1_EXPIRY);
    oracle.h.send(ix, &[]).await.unwrap();
    assert_eq!(
        oracle.h.period(oracle.product, Q1_EXPIRY).await.status,
        PeriodStatus::Failed
    );

    // The admin paths: 2 pauses, 109 rotates while paused, and 129 (the new admin) unpauses.
    let vault = oracle.h.bond_vault();
    let ix = instruction::update_config(
        &program(),
        &admin.pubkey(),
        &usdc_mint,
        &vault,
        None,
        None,
        None,
        None,
        Some(true),
    );
    oracle.h.send(ix, &[&admin]).await.unwrap();
    assert!(oracle.h.config().await.paused);
    let (new_admin, new_oracle) = (Keypair::new(), Keypair::new());
    oracle.h.fund(&new_admin.pubkey(), 1_000_000_000).await;
    let set = derive_settlement_signer_set_pda(&program(), 1).0;
    let ix = instruction::rotate_vault_authorities_v2(
        &program(),
        &admin.pubkey(),
        &authority.pubkey(),
        RotateVaultAuthoritiesV2Params {
            new_admin: new_admin.pubkey(),
            new_oracle_authority: new_oracle.pubkey(),
        },
        &set,
    );
    oracle
        .h
        .send(ix, &[&admin, &authority, &new_admin, &new_oracle])
        .await
        .unwrap();
    let config = oracle.h.config().await;
    assert_eq!(
        (config.admin, config.oracle_authority),
        (new_admin.pubkey(), new_oracle.pubkey())
    );
    let ix = instruction::activate_vault_v2(
        &program(),
        &new_admin.pubkey(),
        &set,
        &usdc_mint,
        Some(oracle.freeze_authority),
    );
    oracle.h.send(ix, &[&new_admin]).await.unwrap();
    assert!(!oracle.h.config().await.paused);
}

/// Everything the reboot drops answers as an unassigned byte, on a booted oracle: 110 and 154
/// through the builders a client would use, the refunds and the terminal cleanup bare, with
/// payloads their handlers would decode.
#[tokio::test]
async fn the_dropped_bytes_are_refused_on_a_booted_oracle() {
    let Some(mut oracle) = boot().await else {
        return;
    };
    let admin = oracle.h.admin.insecure_clone();
    let payer = oracle.h.payer();
    let usdc_mint = oracle.h.usdc_mint;
    let economics = instruction::configure_oracle_economics_template_v2(
        &program(),
        &admin.pubkey(),
        instruction::ConfigureOracleEconomicsTemplateV2Params {
            expected_config_version: 0,
            economics: OracleEconomicParams::default(),
        },
    );
    assert_eq!(economics.data[0], 110);
    assert_oracle_error(
        oracle.h.send(economics, &[&admin]).await,
        OracleError::InvalidInstructionData,
    );
    let vault =
        instruction::initialize_oracle_usdc_reward_vault(&program(), &payer.pubkey(), &usdc_mint);
    assert_eq!(vault.data[0], 154);
    assert_oracle_error(
        oracle.h.send(vault, &[]).await,
        OracleError::InvalidInstructionData,
    );
    let cleanup = Action::TerminalCleanup(TerminalCleanupParams {
        kind: 0,
        bucket_id: [0; 32],
        expected_data_hash: [0; 32],
        expected_lamports: 0,
    });
    for (label, ix) in [
        ("167", probe(167, vec![])),
        // SettleOracleUsdcEscrow: kind 5, an update claim.
        ("169", probe(169, vec![5])),
        ("191", probe(191, vec![])),
        // SettleFailedOracleMonthEscrowV2: kind 0, a listing bond.
        ("194", probe(194, vec![0])),
        ("195", probe(195, vec![])),
        ("30/24", probe(30, cleanup.try_to_vec().unwrap())),
    ] {
        assert_eq!(
            custom(oracle.h.send(ix, &[]).await),
            OracleError::InvalidInstructionData as u32,
            "{label}"
        );
    }
    // The setup is create-once, on this artifact too.
    let controller = PINNED_CONTROLLER_PROGRAM_ID;
    let seats = oracle.council.three();
    let init = instruction::initialize_config_with_council(
        &program(),
        &payer.pubkey(),
        &controller,
        &seats,
        admin.pubkey(),
        &usdc_mint,
    );
    let signers: Vec<Keypair> = oracle.council.seats[..3]
        .iter()
        .map(Keypair::insecure_clone)
        .collect();
    assert_oracle_error(
        oracle
            .h
            .send(init, &signers.iter().collect::<Vec<_>>())
            .await,
        OracleError::AlreadyInitialized,
    );
    let replay = Keypair::new();
    let ix =
        instruction::bootstrap_vault_governance_v2(&program(), &admin.pubkey(), &replay.pubkey());
    assert_oracle_error(
        oracle.h.send(ix, &[&admin, &replay]).await,
        OracleError::SettlementSignerGovernanceRequired,
    );
}

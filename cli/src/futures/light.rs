//! Compressed USDC: the futures API spends a wallet's Light tokens (left by
//! Amoeba's earlier product) by decompressing them into a classic USDC
//! account with the compressed token program's `Transfer2` (API.md,
//! "Spending compressed USDC"; built by the API's `light.rs`). The owner
//! signs those steps, so petri reads each one in full and accepts only the
//! two shapes the API builds, paid into the account `destination`:
//!
//! * leaves: the owner's compressed accounts in, one decompression out to
//!   `destination`, and at most one change account back to the owner;
//! * the Light ATA: a compression out of the owner's Light token ATA and the
//!   same amount decompressed to `destination`.
//!
//! The program itself enforces that the inputs exist and that amounts
//! balance; this check makes sure nothing can leave the owner.

use solana_program::{instruction::CompiledInstruction, pubkey, pubkey::Pubkey};

use super::error::{Result, bail};

/// The compressed token program, which also owns Light token ATAs.
pub const PROGRAM: Pubkey = pubkey!("cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m");
const LIGHT_SYSTEM_PROGRAM: Pubkey = pubkey!("SySTEM1eSU2p4BGQfQpimFEWWSC1XDFeun3Nqzz3rT7");
const CPI_AUTHORITY: Pubkey = pubkey!("GXtd2izAiMJPwMEjfgTRH3d7k9mjn4Jq3JrWFv9gySYy");
const REGISTERED_PROGRAM: Pubkey = pubkey!("35hkDgaAKwMCaxRz2ocSZ6NaUrtKkyNqU6c4RV3tYJRh");
const COMPRESSION_AUTHORITY: Pubkey = pubkey!("HwXnGK3tPkkVY6P439H2p68AxpeuWXd5PcrAxFpbmfbA");
const COMPRESSION_PROGRAM: Pubkey = pubkey!("compr6CUsB5m2jS4Y3831ztGSTnDpnKJTKS95d64XVq");
const TRANSFER2: u8 = 101;
const COMPRESS: u8 = 0;
const DECOMPRESS: u8 = 1;
/// The program's limit on inputs per `Transfer2`.
const MAX_INPUTS: usize = 8;

/// The owner's Light token ATA of `mint`.
pub fn light_ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[owner.as_ref(), PROGRAM.as_ref(), mint.as_ref()], &PROGRAM).0
}

/// The mint's SPL pool (index 0) and its bump.
fn spl_pool(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"pool", mint.as_ref()], &PROGRAM)
}

/// Who and what a step may name.
pub struct Expect<'a> {
    pub owner: &'a Pubkey,
    pub sponsor: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub decimals: u8,
    /// The classic USDC account the step must pay into.
    pub destination: &'a Pubkey,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let out = self.data.get(self.at..self.at + n)?;
        self.at += n;
        Some(out)
    }
    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.bytes(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn u64(&mut self) -> Option<u64> {
        self.bytes(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
}

/// One `Compression`: mode, amount, mint, account, authority, pool account,
/// pool index, bump, decimals.
struct Compression {
    mode: u8,
    amount: u64,
    mint: u8,
    account: u8,
    authority: u8,
    pool: u8,
    pool_index: u8,
    bump: u8,
    decimals: u8,
}

fn compression(r: &mut Reader) -> Option<Compression> {
    Some(Compression {
        mode: r.u8()?,
        amount: r.u64()?,
        mint: r.u8()?,
        account: r.u8()?,
        authority: r.u8()?,
        pool: r.u8()?,
        pool_index: r.u8()?,
        bump: r.u8()?,
        decimals: r.u8()?,
    })
}

/// Checks one step of the compressed token program (`keys`: the message's
/// accounts) and returns the USDC atoms it pays into `e.destination`.
pub fn check(keys: &[Pubkey], ci: &CompiledInstruction, i: usize, e: &Expect) -> Result<u64> {
    let accounts: Option<Vec<Pubkey>> = ci
        .accounts
        .iter()
        .map(|a| keys.get(usize::from(*a)).copied())
        .collect();
    let refuse = || {
        bail(format!(
            "refusing: step {i} is not a decompression of your own USDC"
        ))
    };
    let Some(accounts) = accounts else {
        return refuse();
    };
    match read(&accounts, &ci.data, e) {
        Some(amount) => Ok(amount),
        None => refuse(),
    }
}

fn read(accounts: &[Pubkey], data: &[u8], e: &Expect) -> Option<u64> {
    let mut r = Reader { data, at: 0 };
    // Head: no transaction hash, no lamport change, the output queue, no
    // top-up cap, no CPI context.
    if r.u8()? != TRANSFER2 || r.bytes(4)? != [0, 0, 0, 0] {
        return None;
    }
    let output_queue = r.u8()?;
    if r.bytes(3)? != [0xff, 0xff, 0] || r.u8()? != 1 {
        return None;
    }
    let (pool, bump) = spl_pool(e.mint);
    let amount = match r.u32()? {
        1 => leaves(accounts, &mut r, output_queue, e, &pool, bump)?,
        2 => from_light_ata(accounts, &mut r, output_queue, e, &pool, bump)?,
        _ => return None,
    };
    // No lamports and no extensions, and nothing after them.
    (r.bytes(4)? == [0, 0, 0, 0] && r.at == data.len() && amount > 0).then_some(amount)
}

/// The decompression into the destination, which both shapes end with.
fn pays_destination(
    c: &Compression,
    packed: &[Pubkey],
    e: &Expect,
    pool: &Pubkey,
    bump: u8,
) -> bool {
    let at = |n: u8| packed.get(usize::from(n));
    c.mode == DECOMPRESS
        && at(c.mint) == Some(e.mint)
        && at(c.account) == Some(e.destination)
        && c.authority == 0
        && at(c.pool) == Some(pool)
        && c.pool_index == 0
        && c.bump == bump
        && c.decimals == e.decimals
}

fn leaves(
    accounts: &[Pubkey],
    r: &mut Reader,
    output_queue: u8,
    e: &Expect,
    pool: &Pubkey,
    bump: u8,
) -> Option<u64> {
    let fixed = [
        LIGHT_SYSTEM_PROGRAM,
        *e.sponsor,
        CPI_AUTHORITY,
        REGISTERED_PROGRAM,
        COMPRESSION_AUTHORITY,
        COMPRESSION_PROGRAM,
        solana_program::system_program::id(),
    ];
    if accounts.get(..7)? != fixed {
        return None;
    }
    let packed = &accounts[7..];
    let at = |n: u8| packed.get(usize::from(n));
    at(output_queue)?;
    let c = compression(r)?;
    if !pays_destination(&c, packed, e, pool, bump) {
        return None;
    }
    match r.u8()? {
        0 => {}
        1 => {
            r.bytes(128)?;
        }
        _ => return None,
    }
    let inputs = r.u32()? as usize;
    if inputs == 0 || inputs > MAX_INPUTS {
        return None;
    }
    let mut total = 0u64;
    for _ in 0..inputs {
        // owner, amount, no delegate, mint, version; tree, queue, leaf
        // index, prove by index, root index.
        let owner = r.u8()?;
        let amount = r.u64()?;
        let (has_delegate, delegate, mint, version) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let (tree, queue) = (r.u8()?, r.u8()?);
        r.u32()?;
        let by_index = r.u8()?;
        r.bytes(2)?;
        if at(owner) != Some(e.owner)
            || has_delegate != 0
            || delegate != 0
            || mint != c.mint
            || !(1..=3).contains(&version)
            || at(tree).is_none()
            || at(queue).is_none()
            || by_index > 1
        {
            return None;
        }
        total = total.checked_add(amount)?;
    }
    let change = match r.u32()? {
        0 => 0,
        1 => {
            let owner = r.u8()?;
            let amount = r.u64()?;
            let (has_delegate, delegate, mint, version) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            if at(owner) != Some(e.owner)
                || has_delegate != 0
                || delegate != 0
                || mint != c.mint
                || !(1..=3).contains(&version)
            {
                return None;
            }
            amount
        }
        _ => return None,
    };
    // What goes in is what comes out: the decompression and the owner's change.
    (c.amount.checked_add(change)? == total).then_some(c.amount)
}

fn from_light_ata(
    accounts: &[Pubkey],
    r: &mut Reader,
    output_queue: u8,
    e: &Expect,
    pool: &Pubkey,
    bump: u8,
) -> Option<u64> {
    if accounts.get(..2)? != [CPI_AUTHORITY, *e.sponsor] || output_queue != 0 {
        return None;
    }
    let packed = &accounts[2..];
    let at = |n: u8| packed.get(usize::from(n));
    let out = compression(r)?;
    let into = compression(r)?;
    let ok = out.mode == COMPRESS
        && at(out.mint) == Some(e.mint)
        && at(out.account) == Some(&light_ata(e.owner, e.mint))
        && at(out.authority) == Some(e.owner)
        && (out.pool, out.pool_index, out.bump, out.decimals) == (0, 0, 0, 0)
        && pays_destination(&into, packed, e, pool, bump)
        && into.mint == out.mint
        && into.amount == out.amount;
    // No proof, no inputs, no outputs.
    (ok && r.bytes(9)? == [0, 0, 0, 0, 0, 0, 0, 0, 0]).then_some(out.amount)
}

/// A leaf decompression as the API builds it (its own test vector: leaves of
/// 10 and 5 atoms, 12 paid to `destination`, 3 back to `owner`), for tests.
#[cfg(test)]
pub fn api_step(
    sponsor: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    destination: &Pubkey,
) -> solana_program::instruction::Instruction {
    use solana_program::instruction::{AccountMeta, Instruction};
    let (pool, bump) = spl_pool(mint);
    let mut accounts = vec![
        AccountMeta::new_readonly(LIGHT_SYSTEM_PROGRAM, false),
        AccountMeta::new(*sponsor, true),
        AccountMeta::new_readonly(CPI_AUTHORITY, false),
        AccountMeta::new_readonly(REGISTERED_PROGRAM, false),
        AccountMeta::new_readonly(COMPRESSION_AUTHORITY, false),
        AccountMeta::new_readonly(COMPRESSION_PROGRAM, false),
        AccountMeta::new_readonly(solana_program::system_program::id(), false),
    ];
    accounts.extend([
        AccountMeta::new(
            pubkey!("bmt1LryLZUMmF7ZtqESaw7wifBXLfXHQYoE4GAmrahU"),
            false,
        ),
        AccountMeta::new(
            pubkey!("oq1na8gojfdUhsfCpyjNt6h4JaDWtHf1yQj4koBWfto"),
            false,
        ),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new_readonly(*owner, true),
        AccountMeta::new(*destination, false),
        AccountMeta::new(pool, false),
        AccountMeta::new_readonly(spl_token::id(), false),
    ]);
    Instruction {
        program_id: PROGRAM,
        accounts,
        data: tests::api_vector(bump),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fx {
        sponsor: Pubkey,
        owner: Pubkey,
        mint: Pubkey,
        destination: Pubkey,
        tree: Pubkey,
        queue: Pubkey,
    }

    fn fx() -> Fx {
        Fx {
            sponsor: Pubkey::new_unique(),
            owner: Pubkey::new_unique(),
            mint: Pubkey::new_unique(),
            destination: Pubkey::new_unique(),
            tree: pubkey!("bmt1LryLZUMmF7ZtqESaw7wifBXLfXHQYoE4GAmrahU"),
            queue: pubkey!("oq1na8gojfdUhsfCpyjNt6h4JaDWtHf1yQj4koBWfto"),
        }
    }

    fn expect(f: &Fx) -> Expect<'_> {
        Expect {
            owner: &f.owner,
            sponsor: &f.sponsor,
            mint: &f.mint,
            decimals: 6,
            destination: &f.destination,
        }
    }

    /// The accounts of the API's leaf decompression: the fixed seven, then
    /// tree, queue, mint, owner, destination, pool, token program.
    fn leaf_accounts(f: &Fx) -> Vec<Pubkey> {
        vec![
            LIGHT_SYSTEM_PROGRAM,
            f.sponsor,
            CPI_AUTHORITY,
            REGISTERED_PROGRAM,
            COMPRESSION_AUTHORITY,
            COMPRESSION_PROGRAM,
            solana_program::system_program::id(),
            f.tree,
            f.queue,
            f.mint,
            f.owner,
            f.destination,
            spl_pool(&f.mint).0,
            spl_token::id(),
        ]
    }

    /// The API's own test vector (api/api/src/light.rs,
    /// `decompression_data_follows_the_programs_layout`): leaves of 10 and 5
    /// atoms, 12 decompressed, 3 back to the owner.
    pub(super) fn api_vector(bump: u8) -> Vec<u8> {
        let mut d = vec![101, 0, 0, 0, 0, 1, 0xff, 0xff, 0, 1, 1, 0, 0, 0];
        d.extend([1, 12, 0, 0, 0, 0, 0, 0, 0, 2, 4, 0, 5, 0, bump, 6]);
        d.extend([0, 2, 0, 0, 0]);
        d.extend([
            3, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 0, 1, 7, 0, 0, 0, 1, 0, 0,
        ]);
        d.extend([
            3, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 0, 1, 9, 0, 0, 0, 1, 0, 0,
        ]);
        d.extend([1, 0, 0, 0, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3]);
        d.extend([0, 0, 0, 0]);
        d
    }

    #[test]
    fn the_apis_leaf_decompression_into_your_account_is_accepted() {
        let f = fx();
        let bump = spl_pool(&f.mint).1;
        assert_eq!(
            read(&leaf_accounts(&f), &api_vector(bump), &expect(&f)),
            Some(12)
        );
        // With a ZK proof (Mainnet leaves need one) it reads the same.
        let mut zk = api_vector(bump);
        zk.splice(30..31, std::iter::once(1).chain([9u8; 128]));
        assert_eq!(read(&leaf_accounts(&f), &zk, &expect(&f)), Some(12));
    }

    #[test]
    fn anything_that_could_move_funds_away_is_refused() {
        let f = fx();
        let bump = spl_pool(&f.mint).1;
        let e = expect(&f);
        let other = Pubkey::new_unique();
        // Paid into someone else's account.
        let mut accounts = leaf_accounts(&f);
        accounts[11] = other;
        assert_eq!(read(&accounts, &api_vector(bump), &e), None);
        // Change to another owner: point the output's owner at the destination.
        let mut d = api_vector(bump);
        let out = d.len() - 4 - 13;
        d[out] = 4;
        assert_eq!(read(&leaf_accounts(&f), &d, &e), None);
        // An input that isn't the owner's.
        let mut d = api_vector(bump);
        d[35] = 4;
        assert_eq!(read(&leaf_accounts(&f), &d, &e), None);
        // Amounts that don't balance (more out than in).
        let mut d = api_vector(bump);
        d[15] = 13;
        assert_eq!(read(&leaf_accounts(&f), &d, &e), None);
        // Another fee payer, a delegate, another mint, the wrong pool bump,
        // or trailing bytes.
        let mut accounts = leaf_accounts(&f);
        accounts[1] = other;
        assert_eq!(read(&accounts, &api_vector(bump), &e), None);
        let mut d = api_vector(bump);
        d[44] = 1;
        assert_eq!(read(&leaf_accounts(&f), &d, &e), None);
        let mut accounts = leaf_accounts(&f);
        accounts[9] = other;
        assert_eq!(read(&accounts, &api_vector(bump), &e), None);
        assert_eq!(
            read(&leaf_accounts(&f), &api_vector(bump.wrapping_add(1)), &e),
            None
        );
        let mut d = api_vector(bump);
        d.push(0);
        assert_eq!(read(&leaf_accounts(&f), &d, &e), None);
    }

    /// The API's Light ATA step: accounts CPI authority, sponsor, then mint,
    /// Light ATA, destination, owner, pool, token program.
    fn light_ata_step(f: &Fx, source: &Pubkey, amount: u64) -> (Vec<Pubkey>, Vec<u8>) {
        let (pool, bump) = spl_pool(&f.mint);
        let accounts = vec![
            CPI_AUTHORITY,
            f.sponsor,
            f.mint,
            *source,
            f.destination,
            f.owner,
            pool,
            spl_token::id(),
        ];
        let a = amount.to_le_bytes();
        let mut d = vec![101, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 1, 2, 0, 0, 0];
        d.push(COMPRESS);
        d.extend(a);
        d.extend([0, 1, 3, 0, 0, 0, 0]);
        d.push(DECOMPRESS);
        d.extend(a);
        d.extend([0, 2, 0, 4, 0, bump, 6]);
        d.extend([0, 0, 0, 0, 0, 0, 0, 0, 0]);
        d.extend([0, 0, 0, 0]);
        (accounts, d)
    }

    #[test]
    fn the_light_ata_step_must_take_from_your_light_ata() {
        let f = fx();
        let e = expect(&f);
        let (accounts, d) = light_ata_step(&f, &light_ata(&f.owner, &f.mint), 3);
        assert_eq!(read(&accounts, &d, &e), Some(3));
        let (accounts, d) = light_ata_step(&f, &Pubkey::new_unique(), 3);
        assert_eq!(read(&accounts, &d, &e), None);
        // Uneven amounts.
        let (accounts, mut d) = light_ata_step(&f, &light_ata(&f.owner, &f.mint), 3);
        d[31] = 4;
        assert_eq!(read(&accounts, &d, &e), None);
    }
}

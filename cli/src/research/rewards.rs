//! The Rewards view's earnings: what your agents' finds paid them once a
//! sponsor put them on the oracle. Each sponsorship agreement pays a reserve
//! that is the agent's (its id and its wallet), so they are read by that
//! reserve. Reads run on a worker thread, as the Fund view's do.

use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use ameba_oracle::sponsorship::{reserve_address, split_bounty};
use solana_program::pubkey::Pubkey;

use super::{model::Agent, render::month_name};
use crate::{
    futures::{ctx::Ctx, settings::Settings},
    oracle::{chain::Oracle, skus, sponsor},
};

/// A find of the agent's that a sponsor put on the oracle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Earning {
    /// "RAMX · Dec 2026 · DDR5 16Gb".
    pub title: String,
    /// "RAMX-202612": the oracle month its reward is for.
    pub expiry: String,
    /// "waiting", "live, reward paid", … (`sponsor::outcome`).
    pub outcome: String,
    /// USDC atoms the researcher got: its cash and what its reserve kept.
    pub earned: u64,
}

/// An agent: its id and the wallet its rewards go to.
type Key = (String, String);
/// Each agent's earnings, by its id.
type ByAgent = HashMap<String, Vec<Earning>>;

#[derive(Default)]
pub struct Rewards {
    /// By agent id, once read.
    pub by_agent: ByAgent,
    /// The last read failed: what is shown is the read before it.
    pub offline: bool,
    busy: bool,
    last_refresh: Option<Instant>,
    /// The agents the last read asked for.
    asked: Vec<String>,
    commands: Option<Sender<Vec<Key>>>,
    replies: Option<Receiver<Result<ByAgent, String>>>,
}

impl Rewards {
    pub fn start(&mut self, settings: Settings) {
        let (command_tx, command_rx) = mpsc::channel::<Vec<Key>>();
        let (reply_tx, reply_rx) = mpsc::channel();
        thread::spawn(move || {
            let ctx = Ctx::new(settings);
            // Months keep their names: read each once.
            let mut months = HashMap::new();
            while let Ok(agents) = command_rx.recv() {
                if reply_tx.send(fetch(&ctx, &agents, &mut months)).is_err() {
                    break;
                }
            }
        });
        self.commands = Some(command_tx);
        self.replies = Some(reply_rx);
    }

    /// Takes finished reads; reads again every 10 s while `active`, and every
    /// 30 s otherwise, so the view opens with its data.
    pub fn tick(&mut self, active: bool, agents: &[Agent]) {
        while let Some(reply) = self.replies.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.busy = false;
            match reply {
                Ok(by_agent) => {
                    self.by_agent = by_agent;
                    self.offline = false;
                }
                Err(_) => self.offline = true,
            }
        }
        let every = Duration::from_secs(if active { 10 } else { 30 });
        let due = self.last_refresh.is_none_or(|t| t.elapsed() >= every)
            // Agents connected since the last read are read at once.
            || agents.iter().any(|a| !self.asked.contains(&a.id));
        if !self.busy
            && due
            && !agents.is_empty()
            && let Some(tx) = &self.commands
        {
            let keys = agents
                .iter()
                .map(|a| (a.id.clone(), a.payout_authority.clone()))
                .collect();
            if tx.send(keys).is_ok() {
                self.busy = true;
                self.last_refresh = Some(Instant::now());
                self.asked = agents.iter().map(|a| a.id.clone()).collect();
            }
        }
    }
}

/// The researcher's part of a paid bounty: its cash and its reserve's.
pub(crate) fn researcher_share(paid: u64, researcher_bps: u16, retention_bps: u16) -> u64 {
    split_bounty(paid, researcher_bps, retention_bps).map_or(0, |(_, cash, kept)| cash + kept)
}

fn fetch(
    ctx: &Ctx,
    agents: &[Key],
    months: &mut HashMap<Pubkey, (String, String)>,
) -> Result<ByAgent, String> {
    let oracle = Oracle::reads(ctx).map_err(|e| e.message)?;
    let mut out = HashMap::new();
    for (id, wallet) in agents {
        let (Ok(agent), Ok(wallet)) = (skus::parse_hex32(id, "agent"), wallet.parse::<Pubkey>())
        else {
            out.insert(id.clone(), Vec::new());
            continue;
        };
        let reserve = reserve_address(&oracle.program, &agent, &wallet).0;
        let mut earnings = Vec::new();
        for (_, a) in oracle
            .sponsorships_of_reserve(&reserve)
            .map_err(|e| e.message)?
        {
            let (symbol, expiry) = match months.get(&a.month) {
                Some(known) => known,
                None => {
                    let month = oracle.month_at(&a.month).map_err(|e| e.message)?;
                    months.insert(
                        a.month,
                        (month.period.symbol.clone(), month.period.expiry_id()),
                    );
                    &months[&a.month]
                }
            };
            let product = skus::id_name(&skus::hex(&a.bucket_id))
                .unwrap_or_else(|| format!("Unnamed product {}", &skus::hex(&a.bucket_id)[..6]));
            let month = month_name(expiry).unwrap_or_else(|| expiry.clone());
            earnings.push(Earning {
                title: format!("{} · {month} · {product}", symbol.to_uppercase()),
                expiry: expiry.clone(),
                outcome: sponsor::outcome(&a).to_string(),
                earned: if a.reward_settled {
                    researcher_share(a.reward_paid, a.researcher_bps, a.retention_bps)
                } else {
                    0
                },
            });
        }
        earnings.sort_by(|a, b| month_key(&a.expiry).cmp(month_key(&b.expiry)));
        out.insert(id.clone(), earnings);
    }
    Ok(out)
}

/// "RAMX-202612" sorts as "202612": months in order, whatever the market.
pub(crate) fn month_key(expiry: &str) -> &str {
    expiry.rsplit('-').next().unwrap_or(expiry)
}

/// Earnings per month, oldest first: ("Dec 2026", atoms). Months with
/// nothing paid yet are there too, at zero, so the chart shows them.
pub(crate) fn by_month(earnings: &[Earning]) -> Vec<(String, u64)> {
    let mut months: Vec<(String, String, u64)> = Vec::new();
    for e in earnings {
        let key = month_key(&e.expiry).to_string();
        match months.iter_mut().find(|(k, _, _)| *k == key) {
            Some(month) => month.2 += e.earned,
            None => months.push((
                key,
                month_name(&e.expiry).unwrap_or_else(|| e.expiry.clone()),
                e.earned,
            )),
        }
    }
    months.sort();
    months
        .into_iter()
        .map(|(_, name, atoms)| (name, atoms))
        .collect()
}

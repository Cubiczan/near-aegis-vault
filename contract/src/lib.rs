//! Aegis Vault — an on-chain spend-policy firewall for AI agents.
//!
//! The vault holds funds. The AI agent account may only call `request_spend`
//! (ideally through a NEAR function-call access key restricted to that method).
//! Every request is checked against owner-set policy:
//!   1. duplicate invoice hash          -> BLOCKED
//!   2. payee not on vendor allowlist   -> BLOCKED
//!   3. amount above hard per-tx cap    -> BLOCKED
//!   4. velocity (too many in window)   -> BLOCKED
//!   5. above approval threshold or would exceed daily cap -> HELD for owner
//!   6. otherwise                       -> PAID immediately
//! Every decision is stored on-chain and emitted as a NEP-297 event, with a
//! deterministic, rule-based explanation (no LLM required).
use near_sdk::json_types::{U128, U64};
use near_sdk::store::{IterableMap, IterableSet, LookupSet, Vector};
use near_sdk::{env, near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault, Promise};

const DAY_NS: u64 = 86_400_000_000_000;

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum Key {
    Allow,
    Seen,
    Pending,
    Decisions,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    /// Hard cap per payment (yoctoNEAR). Above this is blocked outright.
    pub per_tx_cap: U128,
    /// Payments at or above this need owner approval.
    pub approval_threshold: U128,
    /// Max auto-paid total per rolling UTC day.
    pub daily_cap: U128,
    /// Max requests accepted (paid or held) within `velocity_window_sec`.
    pub max_requests_per_window: u32,
    pub velocity_window_sec: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Paid,
    Held,
    Blocked,
    Approved,
    Rejected,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Decision {
    pub id: U64,
    pub to: AccountId,
    pub amount: U128,
    pub memo: String,
    pub invoice_hash: String,
    pub outcome: Outcome,
    pub rule: String,
    pub reason: String,
    pub timestamp_ns: U64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct PendingSpend {
    pub id: U64,
    pub to: AccountId,
    pub amount: U128,
    pub memo: String,
    pub invoice_hash: String,
    pub reason: String,
}

#[near(serializers = [json])]
pub struct VaultInfo {
    pub owner: AccountId,
    pub agent: AccountId,
    pub policy: Policy,
    pub balance: U128,
    pub spent_today: U128,
    pub allowlist: Vec<AccountId>,
    pub decisions: u32,
    pub pending: u32,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct AegisVault {
    owner: AccountId,
    agent: AccountId,
    policy: Policy,
    allowlist: IterableSet<AccountId>,
    seen_invoices: LookupSet<String>,
    pending: IterableMap<u64, PendingSpend>,
    decisions: Vector<Decision>,
    day_start_ns: u64,
    spent_today: u128,
    recent_ns: Vec<u64>,
    next_id: u64,
}

fn near_fmt(y: u128) -> String {
    let whole = y / 10u128.pow(24);
    let frac = (y % 10u128.pow(24)) / 10u128.pow(21);
    format!("{whole}.{frac:03} NEAR")
}

#[near]
impl AegisVault {
    #[init]
    pub fn new(owner: AccountId, agent: AccountId, policy: Policy, allowlist: Vec<AccountId>) -> Self {
        require!(policy.approval_threshold.0 <= policy.per_tx_cap.0, "approval_threshold must be <= per_tx_cap");
        let mut s = Self {
            owner,
            agent,
            policy,
            allowlist: IterableSet::new(Key::Allow),
            seen_invoices: LookupSet::new(Key::Seen),
            pending: IterableMap::new(Key::Pending),
            decisions: Vector::new(Key::Decisions),
            day_start_ns: env::block_timestamp(),
            spent_today: 0,
            recent_ns: vec![],
            next_id: 0,
        };
        for a in allowlist {
            s.allowlist.insert(a);
        }
        s
    }

    /// Anyone may top up the vault.
    #[payable]
    pub fn deposit(&mut self) {}

    // ---------------- agent ----------------

    /// Called by the AI agent. Never panics on a policy failure: the block is
    /// recorded on-chain so there is an audit trail of attempted bad spends.
    pub fn request_spend(&mut self, to: AccountId, amount: U128, memo: String, invoice_hash: String) -> Decision {
        require!(env::predecessor_account_id() == self.agent, "only the registered agent may request spends");
        require!(amount.0 > 0, "amount must be positive");
        require!(memo.len() <= 280 && invoice_hash.len() <= 128, "memo/invoice_hash too long");
        self.roll_day();
        let now = env::block_timestamp();
        let window = self.policy.velocity_window_sec * 1_000_000_000;
        self.recent_ns.retain(|t| now.saturating_sub(*t) < window);

        let amt = amount.0;
        let (outcome, rule, reason) = if !invoice_hash.is_empty() && self.seen_invoices.contains(&invoice_hash) {
            (Outcome::Blocked, "DUPLICATE_INVOICE", format!("Invoice {invoice_hash} was already paid or queued; duplicate payment refused."))
        } else if !self.allowlist.contains(&to) {
            (Outcome::Blocked, "PAYEE_NOT_ALLOWLISTED", format!("{to} is not an approved vendor. Owner must add it with add_vendor before the agent can pay it."))
        } else if amt > self.policy.per_tx_cap.0 {
            (Outcome::Blocked, "OVER_PER_TX_CAP", format!("{} exceeds the hard per-payment cap of {}.", near_fmt(amt), near_fmt(self.policy.per_tx_cap.0)))
        } else if self.recent_ns.len() as u32 >= self.policy.max_requests_per_window {
            (Outcome::Blocked, "VELOCITY_LIMIT", format!("{} requests already in the last {}s (limit {}). Possible runaway or hijacked agent.", self.recent_ns.len(), self.policy.velocity_window_sec, self.policy.max_requests_per_window))
        } else if amt >= self.policy.approval_threshold.0 {
            (Outcome::Held, "NEEDS_APPROVAL", format!("{} is at/above the approval threshold of {}; held for owner.", near_fmt(amt), near_fmt(self.policy.approval_threshold.0)))
        } else if self.spent_today + amt > self.policy.daily_cap.0 {
            (Outcome::Held, "DAILY_CAP", format!("Paying {} would take today's auto-spend to {} (cap {}); held for owner.", near_fmt(amt), near_fmt(self.spent_today + amt), near_fmt(self.policy.daily_cap.0)))
        } else if env::account_balance().as_yoctonear() < amt + NearToken::from_near(1).as_yoctonear() {
            (Outcome::Held, "LOW_BALANCE", "Vault balance too low after storage reserve; held for owner.".to_string())
        } else {
            (Outcome::Paid, "WITHIN_POLICY", format!("{} to approved vendor {to}, within caps.", near_fmt(amt)))
        };

        let id = self.next_id;
        self.next_id += 1;
        if outcome != Outcome::Blocked {
            self.recent_ns.push(now);
            if !invoice_hash.is_empty() {
                self.seen_invoices.insert(invoice_hash.clone());
            }
        }
        match outcome {
            Outcome::Paid => {
                self.spent_today += amt;
                Promise::new(to.clone()).transfer(NearToken::from_yoctonear(amt));
            }
            Outcome::Held => {
                self.pending.insert(id, PendingSpend { id: U64(id), to: to.clone(), amount, memo: memo.clone(), invoice_hash: invoice_hash.clone(), reason: reason.clone() });
            }
            _ => {}
        }
        self.record(id, to, amount, memo, invoice_hash, outcome, rule, reason)
    }

    // ---------------- owner ----------------

    pub fn approve(&mut self, id: U64) -> Decision {
        self.only_owner();
        let p = self.pending.remove(&id.0).unwrap_or_else(|| env::panic_str("no such pending spend"));
        Promise::new(p.to.clone()).transfer(NearToken::from_yoctonear(p.amount.0));
        let reason = format!("Owner approved held payment #{} ({}).", id.0, p.reason);
        self.record(id.0, p.to, p.amount, p.memo, p.invoice_hash, Outcome::Approved, "OWNER_APPROVED", reason)
    }

    pub fn reject(&mut self, id: U64) -> Decision {
        self.only_owner();
        let p = self.pending.remove(&id.0).unwrap_or_else(|| env::panic_str("no such pending spend"));
        let reason = format!("Owner rejected held payment #{}.", id.0);
        self.record(id.0, p.to, p.amount, p.memo, p.invoice_hash, Outcome::Rejected, "OWNER_REJECTED", reason)
    }

    pub fn set_policy(&mut self, policy: Policy) {
        self.only_owner();
        require!(policy.approval_threshold.0 <= policy.per_tx_cap.0, "approval_threshold must be <= per_tx_cap");
        self.policy = policy;
    }

    pub fn add_vendor(&mut self, account: AccountId) {
        self.only_owner();
        self.allowlist.insert(account);
    }

    pub fn remove_vendor(&mut self, account: AccountId) {
        self.only_owner();
        self.allowlist.remove(&account);
    }

    /// Kill switch: rotate the agent to a new account (or a dead one).
    pub fn set_agent(&mut self, agent: AccountId) {
        self.only_owner();
        self.agent = agent;
    }

    pub fn withdraw(&mut self, amount: U128) -> Promise {
        self.only_owner();
        Promise::new(self.owner.clone()).transfer(NearToken::from_yoctonear(amount.0))
    }

    // ---------------- views ----------------

    pub fn get_info(&self) -> VaultInfo {
        VaultInfo {
            owner: self.owner.clone(),
            agent: self.agent.clone(),
            policy: self.policy.clone(),
            balance: U128(env::account_balance().as_yoctonear()),
            spent_today: U128(self.spent_today),
            allowlist: self.allowlist.iter().cloned().collect(),
            decisions: self.decisions.len(),
            pending: self.pending.len(),
        }
    }

    pub fn get_pending(&self) -> Vec<PendingSpend> {
        self.pending.values().cloned().collect()
    }

    /// Newest first.
    pub fn get_decisions(&self, from_index: Option<u32>, limit: Option<u32>) -> Vec<Decision> {
        let n = self.decisions.len();
        let from = from_index.unwrap_or(0);
        let limit = limit.unwrap_or(50).min(100);
        (0..n).rev().skip(from as usize).take(limit as usize).filter_map(|i| self.decisions.get(i).cloned()).collect()
    }

    // ---------------- internal ----------------

    fn only_owner(&self) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
    }

    fn roll_day(&mut self) {
        let now = env::block_timestamp();
        if now.saturating_sub(self.day_start_ns) >= DAY_NS {
            self.day_start_ns = now;
            self.spent_today = 0;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record(&mut self, id: u64, to: AccountId, amount: U128, memo: String, invoice_hash: String, outcome: Outcome, rule: &str, reason: String) -> Decision {
        let d = Decision { id: U64(id), to, amount, memo, invoice_hash, outcome, rule: rule.to_string(), reason, timestamp_ns: U64(env::block_timestamp()) };
        env::log_str(&format!(
            "EVENT_JSON:{}",
            near_sdk::serde_json::json!({"standard":"aegis_vault","version":"1.0.0","event":"decision","data":[&d]})
        ));
        self.decisions.push(d.clone());
        d
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;

    fn acc(s: &str) -> AccountId { s.parse().unwrap() }
    fn n(x: u128) -> U128 { U128(x * 10u128.pow(24)) }
    fn ctx(pred: &str) {
        testing_env!(VMContextBuilder::new().predecessor_account_id(acc(pred)).account_balance(NearToken::from_near(100)).build());
    }
    fn vault() -> AegisVault {
        ctx("owner.near");
        AegisVault::new(acc("owner.near"), acc("agent.near"),
            Policy { per_tx_cap: n(5), approval_threshold: n(2), daily_cap: n(3), max_requests_per_window: 3, velocity_window_sec: 60 },
            vec![acc("vendor.near")])
    }

    #[test]
    fn paid_held_blocked() {
        let mut v = vault();
        ctx("agent.near");
        assert_eq!(v.request_spend(acc("vendor.near"), n(1), "ok".into(), "inv-1".into()).outcome, Outcome::Paid);
        assert_eq!(v.request_spend(acc("vendor.near"), n(1), "dup".into(), "inv-1".into()).rule, "DUPLICATE_INVOICE");
        assert_eq!(v.request_spend(acc("evil.near"), n(1), "x".into(), "inv-2".into()).rule, "PAYEE_NOT_ALLOWLISTED");
        assert_eq!(v.request_spend(acc("vendor.near"), n(9), "x".into(), "inv-3".into()).rule, "OVER_PER_TX_CAP");
        assert_eq!(v.request_spend(acc("vendor.near"), n(3), "x".into(), "inv-4".into()).outcome, Outcome::Held);
        assert_eq!(v.get_pending().len(), 1);
    }

    #[test]
    fn velocity_limit() {
        let mut v = vault();
        v.policy.daily_cap = n(50);
        ctx("agent.near");
        for i in 0..3 { assert_eq!(v.request_spend(acc("vendor.near"), n(1), "x".into(), format!("v{i}")).outcome, Outcome::Paid); }
        assert_eq!(v.request_spend(acc("vendor.near"), n(1), "x".into(), "v9".into()).rule, "VELOCITY_LIMIT");
    }

    #[test]
    #[should_panic(expected = "only the registered agent")]
    fn stranger_cannot_spend() {
        let mut v = vault();
        ctx("evil.near");
        v.request_spend(acc("vendor.near"), n(1), "x".into(), "i".into());
    }

    #[test]
    #[should_panic(expected = "owner only")]
    fn agent_cannot_approve() {
        let mut v = vault();
        ctx("agent.near");
        v.request_spend(acc("vendor.near"), n(3), "x".into(), "i".into());
        v.approve(U64(0));
    }
}

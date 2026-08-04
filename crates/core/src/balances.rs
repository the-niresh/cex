//! The balance ledger.
//!
//! Two rules, and everything else follows from them:
//!
//! 1. **Reading never mutates.** `cex` minted 100,000 USD and 10 BTC for any
//!    unseen user from inside a getter, which meant balances were not conserved
//!    and no test could have caught it. [`Balances::get`] takes `&self`.
//! 2. **Every account has an `available` and a `locked` half.** Funds backing a
//!    resting order are moved to `locked`, not deducted from a single total, so
//!    the sum over both halves plus the fee account is constant per asset.
//!
//! Every mutating operation is checked and refuses rather than going negative.

use std::collections::BTreeMap;

use cex_proto::{BalanceView, UserId};
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::math::{checked_add, checked_sub};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Balance {
    /// Spendable right now.
    pub available: i64,
    /// Reserved against a resting order. Still owned by the user, not spendable.
    pub locked: i64,
}

impl Balance {
    #[inline]
    pub fn total(&self) -> i64 {
        self.available + self.locked
    }
}

/// Nested `user -> asset -> balance`.
///
/// `BTreeMap` rather than `HashMap` so iteration order is deterministic —
/// snapshots must be byte-identical across runs or replay cannot be verified.
///
/// Nested rather than keyed on a `(user, asset)` tuple for two reasons: a tuple
/// cannot be expressed as a JSON object key at all, so a snapshot of the flat
/// form simply fails; and listing one user's balances becomes a single lookup
/// instead of a scan of every account on the exchange.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Balances {
    inner: BTreeMap<UserId, BTreeMap<String, Balance>>,
}

impl Balances {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read an account. Never creates one.
    pub fn get(&self, user: UserId, asset: &str) -> Balance {
        self.inner
            .get(&user)
            .and_then(|by_asset| by_asset.get(asset))
            .copied()
            .unwrap_or_default()
    }

    fn entry(&mut self, user: UserId, asset: &str) -> &mut Balance {
        self.inner
            .entry(user)
            .or_default()
            .entry(asset.to_string())
            .or_default()
    }

    fn require_non_negative(amount: i64) -> Result<(), EngineError> {
        if amount < 0 {
            return Err(EngineError::NonPositiveAmount);
        }
        Ok(())
    }

    /// Add to `available`. Used for deposits and for the receiving side of a fill.
    pub fn credit(&mut self, user: UserId, asset: &str, amount: i64) -> Result<(), EngineError> {
        Self::require_non_negative(amount)?;
        let bal = self.entry(user, asset);
        bal.available = checked_add(bal.available, amount)?;
        Ok(())
    }

    /// Remove from `available`. Used for withdrawals.
    pub fn debit(&mut self, user: UserId, asset: &str, amount: i64) -> Result<(), EngineError> {
        Self::require_non_negative(amount)?;
        let current = self.get(user, asset);
        if current.available < amount {
            return Err(EngineError::InsufficientBalance {
                asset: asset.to_string(),
                need: amount,
                have: current.available,
            });
        }
        let bal = self.entry(user, asset);
        bal.available = checked_sub(bal.available, amount)?;
        Ok(())
    }

    /// Reserve funds against a resting order: `available` → `locked`.
    pub fn lock(&mut self, user: UserId, asset: &str, amount: i64) -> Result<(), EngineError> {
        Self::require_non_negative(amount)?;
        let current = self.get(user, asset);
        if current.available < amount {
            return Err(EngineError::InsufficientBalance {
                asset: asset.to_string(),
                need: amount,
                have: current.available,
            });
        }
        let bal = self.entry(user, asset);
        bal.available = checked_sub(bal.available, amount)?;
        bal.locked = checked_add(bal.locked, amount)?;
        Ok(())
    }

    /// Release a reservation without spending it: `locked` → `available`.
    /// Used on cancel, on an unfilled remainder, and on price improvement.
    pub fn unlock(&mut self, user: UserId, asset: &str, amount: i64) -> Result<(), EngineError> {
        Self::require_non_negative(amount)?;
        let current = self.get(user, asset);
        if current.locked < amount {
            return Err(EngineError::InsufficientBalance {
                asset: asset.to_string(),
                need: amount,
                have: current.locked,
            });
        }
        let bal = self.entry(user, asset);
        bal.locked = checked_sub(bal.locked, amount)?;
        bal.available = checked_add(bal.available, amount)?;
        Ok(())
    }

    /// Spend a reservation: the funds leave this account entirely. The caller is
    /// responsible for crediting them somewhere else in the same command, or
    /// supply will not be conserved.
    pub fn settle_locked(
        &mut self,
        user: UserId,
        asset: &str,
        amount: i64,
    ) -> Result<(), EngineError> {
        Self::require_non_negative(amount)?;
        let current = self.get(user, asset);
        if current.locked < amount {
            return Err(EngineError::InsufficientBalance {
                asset: asset.to_string(),
                need: amount,
                have: current.locked,
            });
        }
        let bal = self.entry(user, asset);
        bal.locked = checked_sub(bal.locked, amount)?;
        Ok(())
    }

    /// Every atom of `asset` held anywhere in the ledger, available or locked.
    /// The conservation check compares this against what was deposited.
    pub fn total_supply(&self, asset: &str) -> i64 {
        self.inner
            .values()
            .filter_map(|by_asset| by_asset.get(asset))
            .map(|b| b.total())
            .sum()
    }

    /// Non-empty balances for one user. One lookup, not a scan.
    pub fn for_user(&self, user: UserId) -> Vec<BalanceView> {
        self.inner
            .get(&user)
            .map(|by_asset| {
                by_asset
                    .iter()
                    .filter(|(_, b)| b.total() != 0)
                    .map(|(asset, b)| BalanceView {
                        asset: asset.clone(),
                        available: b.available,
                        locked: b.locked,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every `(user, asset, balance)` currently tracked. Used by the invariant check.
    pub fn accounts(&self) -> impl Iterator<Item = (UserId, &str, &Balance)> {
        self.inner.iter().flat_map(|(user, by_asset)| {
            by_asset
                .iter()
                .map(move |(asset, bal)| (*user, asset.as_str(), bal))
        })
    }
}

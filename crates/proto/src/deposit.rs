//! Per-deposit ceilings shared by the API and the demo tooling.
//!
//! One definition of the limits so a load driver cannot drift from what
//! `POST /deposit` enforces.

/// Per-deposit limit in whole units of USDT.
pub const DEPOSIT_LIMIT_USDT: i64 = 1_000_000;
/// Per-deposit limit in whole BTC.
pub const DEPOSIT_LIMIT_BTC: i64 = 10;
/// Per-deposit limit in whole ETH.
pub const DEPOSIT_LIMIT_ETH: i64 = 100;
/// Per-deposit limit in whole SOL.
pub const DEPOSIT_LIMIT_SOL: i64 = 10_000;

fn pow10(n: u32) -> i64 {
    10i64.pow(n)
}

/// Whole-unit ceiling and atom scale for a known asset, or `None` when unchecked.
pub fn deposit_limit_whole(asset: &str) -> Option<(i64, u32)> {
    match asset {
        "USDT" => Some((DEPOSIT_LIMIT_USDT, 6)),
        "BTC" => Some((DEPOSIT_LIMIT_BTC, 8)),
        "ETH" => Some((DEPOSIT_LIMIT_ETH, 8)),
        "SOL" => Some((DEPOSIT_LIMIT_SOL, 8)),
        _ => None,
    }
}

/// Maximum atoms allowed in a single deposit for a known asset.
pub fn deposit_limit_atoms(asset: &str) -> Option<i64> {
    deposit_limit_whole(asset).map(|(whole, decimals)| whole * pow10(decimals))
}

/// Split `total` atoms into deposits that each fit the per-asset ceiling.
///
/// Unknown assets are returned as one chunk. Zero or negative totals yield none.
pub fn deposit_chunks(asset: &str, total: i64) -> Vec<i64> {
    if total <= 0 {
        return Vec::new();
    }
    let Some(limit) = deposit_limit_atoms(asset) else {
        return vec![total];
    };
    let mut remaining = total;
    let mut chunks = Vec::new();
    while remaining > 0 {
        let chunk = remaining.min(limit);
        chunks.push(chunk);
        remaining -= chunk;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdt_at_limit_is_one_chunk_and_one_atom_over_needs_two() {
        let limit = deposit_limit_atoms("USDT").unwrap();
        assert_eq!(deposit_chunks("USDT", limit), vec![limit]);
        assert_eq!(deposit_chunks("USDT", limit + 1), vec![limit, 1]);
    }

    #[test]
    fn btc_splits_on_the_ceiling() {
        let limit = deposit_limit_atoms("BTC").unwrap();
        assert_eq!(deposit_chunks("BTC", limit * 3), vec![limit, limit, limit]);
        assert_eq!(
            deposit_chunks("BTC", limit * 3 + 1),
            vec![limit, limit, limit, 1]
        );
    }

    #[test]
    fn eth_and_sol_use_their_own_limits() {
        let eth = deposit_limit_atoms("ETH").unwrap();
        let sol = deposit_limit_atoms("SOL").unwrap();
        assert_eq!(deposit_chunks("ETH", eth + 1), vec![eth, 1]);
        assert_eq!(deposit_chunks("SOL", sol * 2), vec![sol, sol]);
    }

    #[test]
    fn unknown_assets_are_not_split() {
        assert_eq!(deposit_chunks("DOGE", 999), vec![999]);
    }
}

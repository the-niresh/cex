//! Market definitions and the scaling rules that hold everywhere else together.
//!
//! Scaling convention, fixed once and never revisited:
//!
//! * A **quantity** is an integer count of `10^-base_decimals` units of the base
//!   asset. For BTC with 8 decimals, `qty = 50_000_000` is 0.5 BTC.
//! * A **price** is an integer count of quote atomic units per *one whole* base
//!   unit. For BTC/USDT with USDT at 6 decimals, `price = 65_000_500_000` is
//!   65,000.50 USDT per BTC.
//! * Therefore `notional = price * qty / 10^base_decimals`, in quote atoms.

use std::collections::BTreeMap;

use cex_proto::MarketView;
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::math::{mul_div, pow10, Rounding};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub symbol: String,
    pub base: String,
    pub quote: String,
    pub base_decimals: u32,
    pub quote_decimals: u32,
    /// Minimum price increment, in quote atoms. Every price must be a multiple.
    pub tick_size: i64,
    /// Minimum quantity increment, in base atoms. Every quantity must be a multiple.
    pub lot_size: i64,
    /// Smallest permitted order notional, in quote atoms. Keeps dust off the book.
    pub min_notional: i64,
    pub maker_fee_bps: i64,
    pub taker_fee_bps: i64,
}

impl Market {
    /// `10^base_decimals` — the divisor in every notional computation.
    #[inline]
    pub fn base_unit(&self) -> Result<i64, EngineError> {
        Ok(pow10(self.base_decimals)?)
    }

    /// Quote atoms for `qty` base atoms at `price`.
    ///
    /// `rounding` decides who absorbs the sub-atom remainder: charge a buyer with
    /// [`Rounding::Up`], credit a seller with [`Rounding::Down`], so the exchange
    /// is never the one left short.
    pub fn notional(&self, price: i64, qty: i64, rounding: Rounding) -> Result<i64, EngineError> {
        Ok(mul_div(price, qty, self.base_unit()?, rounding)?)
    }

    /// Fee on a notional, always rounded up.
    pub fn fee(&self, notional: i64, bps: i64) -> Result<i64, EngineError> {
        Ok(mul_div(notional, bps, 10_000, Rounding::Up)?)
    }

    pub fn validate_price(&self, price: i64) -> Result<(), EngineError> {
        if price <= 0 || price % self.tick_size != 0 {
            return Err(EngineError::BadTick(self.tick_size));
        }
        Ok(())
    }

    pub fn validate_qty(&self, qty: i64) -> Result<(), EngineError> {
        if qty <= 0 || qty % self.lot_size != 0 {
            return Err(EngineError::BadLot(self.lot_size));
        }
        Ok(())
    }

    pub fn view(&self) -> MarketView {
        MarketView {
            symbol: self.symbol.clone(),
            base: self.base.clone(),
            quote: self.quote.clone(),
            base_decimals: self.base_decimals,
            quote_decimals: self.quote_decimals,
            tick_size: self.tick_size,
            lot_size: self.lot_size,
            min_notional: self.min_notional,
            maker_fee_bps: self.maker_fee_bps,
            taker_fee_bps: self.taker_fee_bps,
        }
    }
}

/// The set of listed markets and the assets they imply.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketRegistry {
    markets: BTreeMap<String, Market>,
}

impl MarketRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The markets the exchange boots with. Values chosen to look like a real
    /// venue: 8-decimal base assets, 6-decimal USDT quote, 2 bps maker / 5 bps taker.
    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.insert(Market {
            symbol: "BTC_USDT".into(),
            base: "BTC".into(),
            quote: "USDT".into(),
            base_decimals: 8,
            quote_decimals: 6,
            tick_size: 10_000,       // 0.01 USDT
            lot_size: 1_000,         // 0.00001 BTC
            min_notional: 1_000_000, // 1 USDT
            maker_fee_bps: 2,
            taker_fee_bps: 5,
        });
        r.insert(Market {
            symbol: "ETH_USDT".into(),
            base: "ETH".into(),
            quote: "USDT".into(),
            base_decimals: 8,
            quote_decimals: 6,
            tick_size: 1_000, // 0.001 USDT
            lot_size: 10_000, // 0.0001 ETH
            min_notional: 1_000_000,
            maker_fee_bps: 2,
            taker_fee_bps: 5,
        });
        r.insert(Market {
            symbol: "SOL_USDT".into(),
            base: "SOL".into(),
            quote: "USDT".into(),
            base_decimals: 8,
            quote_decimals: 6,
            tick_size: 100,    // 0.0001 USDT
            lot_size: 100_000, // 0.001 SOL
            min_notional: 1_000_000,
            maker_fee_bps: 2,
            taker_fee_bps: 5,
        });
        r
    }

    pub fn insert(&mut self, market: Market) {
        self.markets.insert(market.symbol.clone(), market);
    }

    pub fn get(&self, symbol: &str) -> Result<&Market, EngineError> {
        self.markets
            .get(symbol)
            .ok_or_else(|| EngineError::UnknownMarket(symbol.to_string()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Market> {
        self.markets.values()
    }

    pub fn symbols(&self) -> impl Iterator<Item = &String> {
        self.markets.keys()
    }

    /// Every asset referenced by any listed market. Used to validate deposits and
    /// to bound the conservation check.
    pub fn assets(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for m in self.markets.values() {
            for a in [&m.base, &m.quote] {
                if !seen.iter().any(|s| s == a) {
                    seen.push(a.clone());
                }
            }
        }
        seen.sort();
        seen
    }

    pub fn has_asset(&self, asset: &str) -> bool {
        self.markets
            .values()
            .any(|m| m.base == asset || m.quote == asset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn btc() -> Market {
        MarketRegistry::with_defaults()
            .get("BTC_USDT")
            .unwrap()
            .clone()
    }

    #[test]
    fn notional_matches_the_worked_example() {
        let m = btc();
        // 0.5 BTC at 65,000.50 USDT = 32,500.25 USDT
        let n = m
            .notional(65_000_500_000, 50_000_000, Rounding::Down)
            .unwrap();
        assert_eq!(n, 32_500_250_000);
    }

    #[test]
    fn notional_rounding_direction_is_respected() {
        let m = btc();
        // A price and qty chosen so the product does not divide evenly.
        let price = 65_000_010_000;
        let qty = 1_500; // 0.000015 BTC
        let down = m.notional(price, qty, Rounding::Down).unwrap();
        let up = m.notional(price, qty, Rounding::Up).unwrap();
        assert_eq!(up - down, 1, "up and down must differ by exactly one atom");
    }

    #[test]
    fn fees_always_round_up() {
        let m = btc();
        // 5 bps of 1 atom is 0.0005 atoms — must still charge 1, never 0.
        assert_eq!(m.fee(1, 5).unwrap(), 1);
        assert_eq!(m.fee(0, 5).unwrap(), 0);
        assert_eq!(m.fee(1_000_000, 5).unwrap(), 500);
    }

    #[test]
    fn tick_and_lot_validation_rejects_misaligned_input() {
        let m = btc();
        assert!(m.validate_price(10_000).is_ok());
        assert!(m.validate_price(10_001).is_err());
        assert!(m.validate_price(0).is_err());
        assert!(m.validate_price(-10_000).is_err());
        assert!(m.validate_qty(1_000).is_ok());
        assert!(m.validate_qty(1_001).is_err());
        assert!(m.validate_qty(0).is_err());
    }

    #[test]
    fn registry_reports_every_referenced_asset() {
        let r = MarketRegistry::with_defaults();
        assert_eq!(r.assets(), vec!["BTC", "ETH", "SOL", "USDT"]);
        assert!(r.has_asset("USDT"));
        assert!(!r.has_asset("DOGE"));
    }
}

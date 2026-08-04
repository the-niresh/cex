use thiserror::Error;

use crate::math::MathError;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EngineError {
    #[error("unknown market {0}")]
    UnknownMarket(String),
    #[error("unknown asset {0}")]
    UnknownAsset(String),
    #[error("unknown order {0}")]
    UnknownOrder(u64),
    #[error("order {0} does not belong to this user")]
    NotOrderOwner(u64),
    #[error("order {0} is already closed")]
    OrderClosed(u64),
    #[error("amount must be positive")]
    NonPositiveAmount,
    #[error("limit order requires a price")]
    MissingPrice,
    #[error("price must be a positive multiple of tick size {0}")]
    BadTick(i64),
    #[error("quantity must be a positive multiple of lot size {0}")]
    BadLot(i64),
    #[error("order notional is below the market minimum of {0}")]
    BelowMinNotional(i64),
    #[error("insufficient {asset}: need {need}, have {have}")]
    InsufficientBalance {
        asset: String,
        need: i64,
        have: i64,
    },
    #[error("market order cannot be filled: book has only {available} of {requested}")]
    InsufficientLiquidity { requested: i64, available: i64 },
    #[error(transparent)]
    Math(#[from] MathError),
}

//! Keeps a demo exchange looking like a market instead of a screenshot.
//!
//! A deployed venue nobody trades on shows a frozen book, an empty tape and a
//! chart that stopped forming. Every part of that is working correctly and all
//! of it reads as broken. This rests a ladder either side of a mid, refreshes
//! it on a timer, and crosses its own spread now and then so prints land on the
//! tape and candles keep forming.
//!
//! Two accounts, the same split the load driver uses: a maker holds the
//! quotes, a taker crosses them. One account doing both would be an order
//! trading with itself, which the engine is entitled to reject and which would
//! make the fills nonsense to read.
//!
//! **This is demo dressing, not a market.** The prices are a random walk inside
//! a band and the volume is this process talking to itself. It exists so an
//! empty venue reads as quiet rather than broken, and for nothing else.
//!
//! Run it against a stack that is already up:
//!   cargo run -p cex-loadgen --bin demo-maker -- \
//!     --host http://localhost:8080

use std::time::Duration;

use anyhow::{Context, Result};
use cex_loadgen::quotes::{ladder, order_notional, quote_mid, walk, Rng};
use cex_loadgen::venue::{
    cancel, fund, last_trade_price, market_limits, place_limit, place_market, register, touch,
    with_reauth, Backoff, Funding, StateLogger,
};
use cex_proto::deposit::deposit_limit_atoms;
use cex_proto::Side;
use clap::Parser;

/// BTC_USDT ticks at 0.01 USDT, in quote atoms.
const TICK: i64 = 10_000;
/// Where the walk starts and the middle of the band it is held in.
const MID: i64 = 50_000_000_000;

/// Enough of both assets for a multi-day run, funded in limit-sized deposits.
fn funding_targets() -> Funding {
    let usdt = deposit_limit_atoms("USDT").expect("USDT limit");
    let btc = deposit_limit_atoms("BTC").expect("BTC limit") * 10;
    Funding { usdt, btc }
}

#[derive(Parser)]
#[command(about = "Rests and refreshes a demo book so an idle venue looks alive")]
struct Args {
    /// Base URL of the API, e.g. http://localhost:8080
    #[arg(long)]
    host: String,
    #[arg(long, default_value = "BTC_USDT")]
    symbol: String,
    /// How many price levels to quote on each side.
    #[arg(long, default_value_t = 12)]
    levels: usize,
    /// Size of the level nearest the touch, in base atoms. Deeper levels scale up.
    #[arg(long, default_value_t = 120_000)]
    size: i64,
    /// Seconds between re-quotes.
    #[arg(long, default_value_t = 5)]
    refresh: u64,
    /// Cross the spread on roughly one cycle in this many. 0 never trades.
    #[arg(long, default_value_t = 6)]
    trade_every: u64,
    /// How far the mid may wander from its start, in ticks.
    #[arg(long, default_value_t = 400)]
    band_ticks: i64,
    /// Seed for the price walk. Omit for a different market each run.
    #[arg(long)]
    seed: Option<u64>,
    /// Stop after this many cycles. Omit to run until interrupted.
    #[arg(long)]
    cycles: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let http = reqwest::Client::new();
    let funding = funding_targets();

    let mut maker = register(&http, &args.host, "demomaker")
        .await
        .context("register the maker")?;
    let mut taker = register(&http, &args.host, "demotaker")
        .await
        .context("register the taker")?;
    fund(&http, &args.host, &maker, funding.usdt, funding.btc)
        .await
        .context("fund the maker")?;
    fund(&http, &args.host, &taker, funding.usdt, funding.btc)
        .await
        .context("fund the taker")?;

    let seed = args.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5EED)
    });
    let mut rng = Rng::new(seed);

    // Anchor on the last trade, not on stray resting orders far from the market.
    let reference = last_trade_price(&http, &args.host, &args.symbol)
        .await
        .context("read the tape before quoting")?
        .unwrap_or(MID);
    let limits = market_limits(&http, &args.host, &args.symbol)
        .await
        .context("read market limits")?;
    let (best_bid, best_ask) = touch(&http, &args.host, &args.symbol)
        .await
        .context("read the book before quoting")?;
    let choice = quote_mid(reference, best_bid, best_ask, TICK);
    let mut mid = choice.mid;
    let band = (mid - args.band_ticks * TICK, mid + args.band_ticks * TICK);

    println!(
        "demo-maker: {} on {} — {} levels/side, refresh {}s, seed {seed}",
        args.symbol, args.host, args.levels, args.refresh
    );
    println!(
        "demo-maker: reference {reference}, book bid {:?} ask {:?}, quoting around {}",
        best_bid, best_ask, mid
    );

    // What this process has resting right now. Cancelled at the top of every
    // cycle, so quotes are replaced rather than piled up — an hour of adding
    // without removing would bury the ladder under its own history.
    let mut resting: Vec<u64> = Vec::new();
    let mut cycle: u64 = 0;
    let mut maker_log = StateLogger::new();
    let mut taker_log = StateLogger::new();
    let mut quote_log = StateLogger::new();
    if choice.ignored_stray_book {
        quote_log.ignored_stray_book(reference, choice.stray_touch.unwrap_or(mid));
    }
    let mut maker_backoff = Backoff::new();
    let mut taker_backoff = Backoff::new();

    loop {
        if let Some(limit) = args.cycles {
            if cycle >= limit {
                break;
            }
        }
        cycle += 1;

        for id in resting.drain(..) {
            let host = args.host.clone();
            if let Err(e) = with_reauth(
                &http,
                &host,
                &mut maker,
                funding,
                &mut maker_log,
                &mut maker_backoff,
                |who| {
                    let http = &http;
                    let host = host.clone();
                    let who = who.clone();
                    async move { cancel(http, &host, &who, id).await }
                },
            )
            .await
            {
                eprintln!("demo-maker: cancel {id}: {e}");
            }
        }

        mid = walk(mid, TICK, rng.drift(), band);

        for quote in ladder(mid, TICK, args.levels, args.size) {
            if order_notional(quote.price, quote.qty, limits.base_decimals) < limits.min_notional {
                quote_log.below_min_notional();
                continue;
            }
            let side = match quote.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            };
            let host = args.host.clone();
            let symbol = args.symbol.clone();
            let price = quote.price;
            let qty = quote.qty;
            match with_reauth(
                &http,
                &host,
                &mut maker,
                funding,
                &mut maker_log,
                &mut maker_backoff,
                |who| {
                    let http = &http;
                    let host = host.clone();
                    let symbol = symbol.clone();
                    let who = who.clone();
                    async move { place_limit(http, &host, &who, &symbol, side, price, qty).await }
                },
            )
            .await
            {
                Ok(id) => resting.push(id),
                Err(e) => eprintln!("demo-maker: quote {side} {price}: {e}"),
            }
        }

        // A trade, sometimes. Without this the book moves but nothing ever
        // prints, so the tape stays empty and the chart stops forming — two of
        // the three things that made the screen look dead.
        if args.trade_every > 0 && rng.one_in(args.trade_every) {
            let side = if rng.one_in(2) { "BUY" } else { "SELL" };
            let qty = args.size / 2;
            let host = args.host.clone();
            let symbol = args.symbol.clone();
            match with_reauth(
                &http,
                &host,
                &mut taker,
                funding,
                &mut taker_log,
                &mut taker_backoff,
                |who| {
                    let http = &http;
                    let host = host.clone();
                    let symbol = symbol.clone();
                    let who = who.clone();
                    async move { place_market(http, &host, &who, &symbol, side, qty).await }
                },
            )
            .await
            {
                Ok(id) => println!("demo-maker: cycle {cycle} traded {side} {qty} (order {id})"),
                Err(e) => eprintln!("demo-maker: market {side}: {e}"),
            }
        }

        tokio::time::sleep(Duration::from_secs(args.refresh)).await;
    }

    // Deliberately leaves the last ladder resting. A book with stale quotes in
    // it reads better than an empty one, and the next run cancels nothing it
    // does not own anyway.
    println!(
        "demo-maker: stopped after {cycle} cycles, {} quotes left resting",
        resting.len()
    );
    Ok(())
}

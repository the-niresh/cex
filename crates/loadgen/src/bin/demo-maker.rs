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
use cex_loadgen::quotes::{ladder, opening_mid, walk, Rng};
use cex_loadgen::venue::{cancel, fund, place_limit, place_market, register, touch};
use cex_proto::Side;
use clap::Parser;

/// BTC_USDT ticks at 0.01 USDT, in quote atoms.
const TICK: i64 = 10_000;
/// Where the walk starts and the middle of the band it is held in.
const MID: i64 = 50_000_000_000;
/// Enough of both assets that a run measured in days cannot go broke quoting.
const FUNDING: i64 = 1_000_000_000_000_000;

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

    let maker = register(&http, &args.host, "demomaker")
        .await
        .context("register the maker")?;
    let taker = register(&http, &args.host, "demotaker")
        .await
        .context("register the taker")?;
    fund(&http, &args.host, &maker, FUNDING)
        .await
        .context("fund the maker")?;
    fund(&http, &args.host, &taker, FUNDING)
        .await
        .context("fund the taker")?;

    let seed = args.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5EED)
    });
    let mut rng = Rng::new(seed);

    // Where the market actually is, not where this tool assumed it was. Skipping
    // this against a venue trading 119 ticks away from `MID` put every ask below
    // the best bid and sold into the book on contact.
    let (best_bid, best_ask) = touch(&http, &args.host, &args.symbol)
        .await
        .context("read the book before quoting")?;
    let mut mid = opening_mid(best_bid, best_ask, TICK, MID);
    let band = (mid - args.band_ticks * TICK, mid + args.band_ticks * TICK);

    println!(
        "demo-maker: {} on {} — {} levels/side, refresh {}s, seed {seed}",
        args.symbol, args.host, args.levels, args.refresh
    );
    println!(
        "demo-maker: book shows bid {:?} ask {:?} — quoting around {}",
        best_bid, best_ask, mid
    );

    // What this process has resting right now. Cancelled at the top of every
    // cycle, so quotes are replaced rather than piled up — an hour of adding
    // without removing would bury the ladder under its own history.
    let mut resting: Vec<u64> = Vec::new();
    let mut cycle: u64 = 0;

    loop {
        if let Some(limit) = args.cycles {
            if cycle >= limit {
                break;
            }
        }
        cycle += 1;

        for id in resting.drain(..) {
            // One failed cancel must not end the run: the order may have just
            // traded, and the next cycle re-quotes the level anyway.
            if let Err(e) = cancel(&http, &args.host, &maker, id).await {
                eprintln!("demo-maker: cancel {id}: {e}");
            }
        }

        mid = walk(mid, TICK, rng.drift(), band);

        for quote in ladder(mid, TICK, args.levels, args.size) {
            let side = match quote.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            };
            match place_limit(
                &http,
                &args.host,
                &maker,
                &args.symbol,
                side,
                quote.price,
                quote.qty,
            )
            .await
            {
                Ok(id) => resting.push(id),
                Err(e) => eprintln!("demo-maker: quote {side} {}: {e}", quote.price),
            }
        }

        // A trade, sometimes. Without this the book moves but nothing ever
        // prints, so the tape stays empty and the chart stops forming — two of
        // the three things that made the screen look dead.
        if args.trade_every > 0 && rng.one_in(args.trade_every) {
            let side = if rng.one_in(2) { "BUY" } else { "SELL" };
            let qty = args.size / 2;
            match place_market(&http, &args.host, &taker, &args.symbol, side, qty).await {
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

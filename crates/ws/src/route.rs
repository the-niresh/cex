//! Turning one `EventBatch` into the updates it should produce.
//!
//! Pure: no sockets, no clock, no Redis. That is deliberate — the rule that a
//! public channel never carries a user id, and that a private update reaches
//! exactly one user, is enforced here and can be tested without opening a
//! connection to anything.

use cex_proto::{Event, EventBatch, Fill, Seq, UserId};

use crate::wire::{Channel, DepthUpdate, Envelope, OrderUpdate, Payload, PublicTrade, Role};

/// One update, addressed and already serialised.
///
/// The payload is encoded once here rather than once per subscriber, because a
/// popular market has thousands of them and they all get the same bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub channel: Channel,
    /// `Some` means private to that user. `None` means anyone subscribed may
    /// see it, which is only ever true of `Depth` and `Trades`.
    pub audience: Option<UserId>,
    pub seq: Seq,
    pub payload: String,
}

impl Update {
    fn new(channel: Channel, audience: Option<UserId>, seq: Seq, data: Payload) -> Update {
        let payload = serde_json::to_string(&Envelope {
            channel: channel.to_string(),
            seq,
            data,
        })
        // Every type in `Payload` is a plain struct of integers, strings and
        // unit enums. There is no input that makes this fail.
        .expect("an envelope is always serialisable");

        Update {
            channel,
            audience,
            seq,
            payload,
        }
    }
}

/// Fan one batch out into the updates it produces, in order.
pub fn route(batch: &EventBatch) -> Vec<Update> {
    let mut out = Vec::new();
    for event in &batch.events {
        route_event(event, batch.seq, &mut out);
    }
    out
}

fn route_event(event: &Event, seq: Seq, out: &mut Vec<Update>) {
    match event {
        Event::DepthUpdated {
            symbol,
            depth_seq,
            deltas,
        } => out.push(Update::new(
            Channel::Depth(symbol.clone()),
            None,
            seq,
            Payload::Depth(DepthUpdate {
                symbol: symbol.clone(),
                depth_seq: *depth_seq,
                deltas: deltas.clone(),
            }),
        )),

        Event::Trades { symbol, fills } => {
            for fill in fills {
                // The public print. Built from named fields rather than from
                // the `Fill` wholesale, so a field added to `Fill` later cannot
                // arrive here by accident.
                out.push(Update::new(
                    Channel::Trades(symbol.clone()),
                    None,
                    seq,
                    Payload::Trade(PublicTrade {
                        symbol: fill.symbol.clone(),
                        price: fill.price,
                        qty: fill.qty,
                        taker_side: fill.taker_side,
                    }),
                ));

                // And one private message to each side, about its own order.
                out.push(private_fill(fill, Role::Maker, seq));
                out.push(private_fill(fill, Role::Taker, seq));
            }
        }

        Event::OrderAccepted {
            order_id,
            user_id,
            symbol,
            side,
            order_type,
            price,
            qty,
        } => out.push(Update::new(
            Channel::Orders,
            Some(*user_id),
            seq,
            Payload::Order(OrderUpdate::Accepted {
                order_id: *order_id,
                symbol: symbol.clone(),
                side: *side,
                order_type: *order_type,
                price: *price,
                qty: *qty,
            }),
        )),

        Event::OrderUpdated {
            order_id,
            user_id,
            filled_qty,
            qty,
            status,
        } => out.push(Update::new(
            Channel::Orders,
            Some(*user_id),
            seq,
            Payload::Order(OrderUpdate::Updated {
                order_id: *order_id,
                filled_qty: *filled_qty,
                qty: *qty,
                status: *status,
            }),
        )),

        Event::OrderCancelled {
            order_id,
            user_id,
            symbol,
            unfilled_qty,
        } => out.push(Update::new(
            Channel::Orders,
            Some(*user_id),
            seq,
            Payload::Order(OrderUpdate::Cancelled {
                order_id: *order_id,
                symbol: symbol.clone(),
                unfilled_qty: *unfilled_qty,
            }),
        )),

        Event::OrderRejected {
            user_id,
            symbol,
            reason,
        } => out.push(Update::new(
            Channel::Orders,
            Some(*user_id),
            seq,
            Payload::Order(OrderUpdate::Rejected {
                symbol: symbol.clone(),
                reason: reason.clone(),
            }),
        )),

        // Balances are not one of the three channels. Routing them to `orders`
        // would hand clients a message shape they never asked for; they belong
        // to a balance feed, if and when there is one.
        Event::Deposited { .. } | Event::Withdrawn { .. } | Event::BalanceUpdated { .. } => {}
    }
}

/// One side's private view of a fill.
///
/// Each side is told its own order, its own side, its own fee and its own role.
/// The counterparty is not named, and there is no field here that could name it.
fn private_fill(fill: &Fill, role: Role, seq: Seq) -> Update {
    let (user, order_id, side, fee) = match role {
        Role::Maker => (
            fill.maker_user_id,
            fill.maker_order_id,
            fill.taker_side.opposite(),
            fill.maker_fee,
        ),
        Role::Taker => (
            fill.taker_user_id,
            fill.taker_order_id,
            fill.taker_side,
            fill.taker_fee,
        ),
    };

    Update::new(
        Channel::Orders,
        Some(user),
        seq,
        Payload::Order(OrderUpdate::Fill {
            order_id,
            symbol: fill.symbol.clone(),
            price: fill.price,
            qty: fill.qty,
            side,
            fee,
            role,
        }),
    )
}

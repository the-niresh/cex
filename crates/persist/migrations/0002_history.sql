-- History. Everything here is written behind the engine by the persister and is
-- never on a request path — the engine must never wait on a database.
--
-- Numbered to continue the sequence started by the api crate's 0001_users.sql.
-- Each binary owns and applies its own tables at boot, so the file lives next to
-- the code that writes it rather than in a shared directory no crate owns.

-- The dedupe guard, and the reason redelivery is safe.
--
-- Recovery makes the engine re-apply commands after the last snapshot, so it
-- republishes events that were already published — same `seq`, new stream id.
-- Redis will also redeliver anything this consumer did not acknowledge. Both
-- are normal. A batch is written at most once because writing it and recording
-- it here happen in the same transaction.
CREATE TABLE IF NOT EXISTS event_batches (
    seq        BIGINT PRIMARY KEY,
    request_id UUID NOT NULL,
    written_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per order, updated in place as it fills or is cancelled.
CREATE TABLE IF NOT EXISTS orders (
    order_id   BIGINT PRIMARY KEY,
    user_id    UUID NOT NULL,
    symbol     TEXT NOT NULL,
    side       TEXT NOT NULL,
    order_type TEXT NOT NULL,
    -- NULL for a market order: it has no limit price.
    price      BIGINT,
    qty        BIGINT NOT NULL,
    filled_qty BIGINT NOT NULL DEFAULT 0,
    status     TEXT NOT NULL,
    -- The seq of the batch that last touched this row. An update carrying an
    -- older seq is refused rather than allowed to overwrite a newer one.
    last_seq   BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Money is integers and quantities are positive. Stated here so a bug that
    -- would write nonsense fails loudly at the boundary instead of becoming
    -- permanent history.
    CONSTRAINT orders_qty_positive CHECK (qty > 0),
    CONSTRAINT orders_filled_qty_sane CHECK (filled_qty >= 0 AND filled_qty <= qty),
    CONSTRAINT orders_price_positive CHECK (price IS NULL OR price > 0)
);

CREATE INDEX IF NOT EXISTS orders_user_idx ON orders (user_id, order_id DESC);
CREATE INDEX IF NOT EXISTS orders_symbol_idx ON orders (symbol, order_id DESC);

-- One row per match. Immutable: a fill that happened cannot un-happen.
--
-- Keyed by (seq, idx) rather than a synthetic id, so re-inserting the same fill
-- collides instead of duplicating even if the batch guard above were bypassed.
CREATE TABLE IF NOT EXISTS fills (
    seq            BIGINT NOT NULL,
    idx            INTEGER NOT NULL,
    symbol         TEXT NOT NULL,
    -- Always the maker's resting price. Price improvement belongs to the taker.
    price          BIGINT NOT NULL,
    qty            BIGINT NOT NULL,
    maker_order_id BIGINT NOT NULL,
    taker_order_id BIGINT NOT NULL,
    maker_user_id  UUID NOT NULL,
    taker_user_id  UUID NOT NULL,
    taker_side     TEXT NOT NULL,
    -- Quote atoms paid by the taker to the maker, before fees.
    notional       BIGINT NOT NULL,
    maker_fee      BIGINT NOT NULL,
    taker_fee      BIGINT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (seq, idx),
    CONSTRAINT fills_qty_positive CHECK (qty > 0),
    CONSTRAINT fills_price_positive CHECK (price > 0)
);

CREATE INDEX IF NOT EXISTS fills_symbol_idx ON fills (symbol, seq DESC, idx DESC);
CREATE INDEX IF NOT EXISTS fills_maker_user_idx ON fills (maker_user_id, seq DESC);
CREATE INDEX IF NOT EXISTS fills_taker_user_idx ON fills (taker_user_id, seq DESC);

-- Append-only ledger trail. One row per balance-affecting event, recording what
-- the event actually carried and nothing inferred.
CREATE TABLE IF NOT EXISTS balance_changes (
    seq        BIGINT NOT NULL,
    idx        INTEGER NOT NULL,
    user_id    UUID NOT NULL,
    asset      TEXT NOT NULL,
    available  BIGINT NOT NULL,
    -- NULL when the event carried only the available balance, which is the case
    -- for a deposit or a withdrawal.
    locked     BIGINT,
    -- 'deposit', 'withdrawal', or 'update' for a settlement-driven change.
    reason     TEXT NOT NULL,
    -- Signed change, positive for a credit. NULL when the event stated a
    -- resulting balance rather than a movement.
    delta      BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (seq, idx)
);

CREATE INDEX IF NOT EXISTS balance_changes_user_idx ON balance_changes (user_id, seq DESC);

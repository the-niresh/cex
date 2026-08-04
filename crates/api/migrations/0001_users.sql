-- The only table on a request path. Everything else Postgres holds is history,
-- written behind the engine by the persister.
CREATE TABLE IF NOT EXISTS users (
    id            UUID PRIMARY KEY,
    username      TEXT NOT NULL,
    -- Argon2 PHC string: algorithm, parameters, salt and hash in one field.
    -- Never the password.
    password_hash TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Usernames are compared case-insensitively, so uniqueness must be too.
-- Without this, "Alice" and "alice" are two accounts that look like one.
CREATE UNIQUE INDEX IF NOT EXISTS users_username_lower_key
    ON users (lower(username));

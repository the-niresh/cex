-- Guests are created by POST /guest with a random name and no password the
-- caller ever sees. The flag lets the API tell them apart from registered
-- accounts and supports cleanup later.
ALTER TABLE users ADD COLUMN IF NOT EXISTS is_guest BOOLEAN NOT NULL DEFAULT false;

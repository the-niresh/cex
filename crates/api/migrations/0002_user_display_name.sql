-- The name a user gives at registration, shown back to them in the UI.
--
-- Nullable on purpose, and permanently so: rows written before this column
-- existed have no name and are still perfectly valid accounts. Nothing about
-- authentication reads it — it is a label, never an identifier, and uniqueness
-- is still the username's job alone.
ALTER TABLE users ADD COLUMN IF NOT EXISTS display_name TEXT;

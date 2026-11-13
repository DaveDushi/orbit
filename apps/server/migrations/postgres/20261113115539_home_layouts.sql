-- Postgres: the same change as apps/server/migrations/20261113115539_home_layouts.sql
CREATE TABLE home_layouts (
    user_id TEXT PRIMARY KEY REFERENCES users(id),
    cards_json TEXT NOT NULL,
    updated_at BIGINT NOT NULL
);

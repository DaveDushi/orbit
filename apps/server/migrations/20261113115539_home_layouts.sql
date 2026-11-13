-- SQLite
-- Each person's Home cards: one layout per user, shared by the web and the iPhone app. `cards_json` is the
-- ordered list of cards (`[{"id": "inbox", "limit": 7}, …]`); the server drops unknown or not-allowed ids.
CREATE TABLE home_layouts (
    user_id TEXT PRIMARY KEY REFERENCES users(id),
    cards_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

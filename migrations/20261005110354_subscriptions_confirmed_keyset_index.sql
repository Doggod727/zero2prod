-- Add migration script here
CREATE INDEX subscriptions_confirmed_keyset_idx
    ON subscriptions (status, subscribed_at DESC, id DESC);
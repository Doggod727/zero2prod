-- Add migration script here
CREATE INDEX next_retry_at_index ON email_delivery_queue(next_retry_at);
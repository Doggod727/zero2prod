-- Add migration script here
CREATE UNIQUE INDEX email_delivery_queue_confirmation_unique ON email_delivery_queue(recipient)
    WHERE task_type = 'confirmation';

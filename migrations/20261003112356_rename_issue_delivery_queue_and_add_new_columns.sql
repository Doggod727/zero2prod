-- Add migration script here
DROP TABLE issue_delivery_queue;
CREATE TABLE email_delivery_queue (
    delivery_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    task_type text NOT NULL,  -- 'newsletter' | 'confirmation'
    recipient text NOT NULL, -- '接收者'

    newsletter_issue_id uuid REFERENCES newsletter_issues(newsletter_issue_id),
    subscription_token text,

    attempts int NOT NULL DEFAULT 0,
    next_retry_at timestamptz NOT NULL DEFAULT now(),
    last_error text,

    CONSTRAINT email_delivery_queue_payload_check CHECK (
        (task_type = 'newsletter' AND newsletter_issue_id IS NOT NULL)
        OR (task_type = 'confirmation' AND subscription_token IS NOT NULL)
        ),
    CONSTRAINT email_delivery_queue_task_type_check CHECK (
        task_type = 'newsletter' OR task_type = 'confirmation'
        ),
    CONSTRAINT email_delivery_queue_unique_delivery UNIQUE (
        newsletter_issue_id, recipient
        )
);

CREATE TABLE dead_letter_queue(
    delivery_id uuid PRIMARY KEY ,
    task_type text NOT NULL,
    recipient text NOT NULL,

    newsletter_issue_id uuid,
    subscription_token text,

    attempts int NOT NULL,
    last_error text NOT NULL,
    died_at timestamptz NOT NULL DEFAULT now(),

    resolved_at timestamptz,
    resolution_note text
);
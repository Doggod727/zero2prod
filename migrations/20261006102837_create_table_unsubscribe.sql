-- Add migration script here
ALTER TABLE subscriptions DROP CONSTRAINT IF EXISTS subscriptions_status_check;
ALTER TABLE subscriptions ADD CONSTRAINT subscriptions_status_check
    CHECK ( status IN ('confirmed', 'pending_confirmation', 'unsubscribed') );

-- 创建退订表
CREATE TABLE IF NOT EXISTS unsubscription_tokens (
    unsubscription_token text PRIMARY KEY,
    subscriber_id uuid NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS unsubscription_tokens_subscriber_id_unique
    ON unsubscription_tokens (subscriber_id);

ALTER TABLE email_delivery_queue DROP CONSTRAINT IF EXISTS email_delivery_queue_task_type_check;
ALTER TABLE email_delivery_queue ADD CONSTRAINT email_delivery_queue_task_type_check
    CHECK ( task_type IN ('newsletter', 'confirmation', 'unsubscription') );

-- ⚠️ 光改 task_type 不够，payload 约束也必须改 —— 这一点我一开始判断错了。
--
-- 原来的约束是：
--     (task_type = 'newsletter'   AND newsletter_issue_id IS NOT NULL)
--     OR (task_type = 'confirmation' AND subscription_token  IS NOT NULL)
-- 它【没有】unsubscription 这一支，所以任何 unsubscription 行都过不了 ——
-- 表现是插入退订通知时报 500：
--     new row for relation "email_delivery_queue"
--     violates check constraint "email_delivery_queue_payload_check"
-- 这个错误在日志里很清楚，但很容易先入为主地以为"task_type 放开了就行"。
--
-- 注意 IF NOT EXISTS 在这里【不能】用（ADD CONSTRAINT 不支持它），
-- 所以保持 DROP IF EXISTS + ADD 的组合来保证可重复执行。
ALTER TABLE email_delivery_queue DROP CONSTRAINT IF EXISTS email_delivery_queue_payload_check;
ALTER TABLE email_delivery_queue ADD CONSTRAINT email_delivery_queue_payload_check
    CHECK (
        (task_type = 'newsletter' AND newsletter_issue_id IS NOT NULL)
        OR (task_type IN ('confirmation', 'unsubscription') AND subscription_token IS NOT NULL)
    );

CREATE UNIQUE INDEX IF NOT EXISTS email_delivery_queue_pending_notification_unique
    ON email_delivery_queue(recipient)
    WHERE task_type IN ('confirmation', 'unsubscription');
ALTER TABLE email_delivery_queue
    DROP CONSTRAINT IF EXISTS email_delivery_queue_unique_delivery;
DROP INDEX IF EXISTS email_delivery_queue_confirmation_unique;
-- Add migration script here
BEGIN;
    -- 为历史记录回填'status'
    UPDATE subscriptions
    SET status = 'confirmed'
    WHERE status IS NULL;
    -- 修改status其不为空
    ALTER TABLE subscriptions
        ALTER COLUMN status SET NOT NULL;
COMMIT;
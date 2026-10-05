-- 收窄 status 的合法值域。
-- 说明：这个约束在本地库里曾被手工执行过一次（约束已存在但迁移未记录），
-- 因此这里用 IF NOT EXISTS 的写法保证可重复执行，避免 "already exists" 卡住后续迁移。
ALTER TABLE subscriptions
    DROP CONSTRAINT IF EXISTS subscriptions_status_check;

ALTER TABLE subscriptions
    ADD CONSTRAINT subscriptions_status_check
    CHECK (status IN ('confirmed', 'pending_confirmation'));
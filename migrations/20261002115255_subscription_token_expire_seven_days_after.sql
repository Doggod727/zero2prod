-- 为确认 token 增加 7 天有效期
-- 注意：此列在部分环境已手工存在，故用 IF NOT EXISTS 保证可重复执行
ALTER TABLE subscription_tokens
    ADD COLUMN IF NOT EXISTS expire_at timestamptz;

-- 回填历史行（已存在的行给一个"从现在起 7 天"的宽限）
UPDATE subscription_tokens
SET expire_at = now() + interval '7 days'
WHERE expire_at IS NULL;

-- 之后新插入的行若未显式给值，默认 7 天后过期
ALTER TABLE subscription_tokens
    ALTER COLUMN expire_at SET DEFAULT (now() + interval '7 days');

ALTER TABLE subscription_tokens
    ALTER COLUMN expire_at SET NOT NULL;
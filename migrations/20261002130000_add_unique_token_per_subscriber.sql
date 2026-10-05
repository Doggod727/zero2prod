-- 一个订阅者只能有一行确认 token —— 用唯一索引把这条不变量写成数据库约束。
--
-- 为什么必须有它：
--   1) store_token 是 INSERT ... ON CONFLICT (subscriber_id) DO UPDATE，
--      ON CONFLICT 要求 subscriber_id 上存在唯一约束/索引，否则 PostgreSQL 直接报错；
--      全新库（例如每个测试用例的库）从零跑迁移时，缺了这个索引订阅就会 500。
--   2) 并发首次订阅时"先查后插"存在 check-then-act 竞态：两个事务都会看到"不存在"
--      然后双双 INSERT。唯一索引让第二个 INSERT 直接报错，而不是留下脏数据。
--
-- 用 DO 块而不是 CREATE INDEX IF NOT EXISTS：
--   后者在索引已存在时会发 NOTICE，sqlx 在 ON_ERROR_STOP 下会把它当成错误而中断迁移。
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_class WHERE relname = 'subscription_tokens_subscriber_id_unique'
    ) THEN
        CREATE UNIQUE INDEX subscription_tokens_subscriber_id_unique
            ON subscription_tokens (subscriber_id);
    END IF;
END $$;
-- 给【已经存在的】confirmed 订阅者补一个退订 token。
--
-- 为什么要补：
--   退订 token 是这次新加的东西，在它出现之前就已经确认的订阅者一条都没有。
--   而发 newsletter 时要用它拼 List-Unsubscribe 头和正文里的退订链接：
--
--       INSERT INTO email_delivery_queue(task_type, recipient, newsletter_issue_id)
--       SELECT 'newsletter', s.email, $1
--       FROM subscriptions s
--       JOIN unsubscription_tokens t ON t.subscriber_id = s.id   ← INNER JOIN
--       WHERE s.status = 'confirmed'
--
--   ⚠️ 用 INNER JOIN 的话，没有 token 的订阅者会被【静默过滤掉】——
--      表现是"这批人从此再也收不到 newsletter"，而且没有任何报错。
--      所以这行 backfill 是 INNER JOIN 能成立的前提，缺了它就是个静默丢人的 bug。
-- ⚠️ 这里踩过一个坑，记下来：
--
--   (SELECT string_agg(substr('<62 个字符>', (floor(random()*62)+1)::int, 1), '')
--    FROM generate_series(1, 25))
--
-- 这个"看着很随机"的表达式，在 INSERT ... SELECT 里【每一行都会得到同一个 token】——
-- 因为它没有引用外层查询的任何列，规划器把那个子查询当成了不变式子查询，
-- 只求值一次然后重复使用。实测：6 行输出 6 个完全相同的 token，
-- 结果就是 unsubscription_tokens_pkey 冲突，迁移直接失败。
--
-- 用 gen_random_uuid() 取而代之：它是 volatile 函数，必须逐行求值；
-- 而且这句 SQL 的随机性直接落在"每行不同"上，不需要额外解释。
-- 长度 32（UUID 去掉连字符后是 32 位十六进制），仍然是纯 alnum，
-- 所以 unsubscribe_url 里那个 debug_assert 的前提依然成立。
CREATE TEMP TABLE backfilled_unsubscription_tokens AS
SELECT
    replace(gen_random_uuid()::text, '-', '') AS unsubscription_token,
    id AS subscriber_id
FROM subscriptions
WHERE status = 'confirmed';

INSERT INTO unsubscription_tokens (unsubscription_token, subscriber_id)
SELECT unsubscription_token, subscriber_id
FROM backfilled_unsubscription_tokens
ON CONFLICT (subscriber_id) DO NOTHING;   -- 目标只有 subscriber_id 这一个约束；
                                          -- token 的 pkey 由 gen_random_uuid 保证不撞

DROP TABLE backfilled_unsubscription_tokens;
-- 最后自检：还有 confirmed 但没 token 的人吗？
-- 有就让这个迁移【失败】。
--
-- 为什么要一个"主动失败"的检查，而不是默默过去：
--   下面 enqueue_delivery_tasks 用的是 INNER JOIN，
--   漏掉的人不会收到 newsletter、也不会报错 —— 这是最难查的一类 bug。
--   与其让它变成线上事故，不如让迁移在部署时当场炸掉。
--   （CHECK 约束在 ALTER TABLE 时会全表验证，所以这里放弃用 VARCHAR + DO 的写法，
--     改成一条明确的断言。）
DO $$
DECLARE
    missing integer;
BEGIN
    SELECT count(*) INTO missing
    FROM subscriptions s
    LEFT JOIN unsubscription_tokens t ON t.subscriber_id = s.id
    WHERE s.status = 'confirmed' AND t.subscriber_id IS NULL;

    IF missing > 0 THEN
        RAISE EXCEPTION
            '还有 % 个 confirmed 订阅者没有退订 token；newsletter 的 INNER JOIN 会静默漏掉他们',
            missing;
    END IF;
END $$;
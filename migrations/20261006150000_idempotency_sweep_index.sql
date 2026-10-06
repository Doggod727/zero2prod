-- 幂等键的过期清扫需要一条能按 create_at 定位候选行的索引。
--
-- 主键是 (user_id, idempotency_key)，清扫条件却是 create_at —— 两者没有公共前缀，
-- 所以现在清扫只能全表扫描。表越大越糟，而这张表是【只增不减】的
-- （在加上 TTL 清扫之前，每一期 newsletter 都留一行，永远不删）。
--
-- 部分索引 WHERE response_body IS NULL 反过来写会更省空间
-- （已完成的行才是多数），但那样索引就没法服务
-- 「DELETE ... WHERE response_body IS NOT NULL AND create_at < $1」这个主要用途。
-- 所以这里不建部分索引，就建普通索引：清扫是按 create_at 切一刀，
-- 与响应体是否为空无关（两种行都要能被它定位，只是删除条件不同）。
CREATE INDEX IF NOT EXISTS idempotency_create_at_index
    ON idempotency (create_at);
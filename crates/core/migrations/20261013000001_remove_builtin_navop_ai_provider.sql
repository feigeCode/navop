-- 内置 Navop AI 供应商（provider_type = 'onet_cli'）已下线：客户端不再提供云端 AI 代理能力。
--
-- 必须显式清理历史行，否则 `ProviderType::from_str` 不再识别 `onet_cli` 之后，
-- 反序列化会在读到该行时报 "Invalid provider type"，导致供应商列表整体加载失败。
DELETE FROM llm_providers WHERE provider_type = 'onet_cli';

-- 被删除的若原本是默认供应商，则让最早创建的剩余供应商接管默认，
-- 避免升级后没有任何默认模型可选。
UPDATE llm_providers
SET is_default = 1
WHERE id = (
    SELECT id FROM llm_providers
    ORDER BY created_at ASC, id ASC
    LIMIT 1
)
AND NOT EXISTS (SELECT 1 FROM llm_providers WHERE is_default = 1);

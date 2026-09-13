BEGIN;

-- 旧内置 MQTT/RocketMQ 连接一次性迁移为扩展连接。
--
-- 背景:MQTT/RocketMQ 内置实现已整体移植到 navop-extensions 扩展子模块
-- (com.navop.middleware.mqtt / com.navop.middleware.rocketmq),主仓不再有
-- 这两类连接的原生打开路径;存量行需改写为 ConnectionType='Extension' 并把
-- params 重组为 ExtensionConnectionParams 形态(schema_version=1)。
--
-- 字段名以扩展侧 extension.json 的 contributes.connections[].form 为准:
-- - MQTT:      config = {host, port, username, client_id, keep_alive_secs, ssh_tunnel?},
--              secrets = {password}(历史 ENC: 密文与 Extension secrets 解密路径
--              格式一致,原样搬运,无需 vault 解锁)。
-- - RocketMQ:  config = {namesrv_addrs(分号串), acl_enabled, access_key, timeout_ms,
--              ssh_tunnel?}, secrets = {secret_key}(历史明文,首次保存时由
--              try_encrypt_params 统一加密)。
-- - SSH 隧道:扩展连接当前不支持隧道(已知能力缺口),原样保留在 config 的
--   ssh_tunnel 惰性字段,provider 忽略;隧道内密文字段保持密文。
-- - credential_reference 惰性保留进 config,扩展路径不解析。
-- - 丢弃字段(扩展表单未提供):use_tls/connect_timeout/mqtt_version/clean_session
--   (MQTT)、domain/connect_timeout(RocketMQ)。
--
-- 兜底:云同步下行等写入路径由 Rust 侧
-- StoredConnection::try_migrate_legacy_middleware_connection 再次归一,
-- 防止旧形态数据重新落库。

UPDATE connections
SET connection_type = 'Extension',
    params = json_object(
        'schema_version', 1,
        'extension_id', 'com.navop.middleware.mqtt',
        'contribution_id', 'mqtt',
        'config', json_object(
            'host', coalesce(json_extract(params, '$.host'), '127.0.0.1'),
            'port', cast(coalesce(json_extract(params, '$.port'), 1883) AS INTEGER),
            'username', coalesce(json_extract(params, '$.username'), ''),
            'client_id', coalesce(json_extract(params, '$.client_id'), ''),
            -- 历史 keep_alive 为空表示运行时默认(30s),此处显式落为 30
            'keep_alive_secs', coalesce(json_extract(params, '$.keep_alive'), 30),
            'credential_reference', json_extract(params, '$.credential_reference'),
            'ssh_tunnel', json_extract(params, '$.ssh_tunnel')
        ),
        'secrets', CASE
            WHEN json_extract(params, '$.password') IS NOT NULL
                 AND json_extract(params, '$.password') != ''
            THEN json_object('password', json_extract(params, '$.password'))
            ELSE json_object()
        END
    )
WHERE connection_type = 'Mqtt';

UPDATE connections
SET connection_type = 'Extension',
    params = json_object(
        'schema_version', 1,
        'extension_id', 'com.navop.middleware.rocketmq',
        'contribution_id', 'rocketmq',
        'config', json_object(
            'namesrv_addrs', coalesce(
                (SELECT group_concat(je.value, ';')
                 FROM json_each(connections.params, '$.namesrv_addrs') AS je),
                '127.0.0.1:9876'
            ),
            'acl_enabled', CASE
                WHEN coalesce(json_extract(params, '$.access_key'), '') != ''
                     OR coalesce(json_extract(params, '$.secret_key'), '') != ''
                THEN 'rocketmq'
                ELSE 'none'
            END,
            'access_key', json_extract(params, '$.access_key'),
            'timeout_ms', json_extract(params, '$.request_timeout'),
            'credential_reference', json_extract(params, '$.credential_reference'),
            'ssh_tunnel', json_extract(params, '$.ssh_tunnel')
        ),
        'secrets', CASE
            WHEN json_extract(params, '$.secret_key') IS NOT NULL
                 AND json_extract(params, '$.secret_key') != ''
            THEN json_object('secret_key', json_extract(params, '$.secret_key'))
            ELSE json_object()
        END
    )
WHERE connection_type = 'Rocketmq';

COMMIT;

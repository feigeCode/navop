//! RocketMQ 4.x ACL 请求签名。
//!
//! 算法对照官方 `acl/src/main/java/org/apache/rocketmq/acl/common/`
//! `AclClientRPCHook.java` / `AclUtils.java` / `AclSigner.java`(release-4.9.4):
//!
//! 1. extFields 中加入 `AccessKey`(可选 `SecurityToken`);
//! 2. 将 extFields 按键名字典序(TreeMap)排列,依次拼接全部 value
//!    (跳过 `Signature` 键自身);
//! 3. 追加请求 body 原始字节;
//! 4. 以 SecretKey 做 HMAC-SHA1 后 Base64 编码,写入 `Signature` 字段。
//!
//! 5.x 服务端通过 `AuthorizationCompatibility` 兼容该校验,签名规则一致。

use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;

use crate::protocol::RemotingCommand;

/// 扩展字段键名(官方 `SessionCredentials` 常量)
pub const ACCESS_KEY: &str = "AccessKey";
pub const SECRET_KEY: &str = "SecretKey";
pub const SIGNATURE: &str = "Signature";
pub const SECURITY_TOKEN: &str = "SecurityToken";

type HmacSha1 = Hmac<Sha1>;

/// ACL 凭据
#[derive(Clone, Debug)]
pub struct AclCredentials {
    /// AccessKey
    pub access_key: String,
    /// SecretKey
    pub secret_key: String,
    /// SecurityToken(可选)
    pub security_token: Option<String>,
}

impl AclCredentials {
    /// 从非空键值对构造
    pub fn new(
        access_key: &str,
        secret_key: &str,
        security_token: Option<String>,
    ) -> Result<Self, &'static str> {
        if access_key.trim().is_empty() || secret_key.trim().is_empty() {
            return Err("ACL AccessKey/SecretKey 不能为空");
        }
        Ok(Self {
            access_key: access_key.trim().to_string(),
            secret_key: secret_key.trim().to_string(),
            security_token: security_token
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty()),
        })
    }

    /// 计算待签名串(按官方 `AclUtils.combineRequestContent`)
    fn signing_payload(command: &RemotingCommand) -> Vec<u8> {
        // 按键名字典序拼接 value(跳过 Signature 自身)
        let mut keys: Vec<&String> = command.ext_fields.keys().collect();
        keys.sort();
        let mut payload = Vec::new();
        for key in keys {
            if key == SIGNATURE {
                continue;
            }
            payload.extend_from_slice(command.ext_fields[key].as_bytes());
        }
        if let Some(body) = command.body.as_deref() {
            payload.extend_from_slice(body);
        }
        payload
    }

    /// 对请求指令就地签名(注入 AccessKey/SecurityToken/Signature)
    pub fn sign_request(&self, command: &mut RemotingCommand) {
        command
            .ext_fields
            .entry(ACCESS_KEY.to_string())
            .or_insert_with(|| self.access_key.clone());
        if let Some(token) = &self.security_token {
            command
                .ext_fields
                .entry(SECURITY_TOKEN.to_string())
                .or_insert_with(|| token.clone());
        }
        let payload = Self::signing_payload(command);
        command.ext_fields.insert(
            SIGNATURE.to_string(),
            hmac_sha1_base64(&payload, &self.secret_key),
        );
    }
}

/// HMAC-SHA1 + Base64(官方 `AclSigner.calSignature` 等价实现)
pub fn hmac_sha1_base64(payload: &[u8], secret_key: &str) -> String {
    let mut mac =
        HmacSha1::new_from_slice(secret_key.as_bytes()).expect("HMAC-SHA1 支持任意长度密钥");
    mac.update(payload);
    let digest = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// HMAC-SHA1 基准向量(独立于实现,由 openssl 生成):
    /// `printf '12akx' | openssl dgst -sha1 -hmac 'sk' -binary | base64`
    #[test]
    fn hmac_matches_openssl_vector() {
        assert_eq!(
            hmac_sha1_base64(b"12akx", "sk"),
            "ZDIFg77q2v1wqmlx+I4wqaY8mLQ="
        );
        // 空 payload 向量
        assert_eq!(
            hmac_sha1_base64(b"", "secret"),
            "Ja9hdKD87MTTRmgKcrfOZEuaiOg="
        );
    }

    /// 签名流程:字典序拼接 + body 追加 + 字段注入
    #[test]
    fn sign_request_follows_official_order() {
        let creds = AclCredentials::new("ak", "sk", Some("token".into())).unwrap();
        let mut ext = HashMap::new();
        ext.insert("b".to_string(), "2".to_string());
        ext.insert("a".to_string(), "1".to_string());
        ext.insert("topic".to_string(), "order".to_string());
        let mut command =
            RemotingCommand::create_request(crate::protocol::RequestCode::QUERY_MESSAGE, ext)
                .with_body(b"x".to_vec());

        creds.sign_request(&mut command);

        // AccessKey/SecurityToken 已注入
        assert_eq!(command.ext_fields[ACCESS_KEY], "ak");
        assert_eq!(command.ext_fields[SECURITY_TOKEN], "token");
        // 待签名串 = "ak" + "token" + "1" + "2" + "order" + body "x"
        //   (键字节序: AccessKey < SecurityToken < a < b < topic,
        //    与 Java TreeMap 的 String.compareTo 自然序一致:大写字母 < 小写字母)
        let expected_payload = b"aktoken12orderx".to_vec();
        let expected = hmac_sha1_base64(&expected_payload, "sk");
        assert_eq!(command.ext_fields[SIGNATURE], expected);
    }

    /// 凭据校验:空键报错
    #[test]
    fn rejects_empty_credentials() {
        assert!(AclCredentials::new("", "sk", None).is_err());
        assert!(AclCredentials::new("ak", " ", None).is_err());
        assert!(AclCredentials::new("ak", "sk", None).is_ok());
    }

    /// 无 body 请求签名(管理命令常态)
    #[test]
    fn sign_request_without_body() {
        let creds = AclCredentials::new("ak", "sk", None).unwrap();
        let mut command = RemotingCommand::create_request(
            crate::protocol::RequestCode::GET_BROKER_CLUSTER_INFO,
            HashMap::new(),
        );
        creds.sign_request(&mut command);
        // payload = "ak"
        assert_eq!(command.ext_fields[SIGNATURE], hmac_sha1_base64(b"ak", "sk"));
    }
}

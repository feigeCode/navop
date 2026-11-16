//! 回归：部分老设备主机公钥里的非规范 mpint 必须能被解析。
//!
//! 华为 VRP 系等老设备的 `ssh-rsa` 主机公钥有两种非规范 mpint：
//! 1. e / n 带多余的前导 `0x00`；
//! 2. 数值最高位为 1 时省略了必需的 `0x00` 符号位（看起来是负数）。
//! RFC 4251 § 5 两种都不允许，但 OpenSSH 风格的客户端读取对端报文时按无符号
//! 数值处理（`sshbuf_get_bignum2_bytes_direct`），所以通过 `[patch.crates-io]`
//! 挂的 ssh-encoding fork 也做归一化。
//!
//! 现象：不开这个 patch 时，算法协商能过，随后解析主机公钥失败，用户看到
//! `Connection Lost` + ``SshKey: `mpint` encoding invalid``（连打 2~3 遍，是
//! anyhow `{:#}` 把错误链打平）。
//!
//! 本测试锁住「patch 仍然生效」：一旦 ssh-encoding 回到 crates.io 的严格版本
//! （或 fork tag 被摘掉），这里会失败，而不是等用户报障。

use russh::keys::ssh_key::{self, PublicKey};

/// 按 RFC 4251 组装 `ssh-rsa` 公钥 blob。`e` / `n` 是裸 mpint 载荷（不含长度前缀）。
fn rsa_blob(e: &[u8], n: &[u8]) -> Vec<u8> {
    /// 写入 `string` 编码：4 字节大端长度 + 原始字节。
    fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
        let len = u32::try_from(bytes.len()).expect("test payload fits in u32");
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(bytes);
    }

    let mut out = Vec::new();
    put_string(&mut out, b"ssh-rsa");
    put_string(&mut out, e);
    put_string(&mut out, n);
    out
}

/// e = 0x010001、n = 0x8001 的规范编码。
fn canonical_blob() -> Vec<u8> {
    rsa_blob(&[0x01, 0x00, 0x01], &[0x00, 0x80, 0x01])
}

/// 同样的值，但 e / n 各带了多余的前导 `0x00`（老设备的实际行为）。
fn non_canonical_blob() -> Vec<u8> {
    rsa_blob(&[0x00, 0x01, 0x00, 0x01], &[0x00, 0x00, 0x80, 0x01])
}

/// russh 在 KEX 阶段解析服务端主机公钥，走的就是 `ssh_key` 的 mpint 解码路径
/// （russh 内部的 `parse_public_key` 即 `KeyData::decode` + `into()`，
/// 与这里的公开入口 `PublicKey::from_bytes` 等价）。
#[test]
fn non_canonical_host_key_is_accepted() {
    let key = PublicKey::from_bytes(&non_canonical_blob())
        .expect("非规范 mpint 主机公钥应被接受（ssh-encoding fork 生效）");

    let expected = PublicKey::from_bytes(&canonical_blob()).expect("规范编码本身应可解析");
    assert_eq!(key, expected, "归一化后的公钥应与规范编码逐字节等价");
}

/// 前导零被裁掉而不是被当成值的一部分：e / n 的数值必须保持不变。
#[test]
fn non_canonical_host_key_keeps_its_value() {
    let key = PublicKey::from_bytes(&non_canonical_blob()).expect("非规范 mpint 应被接受");

    let ssh_key::public::KeyData::Rsa(rsa) = key.key_data() else {
        panic!("期望解析出 RSA 公钥，实际为 {:?}", key.algorithm());
    };

    assert_eq!(rsa.e().as_positive_bytes().unwrap(), &[0x01, 0x00, 0x01]);
    assert_eq!(rsa.n().as_positive_bytes().unwrap(), &[0x80, 0x01]);
}

/// 规范编码不受影响（回归保护）。
#[test]
fn canonical_host_key_still_parses() {
    let key = PublicKey::from_bytes(&canonical_blob()).expect("规范编码应可解析");
    let ssh_key::public::KeyData::Rsa(rsa) = key.key_data() else {
        panic!("期望解析出 RSA 公钥，实际为 {:?}", key.algorithm());
    };

    assert_eq!(rsa.e().as_positive_bytes().unwrap(), &[0x01, 0x00, 0x01]);
    assert_eq!(rsa.n().as_positive_bytes().unwrap(), &[0x80, 0x01]);
}

/// 另一类老设备问题：数值最高位为 1 时本该补 `0x00` 符号位，设备却省略了，
/// 于是首字节 `>= 0x80`、解码时看起来是负数。这种编码同样要按无符号数值接受。
#[test]
fn missing_sign_padding_is_accepted() {
    // n = 0x8001，未补 0x00。
    let key = PublicKey::from_bytes(&rsa_blob(&[0x01, 0x00, 0x01], &[0x80, 0x01]))
        .expect("缺符号位的老设备 mpint 应被接受");

    let expected = PublicKey::from_bytes(&canonical_blob()).expect("规范编码本身应可解析");
    assert_eq!(key, expected, "按无符号数值归一化后应与规范编码等价");

    let ssh_key::public::KeyData::Rsa(rsa) = key.key_data() else {
        panic!("期望解析出 RSA 公钥，实际为 {:?}", key.algorithm());
    };
    assert_eq!(rsa.n().as_positive_bytes().unwrap(), &[0x80, 0x01]);
}

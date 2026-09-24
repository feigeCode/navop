//! 回归：部分老设备主机公钥里的非规范 mpint 必须能被解析。
//!
//! 华为 VRP 系等老设备会在 `ssh-rsa` 主机公钥的 e / n 里带上多余的前导 `0x00`。
//! RFC 4251 § 5 要求省略这种前导零，OpenSSH 在**读取对端报文**时会裁掉它而不是
//! 报错（`sshbuf_get_bignum2_bytes_direct`），所以我们通过 `[patch.crates-io]`
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
    assert_eq!(
        key, expected,
        "归一化后的公钥应与规范编码逐字节等价"
    );
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

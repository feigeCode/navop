use crate::error::Error;
use bytes::Buf;

/// TODO: it is theoretically possible to ensure size in chunks,
///  at least for some types, given that we have the database schema.
#[inline]
pub(crate) fn ensure_size(buffer: impl Buf, size: usize) -> crate::error::Result<()> {
    if buffer.remaining() < size {
        Err(Error::NotEnoughData)
    } else {
        Ok(())
    }
}

#[inline]
pub(crate) fn get_unsigned_leb128(mut buffer: impl Buf) -> crate::error::Result<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;

    loop {
        ensure_size(&mut buffer, 1)?;

        let byte = buffer.get_u8();
        let payload = byte & 0x7f;
        if shift == 63 && (payload > 1 || byte & 0x80 != 0) {
            return Err(Error::Custom(
                "decoding LEB128, value overflows u64".to_string(),
            ));
        }
        value |= (payload as u64) << shift;

        if byte & 0x80 == 0 {
            break;
        }

        shift += 7;
    }

    Ok(value)
}

#[test]
fn it_deserializes_unsigned_leb128() {
    let buf = &[0xe5, 0x8e, 0x26][..];
    assert_eq!(get_unsigned_leb128(buf).unwrap(), 624_485);
}

#[test]
fn it_deserializes_unsigned_leb128_boundaries() {
    let mut max_encoding = vec![0xFFu8; 9];
    max_encoding.push(0x01);
    let mut max_minus_one_encoding = max_encoding.clone();
    max_minus_one_encoding[0] = 0xFE;

    assert_eq!(
        get_unsigned_leb128(max_minus_one_encoding.as_slice()).unwrap(),
        u64::MAX - 1
    );
    assert_eq!(
        get_unsigned_leb128(max_encoding.as_slice()).unwrap(),
        u64::MAX
    );
}

#[test]
fn it_rejects_unsigned_leb128_overflow() {
    for last_byte in [0x02, 0x80] {
        let mut buf = vec![0xFFu8; 9];
        buf.push(last_byte);

        assert!(matches!(
            get_unsigned_leb128(buf.as_slice()),
            Err(Error::Custom(_))
        ));
    }
}

#[test]
fn it_rejects_truncated_unsigned_leb128() {
    let buf = vec![0xFFu8; 9];

    assert!(matches!(
        get_unsigned_leb128(buf.as_slice()),
        Err(Error::NotEnoughData)
    ));
}

use bytes::{Buf, BufMut};
use std::ops::ControlFlow;

/// Write a `usize` as a native `VarUInt`
#[allow(clippy::cast_possible_truncation)] // truncation is intentional here
pub(super) fn write(mut buf: impl BufMut, mut uint: usize) {
    while uint > 0x7F {
        let b = (uint as u8) | 0x80;
        buf.put_u8(b);
        uint >>= 7;
    }

    buf.put_u8(uint as u8);
}

#[derive(Default)]
pub(super) struct ParseVarUInt {
    accumulator: u64,
    shift: u32,
}

#[derive(Debug, thiserror::Error)]
#[cfg_attr(test, derive(PartialEq, Eq))]
#[error("VarUInt repr overflowed: {accumulator:#x} byte: {byte:#02x}")]
pub(super) struct VarUIntOverflowError {
    accumulator: u64,
    byte: u8,
}

impl ParseVarUInt {
    pub(super) fn feed(
        &mut self,
        mut buf: impl Buf,
    ) -> Result<ControlFlow<u64>, VarUIntOverflowError> {
        // This loop is guaranteed to terminate:
        // if `self.shift` is >= 63, the next byte _must_ be a terminating byte
        // or else an error is returned.
        loop {
            let Ok(b) = buf.try_get_u8() else {
                return Ok(ControlFlow::Continue(()));
            };

            // The tenth byte may only have one bit
            // `.checked_shl()` doesn't check for overflow, just that the shift is in-bounds
            // This could be replaced by `.shl_exact()` when stable:
            // https://doc.rust-lang.org/stable/std/primitive.u64.html#method.shl_exact
            if self.shift >= 63 && b > 1 {
                return Err(VarUIntOverflowError {
                    accumulator: self.accumulator,
                    byte: b,
                })?;
            }

            self.accumulator |= ((b & 0x7F) as u64) << self.shift;

            if b <= 0x7F {
                return Ok(ControlFlow::Break(self.accumulator));
            }

            self.shift += 7;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::native::varuint::{ParseVarUInt, VarUIntOverflowError};
    use bytes::Buf;
    use std::ops::ControlFlow;
    const ENCODED_AND_DECODED: &[(&[u8], u64)] = &[
        (&[0], 0u64),
        (&[1], 1),
        (&[127], 127),
        (&[0x80, 0x01], 1 << 7),
        (&[0x80, 0x80, 0x01], 1 << 14),
        (&[0x80, 0x80, 0x80, 0x01], 1 << 21),
        (&[0x80, 0x80, 0x80, 0x80, 0x01], 1 << 28),
        (&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x1F], 0xFF_FF_FF_FF_FF),
        (
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F],
            0x7F_FF_FF_FF_FF_FF_FF_FF,
        ),
        (
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x80, 0x01],
            0x80_FF_FF_FF_FF_FF_FF_FF,
        ),
        // Max 64-bit value
        (
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01],
            0xFF_FF_FF_FF_FF_FF_FF_FF,
        ),
    ];

    const PAD_LEN: usize = 16;

    fn pad(bytes: &[u8], len: usize) -> Vec<u8> {
        bytes
            .iter()
            .copied()
            // A recognizable but non-constant byte pattern
            // This padding is also guaranteed to overflow
            // because it takes 128 iterations to reach `0x7F`, the first terminating byte
            .chain((0..=255).rev().cycle())
            .take(len)
            .collect()
    }

    #[test]
    fn parse_varuint() {
        for (encoded, decoded) in ENCODED_AND_DECODED {
            let padded = pad(encoded, PAD_LEN);

            // Test feeding slices in different size chunks
            for chunk_size in 1..=padded.len() {
                let mut parser = ParseVarUInt::default();

                let mut slice = &padded[..];

                let mut last_remaining = slice.len();

                loop {
                    match parser.feed((&mut slice).take(chunk_size)) {
                        Ok(ControlFlow::Break(res)) => {
                            assert_eq!(
                                res, *decoded,
                                "invalid decoding; chunk_size: {chunk_size}, padded: {padded:?}, remaining: {slice:?}"
                            );
                            assert_eq!(
                                slice.len(),
                                padded.len() - encoded.len(),
                                "extra data consumed: {slice:?}"
                            );
                            break;
                        }
                        Ok(ControlFlow::Continue(())) => {
                            assert!(
                                !slice.is_empty(),
                                "full slice consumed without giving a result"
                            );
                            assert_ne!(
                                slice.len(),
                                last_remaining,
                                "parser failed to make progress"
                            );
                            last_remaining = slice.len();
                        }
                        Err(e) => {
                            panic!(
                                "error: {e:?}, chunk_size: {chunk_size}, padded: {padded:?}, remaining: {slice:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn parse_varuint_overflows() {
        let invalid_encodings_and_errors: &[(&[u8], VarUIntOverflowError)] = &[
            // One bit too large
            (
                &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02],
                VarUIntOverflowError {
                    accumulator: 0x7F_FF_FF_FF_FF_FF_FF_FF,
                    byte: 0x02,
                },
            ),
            // Invalid terminating byte
            (
                &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x80],
                VarUIntOverflowError {
                    accumulator: 0x7F_FF_FF_FF_FF_FF_FF_FF,
                    byte: 0x80,
                },
            ),
            // Whole extra byte
            (
                &[
                    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,
                ],
                VarUIntOverflowError {
                    accumulator: 0x7F_FF_FF_FF_FF_FF_FF_FF,
                    byte: 0xFF,
                },
            ),
        ];

        for (encoding, expected_err) in invalid_encodings_and_errors {
            // Test feeding slices in different size chunks
            for chunk_size in 1..=encoding.len() {
                let mut parser = ParseVarUInt::default();

                let mut slice = &encoding[..];
                let mut last_remaining = slice.len();

                loop {
                    match parser.feed((&mut slice).take(chunk_size)) {
                        Ok(ControlFlow::Break(res)) => {
                            unreachable!(
                                "encoding should be invalid but gave {res} (chunk size {chunk_size}): {encoding:?}"
                            )
                        }
                        Ok(ControlFlow::Continue(())) => {
                            assert!(
                                !slice.is_empty(),
                                "full slice consumed without giving a result"
                            );
                            assert_ne!(
                                slice.len(),
                                last_remaining,
                                "parser failed to make progress"
                            );
                            last_remaining = slice.len();
                        }
                        Err(e) => {
                            assert_eq!(
                                e, *expected_err,
                                "got wrong error for encoding (chunk size {chunk_size}): {encoding:?}"
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn write_varuint() {
        let mut buf = Vec::with_capacity(10);

        for (encoded, decoded) in ENCODED_AND_DECODED {
            buf.clear();

            super::write(&mut buf, (*decoded).try_into().unwrap());

            assert_eq!(*buf, **encoded, "decoded: {decoded}");
        }
    }
}

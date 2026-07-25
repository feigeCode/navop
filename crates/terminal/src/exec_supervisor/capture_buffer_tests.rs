use super::capture_buffer::BoundedCaptureBuffer;

#[test]
fn capture_buffer_never_exceeds_limit_and_keeps_newest_tail() {
    let mut buffer = BoundedCaptureBuffer::new(4);

    buffer.extend_from_slice(b"ab");
    buffer.extend_from_slice(b"cde");

    assert_eq!(buffer.len(), 4);
    assert_eq!(buffer.to_vec(), b"bcde");
}

#[test]
fn capture_chunk_larger_than_limit_replaces_older_bytes() {
    let mut buffer = BoundedCaptureBuffer::new(4);
    buffer.extend_from_slice(b"old");

    buffer.extend_from_slice(b"123456");

    assert_eq!(buffer.len(), 4);
    assert_eq!(buffer.to_vec(), b"3456");
}

#[test]
fn capture_buffer_preserves_order_without_truncation() {
    let mut buffer = BoundedCaptureBuffer::new(8);

    buffer.extend_from_slice(b"ab");
    buffer.extend_from_slice(b"cd");

    assert_eq!(buffer.to_vec(), b"abcd");
}

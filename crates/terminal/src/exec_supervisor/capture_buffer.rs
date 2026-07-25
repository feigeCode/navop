use std::collections::VecDeque;

pub(super) struct BoundedCaptureBuffer {
    bytes: VecDeque<u8>,
    limit: usize,
}

impl BoundedCaptureBuffer {
    pub(super) fn new(limit: usize) -> Self {
        assert!(limit > 0, "capture buffer limit must be positive");
        Self {
            bytes: VecDeque::with_capacity(limit),
            limit,
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(super) fn extend_from_slice(&mut self, data: &[u8]) {
        if data.len() >= self.limit {
            self.bytes.clear();
            self.bytes.extend(
                data[data.len().saturating_sub(self.limit)..]
                    .iter()
                    .copied(),
            );
            return;
        }

        let overflow = self
            .bytes
            .len()
            .saturating_add(data.len())
            .saturating_sub(self.limit);

        self.bytes.drain(..overflow);
        self.bytes.extend(data.iter().copied());
    }

    pub(super) fn to_vec(&self) -> Vec<u8> {
        self.bytes.iter().copied().collect()
    }
}

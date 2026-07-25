use std::collections::VecDeque;

pub(super) struct BoundedCaptureBuffer {
    bytes: VecDeque<u8>,
    limit: usize,
    discarded_bytes: u64,
}

impl BoundedCaptureBuffer {
    pub(super) fn new(limit: usize) -> Self {
        assert!(limit > 0, "capture buffer limit must be positive");
        Self {
            bytes: VecDeque::with_capacity(limit),
            limit,
            discarded_bytes: 0,
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(super) fn captured_bytes(&self) -> usize {
        self.bytes.len()
    }

    pub(super) fn discarded_bytes(&self) -> u64 {
        self.discarded_bytes
    }

    pub(super) fn truncated(&self) -> bool {
        self.discarded_bytes > 0
    }

    pub(super) fn extend_from_slice(&mut self, data: &[u8]) {
        if data.len() >= self.limit {
            let discarded = self
                .bytes
                .len()
                .saturating_add(data.len().saturating_sub(self.limit));
            self.record_discarded(discarded);
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

        self.record_discarded(overflow);
        self.bytes.drain(..overflow);
        self.bytes.extend(data.iter().copied());
    }

    pub(super) fn to_vec(&self) -> Vec<u8> {
        self.bytes.iter().copied().collect()
    }

    fn record_discarded(&mut self, count: usize) {
        self.discarded_bytes = self
            .discarded_bytes
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    }
}

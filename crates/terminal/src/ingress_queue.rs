//! GPUI-independent terminal ingress with byte/chunk-bounded data and a
//! separately bounded control lane.
//! Last-sender close drains accepted items; abort and receiver drop discard them.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

mod types;

pub use types::*;

pub fn bounded_terminal_queue<C>(
    budget: TerminalIngressBudget,
) -> (BoundedTerminalSender<C>, BoundedTerminalReceiver<C>) {
    let (data_tx, data_rx) = mpsc::channel(budget.max_pending_chunks);
    let (control_tx, control_rx) = mpsc::channel(budget.max_pending_controls);
    let state = Arc::new(QueueState::new(budget.max_pending_bytes));
    (
        BoundedTerminalSender {
            data_tx,
            control_tx,
            state: state.clone(),
        },
        BoundedTerminalReceiver {
            data_rx,
            control_rx,
            state,
            data_closed: false,
            control_closed: false,
            aborted: false,
        },
    )
}

struct QueueState {
    max_pending_bytes: usize,
    byte_permits: Arc<Semaphore>,
    // Semaphore availability includes permits partially assigned to a waiter.
    // This counter changes only after one complete byte acquisition.
    pending_bytes: AtomicUsize,
    peak_pending_bytes: AtomicUsize,
    abort: CancellationToken,
}

impl QueueState {
    fn new(max_pending_bytes: u32) -> Self {
        let max_pending_bytes = max_pending_bytes as usize;
        Self {
            max_pending_bytes,
            byte_permits: Arc::new(Semaphore::new(max_pending_bytes)),
            pending_bytes: AtomicUsize::new(0),
            peak_pending_bytes: AtomicUsize::new(0),
            abort: CancellationToken::new(),
        }
    }

    fn abort(&self) {
        self.byte_permits.close();
        self.abort.cancel();
    }

    fn pending_bytes(&self) -> usize {
        self.pending_bytes.load(Ordering::Relaxed)
    }

    fn reserve_bytes(
        self: &Arc<Self>,
        byte_count: usize,
        permit: OwnedSemaphorePermit,
    ) -> ByteReservation {
        let previous = self.pending_bytes.fetch_add(byte_count, Ordering::Relaxed);
        let current = previous + byte_count;
        debug_assert!(current <= self.max_pending_bytes);
        self.peak_pending_bytes
            .fetch_max(current, Ordering::Relaxed);
        ByteReservation {
            byte_count,
            _permit: permit,
            state: self.clone(),
        }
    }

    fn peak_pending_bytes(&self) -> usize {
        self.peak_pending_bytes.load(Ordering::Relaxed)
    }
}

struct ByteReservation {
    byte_count: usize,
    _permit: OwnedSemaphorePermit,
    state: Arc<QueueState>,
}

impl Drop for ByteReservation {
    fn drop(&mut self) {
        // Linearize release before making the permits available to a new sender.
        let previous = self
            .state
            .pending_bytes
            .fetch_sub(self.byte_count, Ordering::Relaxed);
        debug_assert!(previous >= self.byte_count);
    }
}

pub struct BoundedTerminalSender<C> {
    data_tx: mpsc::Sender<QueuedData>,
    control_tx: mpsc::Sender<C>,
    state: Arc<QueueState>,
}

impl<C> Clone for BoundedTerminalSender<C> {
    fn clone(&self) -> Self {
        Self {
            data_tx: self.data_tx.clone(),
            control_tx: self.control_tx.clone(),
            state: self.state.clone(),
        }
    }
}

impl<C> BoundedTerminalSender<C> {
    pub async fn send_data(&self, data: Vec<u8>) -> Result<(), TerminalDataSendError> {
        let byte_count = data.len();
        if byte_count == 0 {
            return Err(TerminalDataSendError::Empty(data));
        }
        if byte_count > self.state.max_pending_bytes {
            return Err(TerminalDataSendError::Oversized {
                data,
                max_bytes: self.state.max_pending_bytes,
            });
        }

        let (data, byte_permit) = self.acquire_bytes(byte_count, data).await?;
        let slot = tokio::select! {
            biased;
            _ = self.state.abort.cancelled() => {
                return Err(TerminalDataSendError::Closed(data));
            }
            slot = self.data_tx.reserve() => slot,
        };
        let slot = match slot {
            Ok(slot) => slot,
            Err(_) => return Err(TerminalDataSendError::Closed(data)),
        };
        if self.state.abort.is_cancelled() {
            return Err(TerminalDataSendError::Closed(data));
        }
        slot.send(QueuedData::new(data, byte_permit));
        Ok(())
    }

    async fn acquire_bytes(
        &self,
        byte_count: usize,
        data: Vec<u8>,
    ) -> Result<(Vec<u8>, ByteReservation), TerminalDataSendError> {
        let permits = u32::try_from(byte_count).expect("validated byte count fits in u32");
        let semaphore = self.state.byte_permits.clone();
        let permit = tokio::select! {
            biased;
            _ = self.state.abort.cancelled() => {
                return Err(TerminalDataSendError::Closed(data));
            }
            permit = semaphore.acquire_many_owned(permits) => permit,
        };
        let permit = match permit {
            Ok(permit) => permit,
            Err(_) => return Err(TerminalDataSendError::Closed(data)),
        };
        let reservation = self.state.reserve_bytes(byte_count, permit);
        Ok((data, reservation))
    }

    pub async fn send_control(&self, control: C) -> Result<(), TerminalControlSendError<C>> {
        let slot = tokio::select! {
            biased;
            _ = self.state.abort.cancelled() => {
                return Err(TerminalControlSendError::Closed(control));
            }
            slot = self.control_tx.reserve() => slot,
        };
        let slot = match slot {
            Ok(slot) => slot,
            Err(_) => return Err(TerminalControlSendError::Closed(control)),
        };
        if self.state.abort.is_cancelled() {
            return Err(TerminalControlSendError::Closed(control));
        }
        slot.send(control);
        Ok(())
    }

    pub fn abort(&self) {
        self.state.abort();
    }

    pub fn pending_bytes(&self) -> usize {
        self.state.pending_bytes()
    }

    pub fn peak_pending_bytes(&self) -> usize {
        self.state.peak_pending_bytes()
    }
}

struct QueuedData {
    data: Vec<u8>,
    reservation: ByteReservation,
}

impl QueuedData {
    fn new(data: Vec<u8>, reservation: ByteReservation) -> Self {
        Self { data, reservation }
    }

    fn into_data(self) -> Vec<u8> {
        let Self { data, reservation } = self;
        drop(reservation);
        data
    }
}

pub struct BoundedTerminalReceiver<C> {
    data_rx: mpsc::Receiver<QueuedData>,
    control_rx: mpsc::Receiver<C>,
    state: Arc<QueueState>,
    data_closed: bool,
    control_closed: bool,
    aborted: bool,
}

impl<C> BoundedTerminalReceiver<C> {
    pub async fn recv(&mut self) -> Option<TerminalIngressItem<C>> {
        loop {
            if self.aborted || self.state.abort.is_cancelled() {
                self.finish_abort();
                return None;
            }
            if self.data_closed && self.control_closed {
                return None;
            }

            let abort = self.state.abort.clone();
            tokio::select! {
                biased;
                _ = abort.cancelled() => {
                    self.finish_abort();
                    return None;
                }
                control = self.control_rx.recv(), if !self.control_closed => {
                    match control {
                        Some(control) => return Some(TerminalIngressItem::Control(control)),
                        None => self.control_closed = true,
                    }
                }
                data = self.data_rx.recv(), if !self.data_closed => {
                    match data {
                        Some(data) => return Some(TerminalIngressItem::Data(data.into_data())),
                        None => self.data_closed = true,
                    }
                }
            }
        }
    }

    pub fn abort(&mut self) {
        self.finish_abort();
    }

    pub fn pending_bytes(&self) -> usize {
        self.state.pending_bytes()
    }

    pub fn peak_pending_bytes(&self) -> usize {
        self.state.peak_pending_bytes()
    }

    fn finish_abort(&mut self) {
        self.state.abort();
        self.data_rx.close();
        self.control_rx.close();
        while let Ok(data) = self.data_rx.try_recv() {
            drop(data);
        }
        while self.control_rx.try_recv().is_ok() {}
        self.aborted = true;
    }
}

impl<C> Drop for BoundedTerminalReceiver<C> {
    fn drop(&mut self) {
        self.finish_abort();
    }
}

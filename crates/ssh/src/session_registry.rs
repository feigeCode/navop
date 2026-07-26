//! Application-level single-flight slots for shared SSH session managers.
//!
//! This module deliberately stops at the registry/slot contract.  It does not
//! introduce leases, idle eviction, a GPUI global owner, or migrate existing
//! consumers.  Those lifecycle policies build on top of the guarantees here:
//!
//! - one in-flight manager creation per [`ConnectionKey`];
//! - unrelated keys can make progress independently;
//! - manager creation never runs while the registry state lock is held;
//! - a result from a retired slot can never replace a newer generation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::{ConnectionKey, SshConnectConfig, SshSessionManager};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SlotGeneration(u64);

#[derive(Clone)]
struct CreationToken {
    generation: SlotGeneration,
    identity: Arc<()>,
}

impl CreationToken {
    fn new(generation: SlotGeneration) -> Self {
        Self {
            generation,
            identity: Arc::new(()),
        }
    }

    fn is_same_flight(&self, other: &Self) -> bool {
        self.generation == other.generation && Arc::ptr_eq(&self.identity, &other.identity)
    }
}

enum FlightOutcome<M> {
    Ready(Arc<M>),
    Failed(Arc<str>),
    Superseded,
}

enum RegistrySlot<M> {
    Creating {
        token: CreationToken,
        waiters: Vec<oneshot::Sender<FlightOutcome<M>>>,
    },
    Ready(Arc<M>),
}

struct RegistryState<M> {
    slots: HashMap<ConnectionKey, RegistrySlot<M>>,
    next_generation: u64,
}

impl<M> Default for RegistryState<M> {
    fn default() -> Self {
        Self {
            slots: HashMap::new(),
            next_generation: 0,
        }
    }
}

impl<M> RegistryState<M> {
    fn next_token(&mut self) -> CreationToken {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        CreationToken::new(SlotGeneration(self.next_generation))
    }

    fn take_current_creation(
        &mut self,
        key: &ConnectionKey,
        expected: &CreationToken,
    ) -> Option<Vec<oneshot::Sender<FlightOutcome<M>>>> {
        let is_current = matches!(
            self.slots.get(key),
            Some(RegistrySlot::Creating { token, .. }) if token.is_same_flight(expected)
        );
        if !is_current {
            return None;
        }

        match self.slots.remove(key) {
            Some(RegistrySlot::Creating { waiters, .. }) => Some(waiters),
            Some(RegistrySlot::Ready(_)) | None => {
                unreachable!("slot shape changed while the registry lock was held")
            }
        }
    }
}

struct RegistryShared<M> {
    state: StdMutex<RegistryState<M>>,
}

impl<M> Default for RegistryShared<M> {
    fn default() -> Self {
        Self {
            state: StdMutex::new(RegistryState::default()),
        }
    }
}

#[async_trait]
trait SessionManagerFactory<M>: Send + Sync + 'static {
    async fn create(&self, config: SshConnectConfig) -> Result<M>;
}

enum AcquirePhase<M> {
    Ready(Arc<M>),
    Wait(oneshot::Receiver<FlightOutcome<M>>),
    Start {
        token: CreationToken,
        receiver: oneshot::Receiver<FlightOutcome<M>>,
    },
}

struct SessionRegistryCore<M, F> {
    shared: Arc<RegistryShared<M>>,
    factory: F,
}

impl<M, F> SessionRegistryCore<M, F>
where
    M: Send + Sync + 'static,
    F: SessionManagerFactory<M>,
{
    fn new(factory: F) -> Self {
        Self {
            shared: Arc::new(RegistryShared::default()),
            factory,
        }
    }

    /// Acquire the manager for `key`, starting one detached creation flight
    /// when the slot is absent.
    ///
    /// The detached flight is intentional: cancelling the first acquire must
    /// not strand the slot or cause every other waiter to redial.
    async fn acquire(
        self: &Arc<Self>,
        key: &ConnectionKey,
        config: SshConnectConfig,
    ) -> Result<Arc<M>> {
        loop {
            let phase = {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                match state.slots.get_mut(key) {
                    Some(RegistrySlot::Ready(manager)) => AcquirePhase::Ready(manager.clone()),
                    Some(RegistrySlot::Creating { waiters, .. }) => {
                        let (sender, receiver) = oneshot::channel();
                        waiters.push(sender);
                        AcquirePhase::Wait(receiver)
                    }
                    None => {
                        let token = state.next_token();
                        let (sender, receiver) = oneshot::channel();
                        state.slots.insert(
                            key.clone(),
                            RegistrySlot::Creating {
                                token: token.clone(),
                                waiters: vec![sender],
                            },
                        );
                        AcquirePhase::Start { token, receiver }
                    }
                }
            };

            let receiver = match phase {
                AcquirePhase::Ready(manager) => return Ok(manager),
                AcquirePhase::Wait(receiver) => receiver,
                AcquirePhase::Start { token, receiver } => {
                    self.spawn_creation(key.clone(), token, config.clone());
                    receiver
                }
            };

            match receiver.await {
                Ok(FlightOutcome::Ready(manager)) => return Ok(manager),
                Ok(FlightOutcome::Failed(message)) => {
                    return Err(anyhow!(message.to_string()));
                }
                Ok(FlightOutcome::Superseded) | Err(_) => {
                    // A retire/replacement raced this acquire.  Re-enter the
                    // map and join (or create) the current generation.
                }
            }
        }
    }

    fn spawn_creation(
        self: &Arc<Self>,
        key: ConnectionKey,
        token: CreationToken,
        config: SshConnectConfig,
    ) {
        let registry = self.clone();
        tokio::spawn(async move {
            let mut cleanup =
                CreationCleanup::new(registry.shared.clone(), key.clone(), token.clone());
            let result = registry.factory.create(config).await;
            registry.finish_creation(&key, &token, result);
            cleanup.disarm();
        });
    }

    fn finish_creation(&self, key: &ConnectionKey, token: &CreationToken, result: Result<M>) {
        let (waiters, outcome) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(waiters) = state.take_current_creation(key, token) else {
                // The slot was retired or a newer generation already owns the
                // key.  Dropping `result` here prevents a stale manager from
                // becoming visible.
                return;
            };

            match result {
                Ok(manager) => {
                    let manager = Arc::new(manager);
                    state
                        .slots
                        .insert(key.clone(), RegistrySlot::Ready(manager.clone()));
                    (waiters, FlightOutcome::Ready(manager))
                }
                Err(error) => (
                    waiters,
                    FlightOutcome::Failed(Arc::from(format!("{error:#}"))),
                ),
            }
        };
        notify_waiters(waiters, outcome);
    }

    /// Retire only the currently published/in-flight slot.
    ///
    /// Lease accounting and transport disconnect are intentionally left to a
    /// later lifecycle layer.  Waiters on an in-flight generation retry
    /// against the new map state instead of accepting its stale result.
    fn retire(&self, key: &ConnectionKey) -> Option<Arc<M>> {
        let (manager, waiters) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match state.slots.remove(key) {
                Some(RegistrySlot::Ready(manager)) => (Some(manager), Vec::new()),
                Some(RegistrySlot::Creating { waiters, .. }) => (None, waiters),
                None => (None, Vec::new()),
            }
        };
        notify_waiters(waiters, FlightOutcome::Superseded);
        manager
    }
}

/// Removes a creation slot if its detached task is cancelled or panics before
/// publishing a result.  The registry state uses a synchronous mutex precisely
/// so this cleanup is possible from `Drop`; no state lock is ever held across
/// async manager creation.
struct CreationCleanup<M> {
    shared: Arc<RegistryShared<M>>,
    key: ConnectionKey,
    token: CreationToken,
    armed: bool,
}

impl<M> CreationCleanup<M> {
    fn new(shared: Arc<RegistryShared<M>>, key: ConnectionKey, token: CreationToken) -> Self {
        Self {
            shared,
            key,
            token,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<M> Drop for CreationCleanup<M> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let waiters = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state
                .take_current_creation(&self.key, &self.token)
                .unwrap_or_default()
        };
        notify_waiters(
            waiters,
            FlightOutcome::Failed(Arc::from("SSH session manager creation task was aborted")),
        );
    }
}

fn notify_waiters<M>(waiters: Vec<oneshot::Sender<FlightOutcome<M>>>, outcome: FlightOutcome<M>) {
    match outcome {
        FlightOutcome::Ready(manager) => {
            for waiter in waiters {
                let _ = waiter.send(FlightOutcome::Ready(manager.clone()));
            }
        }
        FlightOutcome::Failed(message) => {
            for waiter in waiters {
                let _ = waiter.send(FlightOutcome::Failed(message.clone()));
            }
        }
        FlightOutcome::Superseded => {
            for waiter in waiters {
                let _ = waiter.send(FlightOutcome::Superseded);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct DefaultSessionManagerFactory;

#[async_trait]
impl SessionManagerFactory<SshSessionManager> for DefaultSessionManagerFactory {
    async fn create(&self, config: SshConnectConfig) -> Result<SshSessionManager> {
        Ok(SshSessionManager::new(config))
    }
}

/// In-memory registry that coalesces manager creation by [`ConnectionKey`].
///
/// This type is intentionally not a process-global singleton.  The future
/// application service owns one registry instance and later adds lease,
/// shutdown, health and idle-eviction policy around it.
#[derive(Clone)]
pub struct SshSessionRegistry {
    inner: Arc<SessionRegistryCore<SshSessionManager, DefaultSessionManagerFactory>>,
}

impl SshSessionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SessionRegistryCore::new(DefaultSessionManagerFactory)),
        }
    }

    /// Return the single manager slot for `key`.
    ///
    /// Callers must build `key` from the same `config` and current opaque
    /// credential revisions.  Passing changed authentication material without
    /// advancing its revision violates the [`ConnectionKey`] contract.
    pub async fn acquire(
        &self,
        key: &ConnectionKey,
        config: SshConnectConfig,
    ) -> Result<Arc<SshSessionManager>> {
        self.inner.acquire(key, config).await
    }

    /// Remove the currently published or in-flight slot for `key`.
    ///
    /// This operation only changes registry visibility.  It deliberately does
    /// not disconnect a returned manager: a later lease/lifecycle layer must
    /// decide when the last consumer has released it and when idle retirement
    /// is safe.  Waiters on an in-flight slot transparently join a newer
    /// generation, and that retired flight can no longer publish its result.
    pub fn retire(&self, key: &ConnectionKey) -> Option<Arc<SshSessionManager>> {
        self.inner.retire(key)
    }
}

impl Default for SshSessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionManagerFactory, SessionRegistryCore};
    use crate::{
        ConnectionCredentialRevisions, ConnectionKey, CredentialRevision, HostKeyVerifier,
        JumpServerConnectConfig, ProxyConnectConfig, SshAuth, SshConnectConfig,
    };
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Semaphore;
    use tokio::time::{Duration, timeout};

    #[derive(Debug)]
    struct FakeManager {
        id: usize,
        host: String,
    }

    #[derive(Default)]
    struct CountingFactory {
        create_count: AtomicUsize,
    }

    #[async_trait]
    impl SessionManagerFactory<FakeManager> for Arc<CountingFactory> {
        async fn create(&self, config: SshConnectConfig) -> Result<FakeManager> {
            let id = self.create_count.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(FakeManager {
                id,
                host: config.host,
            })
        }
    }

    struct BlockingFactory {
        create_count: AtomicUsize,
        completed_count: AtomicUsize,
        blocked_hosts: HashSet<String>,
        permits: Semaphore,
    }

    impl BlockingFactory {
        fn new(blocked_hosts: impl IntoIterator<Item = &'static str>) -> Self {
            Self {
                create_count: AtomicUsize::new(0),
                completed_count: AtomicUsize::new(0),
                blocked_hosts: blocked_hosts.into_iter().map(str::to_owned).collect(),
                permits: Semaphore::new(0),
            }
        }

        fn release_one(&self) {
            self.permits.add_permits(1);
        }
    }

    #[async_trait]
    impl SessionManagerFactory<FakeManager> for Arc<BlockingFactory> {
        async fn create(&self, config: SshConnectConfig) -> Result<FakeManager> {
            let id = self.create_count.fetch_add(1, Ordering::SeqCst) + 1;
            if self.blocked_hosts.contains(&config.host) {
                self.permits
                    .acquire()
                    .await
                    .expect("test semaphore should remain open")
                    .forget();
            }
            self.completed_count.fetch_add(1, Ordering::SeqCst);
            Ok(FakeManager {
                id,
                host: config.host,
            })
        }
    }

    struct FirstFlightBlockingFactory {
        create_count: AtomicUsize,
        completed_count: AtomicUsize,
        first_permit: Semaphore,
        created_ids: StdMutex<Vec<usize>>,
    }

    impl FirstFlightBlockingFactory {
        fn new() -> Self {
            Self {
                create_count: AtomicUsize::new(0),
                completed_count: AtomicUsize::new(0),
                first_permit: Semaphore::new(0),
                created_ids: StdMutex::new(Vec::new()),
            }
        }
    }

    struct FirstFlightFailingFactory {
        create_count: AtomicUsize,
        first_permit: Semaphore,
    }

    impl FirstFlightFailingFactory {
        fn new() -> Self {
            Self {
                create_count: AtomicUsize::new(0),
                first_permit: Semaphore::new(0),
            }
        }
    }

    #[async_trait]
    impl SessionManagerFactory<FakeManager> for Arc<FirstFlightFailingFactory> {
        async fn create(&self, config: SshConnectConfig) -> Result<FakeManager> {
            let id = self.create_count.fetch_add(1, Ordering::SeqCst) + 1;
            if id == 1 {
                self.first_permit
                    .acquire()
                    .await
                    .expect("test semaphore should remain open")
                    .forget();
                return Err(anyhow!("shared fake creation failure"));
            }
            Ok(FakeManager {
                id,
                host: config.host,
            })
        }
    }

    #[async_trait]
    impl SessionManagerFactory<FakeManager> for Arc<FirstFlightBlockingFactory> {
        async fn create(&self, config: SshConnectConfig) -> Result<FakeManager> {
            let id = self.create_count.fetch_add(1, Ordering::SeqCst) + 1;
            if id == 1 {
                self.first_permit
                    .acquire()
                    .await
                    .expect("test semaphore should remain open")
                    .forget();
            }
            self.created_ids
                .lock()
                .expect("created ids lock should not be poisoned")
                .push(id);
            self.completed_count.fetch_add(1, Ordering::SeqCst);
            Ok(FakeManager {
                id,
                host: config.host,
            })
        }
    }

    fn test_config(host: &str) -> SshConnectConfig {
        SshConnectConfig {
            host: host.to_owned(),
            port: 22,
            username: "tester".to_owned(),
            auth: SshAuth::Agent,
            timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            jump_server: None::<JumpServerConnectConfig>,
            proxy: None::<ProxyConnectConfig>,
            keyboard_interactive_responder: None,
            host_key_verifier: HostKeyVerifier::default(),
            x11_forwarding: false,
        }
    }

    fn test_key(config: &SshConnectConfig, slot: u64) -> ConnectionKey {
        ConnectionKey::from_config(
            config,
            ConnectionCredentialRevisions::new(CredentialRevision::new(slot, 1)),
        )
        .expect("test config should produce a key")
    }

    async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
        timeout(Duration::from_secs(2), async {
            while counter.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("counter should reach expected value");
    }

    async fn wait_for_waiter_count<M, F>(
        registry: &SessionRegistryCore<M, F>,
        key: &ConnectionKey,
        expected: usize,
    ) {
        timeout(Duration::from_secs(2), async {
            loop {
                let waiter_count = {
                    let state = registry
                        .shared
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    match state.slots.get(key) {
                        Some(super::RegistrySlot::Creating { waiters, .. }) => waiters.len(),
                        Some(super::RegistrySlot::Ready(_)) | None => 0,
                    }
                };
                if waiter_count >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("slot should reach expected waiter count");
    }

    #[tokio::test]
    async fn concurrent_acquires_for_same_key_create_one_manager() {
        let factory = Arc::new(BlockingFactory::new(["shared.example"]));
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("shared.example");
        let key = test_key(&config, 1);

        let mut acquires = Vec::new();
        for _ in 0..12 {
            let registry = registry.clone();
            let config = config.clone();
            let key = key.clone();
            acquires.push(tokio::spawn(
                async move { registry.acquire(&key, config).await },
            ));
        }

        wait_for_count(&factory.create_count, 1).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        factory.release_one();

        let mut managers = Vec::new();
        for acquire in acquires {
            managers.push(
                acquire
                    .await
                    .expect("acquire task should not panic")
                    .expect("manager creation should succeed"),
            );
        }
        assert!(
            managers
                .iter()
                .all(|manager| Arc::ptr_eq(manager, &managers[0]))
        );
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_keys_never_share_a_manager() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let first_config = test_config("first.example");
        let second_config = test_config("second.example");
        let first_key = test_key(&first_config, 1);
        let second_key = test_key(&second_config, 2);

        let first = registry
            .acquire(&first_key, first_config)
            .await
            .expect("first manager should be created");
        let second = registry
            .acquire(&second_key, second_config)
            .await
            .expect("second manager should be created");

        assert!(!Arc::ptr_eq(&first, &second));
        assert_ne!(first.id, second.id);
        assert_eq!(first.host, "first.example");
        assert_eq!(second.host, "second.example");
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn blocked_creation_does_not_hold_the_registry_lock() {
        let factory = Arc::new(BlockingFactory::new(["blocked.example"]));
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let blocked_config = test_config("blocked.example");
        let blocked_key = test_key(&blocked_config, 1);
        let free_config = test_config("free.example");
        let free_key = test_key(&free_config, 2);

        let blocked_registry = registry.clone();
        let blocked =
            tokio::spawn(
                async move { blocked_registry.acquire(&blocked_key, blocked_config).await },
            );
        wait_for_count(&factory.create_count, 1).await;

        let free = timeout(
            Duration::from_secs(1),
            registry.acquire(&free_key, free_config),
        )
        .await
        .expect("unrelated key should not wait for blocked creation")
        .expect("unrelated manager should be created");
        assert_eq!(free.host, "free.example");
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);

        factory.release_one();
        blocked
            .await
            .expect("blocked acquire task should not panic")
            .expect("blocked manager should finish");
    }

    #[tokio::test]
    async fn failed_flight_is_shared_and_a_later_acquire_retries() {
        let factory = Arc::new(FirstFlightFailingFactory::new());
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("retry.example");
        let key = test_key(&config, 1);

        let mut failed_acquires = Vec::new();
        for _ in 0..2 {
            let registry = registry.clone();
            let config = config.clone();
            let key = key.clone();
            failed_acquires.push(tokio::spawn(
                async move { registry.acquire(&key, config).await },
            ));
        }

        wait_for_count(&factory.create_count, 1).await;
        wait_for_waiter_count(&registry, &key, 2).await;
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        factory.first_permit.add_permits(1);

        for acquire in failed_acquires {
            let error = acquire
                .await
                .expect("acquire task should not panic")
                .expect_err("shared creation should fail");
            assert_eq!(error.to_string(), "shared fake creation failure");
        }

        let second = registry
            .acquire(&key, config)
            .await
            .expect("next acquire should create a fresh flight");
        assert_eq!(second.id, 2);
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stale_generation_cannot_replace_the_new_slot() {
        let factory = Arc::new(FirstFlightBlockingFactory::new());
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("generation.example");
        let key = test_key(&config, 1);

        let first_registry = registry.clone();
        let first_config = config.clone();
        let first_key = key.clone();
        let first =
            tokio::spawn(async move { first_registry.acquire(&first_key, first_config).await });
        wait_for_count(&factory.create_count, 1).await;

        assert!(registry.retire(&key).is_none());
        wait_for_count(&factory.create_count, 2).await;

        let current = registry
            .acquire(&key, config.clone())
            .await
            .expect("replacement generation should succeed");
        assert_eq!(current.id, 2);

        factory.first_permit.add_permits(1);
        wait_for_count(&factory.completed_count, 2).await;
        let first_result = first
            .await
            .expect("first acquire task should not panic")
            .expect("first acquire should join the replacement generation");
        assert!(Arc::ptr_eq(&first_result, &current));

        let after_stale_completion = registry
            .acquire(&key, config)
            .await
            .expect("current slot should remain available");
        assert!(Arc::ptr_eq(&after_stale_completion, &current));
        assert_eq!(
            *factory
                .created_ids
                .lock()
                .expect("created ids lock should not be poisoned"),
            vec![2, 1],
            "the stale result may finish, but it must not replace manager 2"
        );
    }

    #[tokio::test]
    async fn cancelling_first_waiter_does_not_cancel_shared_creation() {
        let factory = Arc::new(BlockingFactory::new(["cancel.example"]));
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("cancel.example");
        let key = test_key(&config, 1);

        let cancelled_registry = registry.clone();
        let cancelled_config = config.clone();
        let cancelled_key = key.clone();
        let cancelled = tokio::spawn(async move {
            cancelled_registry
                .acquire(&cancelled_key, cancelled_config)
                .await
        });
        wait_for_count(&factory.create_count, 1).await;
        cancelled.abort();
        let _ = cancelled.await;

        let surviving_registry = registry.clone();
        let surviving_config = config.clone();
        let surviving_key = key.clone();
        let surviving = tokio::spawn(async move {
            surviving_registry
                .acquire(&surviving_key, surviving_config)
                .await
        });
        factory.release_one();
        let manager = surviving
            .await
            .expect("surviving acquire task should not panic")
            .expect("detached creation should satisfy later waiters");

        let cached = registry
            .acquire(&key, config)
            .await
            .expect("completed slot should remain cached");
        assert!(Arc::ptr_eq(&manager, &cached));
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
    }
}

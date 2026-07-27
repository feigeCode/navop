/// Whether a reconnect can reuse the currently installed generation-bound
/// lease or must acquire one for the latest persisted identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SshLeaseSelection {
    ReuseInstalled,
    AcquireReplacement,
}

pub(crate) fn select_ssh_lease<I: Eq>(
    installed_identity: Option<&I>,
    desired_identity: &I,
) -> SshLeaseSelection {
    match installed_identity {
        Some(installed) if installed == desired_identity => SshLeaseSelection::ReuseInstalled,
        Some(_) | None => SshLeaseSelection::AcquireReplacement,
    }
}

/// Stable outcome for an asynchronous application-service acquisition after
/// returning to the Terminal entity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SshLeaseAcquireResolution {
    Install,
    RejectStale,
    FailCurrent,
}

/// Identity and Terminal connection generation captured before awaiting the
/// application SSH service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SshLeaseAcquireRequest<I> {
    generation: u64,
    identity: I,
}

impl<I> SshLeaseAcquireRequest<I> {
    pub(crate) fn new(generation: u64, identity: I) -> Self {
        Self {
            generation,
            identity,
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn identity(&self) -> &I {
        &self.identity
    }
}

impl<I: Eq> SshLeaseAcquireRequest<I> {
    pub(crate) fn resolve(
        &self,
        current_generation: u64,
        current_desired_identity: Option<&I>,
        acquired: bool,
    ) -> SshLeaseAcquireResolution {
        let is_current = self.generation == current_generation
            && current_desired_identity.is_some_and(|identity| identity == &self.identity);
        if !is_current {
            return SshLeaseAcquireResolution::RejectStale;
        }
        if acquired {
            SshLeaseAcquireResolution::Install
        } else {
            SshLeaseAcquireResolution::FailCurrent
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SshLeaseAcquireRequest, SshLeaseAcquireResolution, SshLeaseSelection, select_ssh_lease,
    };

    #[test]
    fn ordinary_reconnect_reuses_the_installed_identity() {
        assert_eq!(
            SshLeaseSelection::ReuseInstalled,
            select_ssh_lease(Some(&"persisted-v1"), &"persisted-v1")
        );
    }

    #[test]
    fn missing_or_changed_installed_identity_acquires_a_replacement() {
        assert_eq!(
            SshLeaseSelection::AcquireReplacement,
            select_ssh_lease::<&str>(None, &"persisted-v1")
        );
        assert_eq!(
            SshLeaseSelection::AcquireReplacement,
            select_ssh_lease(Some(&"persisted-v1"), &"persisted-v2")
        );
    }

    #[test]
    fn stale_generation_cannot_install_an_acquired_lease() {
        let request = SshLeaseAcquireRequest::new(4, "persisted-v1");

        assert_eq!(
            SshLeaseAcquireResolution::RejectStale,
            request.resolve(5, Some(&"persisted-v1"), true)
        );
    }

    #[test]
    fn changed_desired_identity_rejects_an_acquired_lease() {
        let request = SshLeaseAcquireRequest::new(5, "persisted-v1");

        assert_eq!(
            SshLeaseAcquireResolution::RejectStale,
            request.resolve(5, Some(&"persisted-v2"), true)
        );
        assert_eq!(
            SshLeaseAcquireResolution::RejectStale,
            request.resolve(5, None, true)
        );
    }

    #[test]
    fn newer_acquire_installs_before_older_result_is_rejected() {
        let older = SshLeaseAcquireRequest::new(5, "persisted-v1");
        let newer = SshLeaseAcquireRequest::new(6, "persisted-v2");

        assert_eq!(
            SshLeaseAcquireResolution::Install,
            newer.resolve(6, Some(&"persisted-v2"), true)
        );
        assert_eq!(
            SshLeaseAcquireResolution::RejectStale,
            older.resolve(6, Some(&"persisted-v2"), true)
        );
    }

    #[test]
    fn current_acquire_failure_has_an_explicit_terminal_resolution() {
        let request = SshLeaseAcquireRequest::new(5, "persisted-v1");

        assert_eq!(
            SshLeaseAcquireResolution::FailCurrent,
            request.resolve(5, Some(&"persisted-v1"), false)
        );
        assert_eq!(
            SshLeaseAcquireResolution::RejectStale,
            request.resolve(6, Some(&"persisted-v1"), false)
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbSessionResource {
    extension_id: String,
    connection_id: String,
    session_id: String,
    closed: bool,
}

impl DbSessionResource {
    pub fn new(
        extension_id: impl Into<String>,
        connection_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            connection_id: connection_id.into(),
            session_id: session_id.into(),
            closed: false,
        }
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub fn extension_id(&self) -> &str {
        &self.extension_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiProgressResource {
    extension_id: String,
    progress_id: String,
    closed: bool,
}

impl UiProgressResource {
    pub fn new(extension_id: impl Into<String>, progress_id: impl Into<String>) -> Self {
        Self {
            extension_id: extension_id.into(),
            progress_id: progress_id.into(),
            closed: false,
        }
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_session_close_is_idempotent() {
        let mut session = DbSessionResource::new("ext", "conn", "session");

        assert!(!session.is_closed());
        session.close();
        session.close();
        assert!(session.is_closed());
    }
}

pub(crate) fn require_tokio_runtime(operation: &str) -> anyhow::Result<()> {
    tokio::runtime::Handle::try_current()
        .map(|_| ())
        .map_err(|_| anyhow::anyhow!("{operation} requires the application Tokio runtime"))
}

#[cfg(test)]
mod tests {
    use super::require_tokio_runtime;

    #[test]
    fn rejects_database_operation_outside_tokio_runtime() {
        let error = require_tokio_runtime("database export")
            .expect_err("operation outside Tokio should fail");

        assert!(error.to_string().contains("database export"));
        assert!(error.to_string().contains("Tokio runtime"));
    }

    #[tokio::test]
    async fn accepts_database_operation_inside_tokio_runtime() {
        require_tokio_runtime("database export").expect("operation inside Tokio should succeed");
    }
}

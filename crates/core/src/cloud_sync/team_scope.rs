#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CloudAccountScope {
    pub environment: String,
    pub user_id: String,
}

impl CloudAccountScope {
    pub fn new(environment: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self {
            environment: environment.into().trim().trim_end_matches('/').to_string(),
            user_id: user_id.into(),
        }
    }
}

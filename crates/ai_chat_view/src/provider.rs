use one_core::llm::ProviderConfig;

#[derive(Clone, Debug)]
pub struct ProviderItem {
    pub id: String,
    pub name: String,
    pub model: String,
    pub provider_type: String,
    pub models: Vec<String>,
    pub is_default: bool,
    pub is_builtin: bool,
}

impl ProviderItem {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        model: impl Into<String>,
        provider_type: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            model: model.into(),
            provider_type: provider_type.into(),
            models: Vec::new(),
            is_default: false,
            is_builtin: false,
        }
    }

    pub fn from_config(config: &ProviderConfig) -> Self {
        let models = if config.models.is_empty() {
            vec![config.model.clone()]
        } else {
            config.models.clone()
        };
        Self {
            id: config.id.to_string(),
            name: config.name.clone(),
            model: config.model.clone(),
            provider_type: config.provider_type.display_name().to_string(),
            models,
            is_default: config.is_default,
            is_builtin: config.is_builtin(),
        }
    }

    pub fn with_models(mut self, models: Vec<String>) -> Self {
        self.models = models;
        self
    }

    pub fn with_default(mut self, is_default: bool) -> Self {
        self.is_default = is_default;
        self
    }

    pub fn display_name(&self) -> String {
        format!("{}  ({})", self.name, self.provider_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_item_display_name_includes_provider_type() {
        let item = ProviderItem::new("p1", "OpenAI", "gpt-4.1", "OpenAI");

        assert_eq!(item.display_name(), "OpenAI  (OpenAI)");
    }

    #[test]
    fn provider_item_builders_set_models_and_default_flag() {
        let item = ProviderItem::new("p1", "OpenAI", "gpt-4.1", "OpenAI")
            .with_models(vec!["gpt-4.1".to_string(), "o3".to_string()])
            .with_default(true);

        assert_eq!(item.models, vec!["gpt-4.1".to_string(), "o3".to_string()]);
        assert!(item.is_default);
    }
}

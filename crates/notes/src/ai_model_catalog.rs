use cditor_app::AiModelDescriptor;
use one_core::llm::ProviderConfig;
use std::collections::HashMap;

#[derive(Clone)]
pub(crate) struct ModelRoute {
    pub config: ProviderConfig,
    pub model: String,
}

pub(crate) struct ModelCatalog {
    pub routes: HashMap<String, ModelRoute>,
    pub descriptors: Vec<AiModelDescriptor>,
    pub default_model_id: Option<String>,
}

pub(crate) fn build_catalog(configs: Vec<ProviderConfig>) -> ModelCatalog {
    let mut routes = HashMap::new();
    let mut descriptors = Vec::new();
    let mut default_model_id = None;
    for config in configs.into_iter().filter(|config| config.enabled) {
        let provider_name = if config.name.trim().is_empty() {
            config.provider_type.display_name().to_owned()
        } else {
            config.name.clone()
        };
        for model in model_names(&config) {
            let model_id = format!("navop:{}:{}", config.id, model);
            let mut routed_config = config.clone();
            routed_config.model = model.clone();
            routes.insert(
                model_id.clone(),
                ModelRoute {
                    config: routed_config,
                    model: model.clone(),
                },
            );
            descriptors.push(
                AiModelDescriptor::new(
                    model_id.clone(),
                    format!("{provider_name} / {model}"),
                    provider_name.clone(),
                )
                .with_description(config.provider_type.display_name()),
            );
            if default_model_id.is_none() && config.is_default {
                default_model_id = Some(model_id);
            }
        }
    }
    if default_model_id.is_none() {
        default_model_id = descriptors.first().map(|descriptor| descriptor.id.clone());
    }
    ModelCatalog {
        routes,
        descriptors,
        default_model_id,
    }
}

pub(crate) fn model_names(config: &ProviderConfig) -> Vec<String> {
    let mut models = config
        .models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !config.model.trim().is_empty() {
        models.retain(|model| model != &config.model);
        models.insert(0, config.model.clone());
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::llm::ProviderType;

    fn config() -> ProviderConfig {
        ProviderConfig {
            id: 7,
            name: "DeepSeek".to_owned(),
            provider_type: ProviderType::DeepSeek,
            model: "deepseek-chat".to_owned(),
            models: vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned()],
            is_default: true,
            ..Default::default()
        }
    }

    #[test]
    fn catalog_namespaces_provider_and_model_ids() {
        let catalog = build_catalog(vec![config()]);
        assert_eq!(2, catalog.descriptors.len());
        assert_eq!("navop:7:deepseek-chat", catalog.descriptors[0].id);
        assert_eq!(
            Some("navop:7:deepseek-chat".to_owned()),
            catalog.default_model_id
        );
    }

    #[test]
    fn model_names_keeps_configured_model_first() {
        let mut provider = config();
        provider.model = "deepseek-reasoner".to_owned();
        assert_eq!(
            vec!["deepseek-reasoner", "deepseek-chat"],
            model_names(&provider)
        );
    }
}

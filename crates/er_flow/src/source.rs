use anyhow::{Context, Result};

use crate::ErDiagram;

pub trait ErDataSource {
    fn load_er_diagram(&self) -> Result<ErDiagram>;
}

impl ErDataSource for ErDiagram {
    fn load_er_diagram(&self) -> Result<ErDiagram> {
        Ok(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct StaticErDataSource {
    diagram: ErDiagram,
}

impl StaticErDataSource {
    pub fn new(diagram: ErDiagram) -> Self {
        Self { diagram }
    }
}

impl ErDataSource for StaticErDataSource {
    fn load_er_diagram(&self) -> Result<ErDiagram> {
        Ok(self.diagram.clone())
    }
}

#[derive(Debug, Clone)]
pub struct JsonErDataSource {
    value: serde_json::Value,
}

impl JsonErDataSource {
    pub fn from_value(value: serde_json::Value) -> Self {
        Self { value }
    }

    pub fn from_json_str(json: impl AsRef<str>) -> Result<Self> {
        let value =
            serde_json::from_str(json.as_ref()).context("failed to parse ER JSON source")?;
        Ok(Self { value })
    }
}

impl ErDataSource for JsonErDataSource {
    fn load_er_diagram(&self) -> Result<ErDiagram> {
        serde_json::from_value(self.value.clone()).context("failed to deserialize ER diagram")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErEntity, ErField};

    #[test]
    fn static_source_returns_diagram() {
        let diagram = ErDiagram {
            entities: vec![ErEntity {
                id: "users".to_string(),
                name: "users".to_string(),
                comment: None,
                fields: vec![ErField {
                    name: "id".to_string(),
                    data_type: "uuid".to_string(),
                    nullable: false,
                    primary_key: true,
                    unique: true,
                    comment: None,
                }],
            }],
            relationships: vec![],
        };

        let source = StaticErDataSource::new(diagram.clone());

        assert_eq!(source.load_er_diagram().unwrap(), diagram);
    }

    #[test]
    fn json_source_deserializes_diagram() {
        let source = JsonErDataSource::from_json_str(
            r#"{
                "entities": [{
                    "id": "users",
                    "name": "users",
                    "fields": [{"name": "id", "data_type": "uuid", "primary_key": true}]
                }],
                "relationships": []
            }"#,
        )
        .unwrap();

        let diagram = source.load_er_diagram().unwrap();

        assert_eq!(diagram.entities.len(), 1);
        assert_eq!(diagram.entities[0].fields[0].name, "id");
        assert!(diagram.entities[0].fields[0].primary_key);
    }
}

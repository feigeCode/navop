//! Chart JSON 协议与解析。
//!
//! 用于识别 AI 返回的图表 JSON 代码块，并转换为可渲染数据。这里只提供通用数据
//! contract，具体图表 UI 可由业务 crate 或后续内置卡片实现。

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    Line,
    Bar,
    Pie,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChartJsonBlock {
    pub chart_type: ChartType,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub x_key: Option<String>,
    #[serde(default)]
    pub y_key: Option<String>,
    #[serde(default)]
    pub category_key: Option<String>,
    #[serde(default)]
    pub value_key: Option<String>,
    pub data: Vec<Value>,
}

impl ChartJsonBlock {
    pub fn to_xy_points(&self) -> Vec<ChartXYPoint> {
        let x_key = self
            .x_key
            .as_deref()
            .or(self.category_key.as_deref())
            .unwrap_or("x");
        let y_key = self
            .y_key
            .as_deref()
            .or(self.value_key.as_deref())
            .unwrap_or("y");

        self.data
            .iter()
            .filter_map(|row| {
                let obj = row.as_object()?;
                let x = obj.get(x_key).and_then(value_to_label)?;
                let y = obj.get(y_key).and_then(value_to_f64)?;
                Some(ChartXYPoint { x, y })
            })
            .collect()
    }

    pub fn to_pie_points(&self) -> Vec<ChartPiePoint> {
        let category_key = self.category_key.as_deref().unwrap_or("category");
        let value_key = self.value_key.as_deref().unwrap_or("value");

        self.data
            .iter()
            .filter_map(|row| {
                let obj = row.as_object()?;
                let category = obj.get(category_key).and_then(value_to_label)?;
                let value = obj.get(value_key).and_then(value_to_f64)?;
                Some(ChartPiePoint { category, value })
            })
            .collect()
    }

    pub fn is_renderable(&self) -> bool {
        match self.chart_type {
            ChartType::Line | ChartType::Bar => !self.to_xy_points().is_empty(),
            ChartType::Pie => !self.to_pie_points().is_empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartXYPoint {
    pub x: String,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartPiePoint {
    pub category: String,
    pub value: f64,
}

pub fn parse_chart_json_block(code: &str, language: Option<&str>) -> Option<ChartJsonBlock> {
    if let Some(lang) = language {
        let lower = lang.to_ascii_lowercase();
        if lower != "json" && lower != "chart" && lower != "chart-json" {
            return None;
        }
    }

    let parsed: ChartJsonBlock = serde_json::from_str(code).ok()?;
    if parsed.data.is_empty() || !parsed.is_renderable() {
        return None;
    }
    Some(parsed)
}

fn value_to_label(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = value.as_f64() {
        return Some(n.to_string());
    }
    if let Some(b) = value.as_bool() {
        return Some(b.to_string());
    }
    None
}

fn value_to_f64(value: &Value) -> Option<f64> {
    if let Some(n) = value.as_f64() {
        return Some(n);
    }
    value.as_str().and_then(|s| s.parse::<f64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_line_chart_json_with_custom_keys() {
        let src = r#"{
            "chart_type": "line",
            "x_key": "month",
            "y_key": "revenue",
            "data": [
                {"month": "Jan", "revenue": 10},
                {"month": "Feb", "revenue": "20.5"}
            ]
        }"#;

        let parsed = parse_chart_json_block(src, Some("json")).expect("chart should parse");
        let points = parsed.to_xy_points();

        assert_eq!(2, points.len());
        assert_eq!("Jan", points[0].x);
        assert_eq!(20.5, points[1].y);
    }

    #[test]
    fn parses_pie_chart_json_with_default_keys() {
        let src = r#"{
            "chart_type": "pie",
            "data": [
                {"category": "A", "value": 3},
                {"category": "B", "value": 7}
            ]
        }"#;

        let parsed = parse_chart_json_block(src, Some("chart")).expect("chart should parse");
        let points = parsed.to_pie_points();

        assert_eq!(2, points.len());
        assert_eq!("A", points[0].category);
        assert_eq!(7.0, points[1].value);
    }

    #[test]
    fn rejects_non_chart_language_and_empty_data() {
        assert!(
            parse_chart_json_block(
                r#"{"chart_type":"line","data":[{"x":"a","y":1}]}"#,
                Some("sql")
            )
            .is_none()
        );
        assert!(
            parse_chart_json_block(r#"{"chart_type":"line","data":[]}"#, Some("json")).is_none()
        );
    }
}

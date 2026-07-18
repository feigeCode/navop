use std::collections::{HashMap, HashSet};

use bson::{Bson, Document, doc};
use gpui::{
    App, AppContext, Context, IntoElement, ParentElement, SharedString, Styled, Subscription,
    Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme,
    input::{InputEvent, InputState},
};
use one_ui::edit_table::{CellEditor, Column, EditTableDelegate, EditTableState};

use crate::types::{MongoError, bson_to_compact_json, bson_to_string};

const TABLE_MAX_COLUMNS: usize = 24;
const TABLE_ID_COLUMN_WIDTH: f32 = 220.0;
const TABLE_FIELD_COLUMN_WIDTH: f32 = 160.0;

#[derive(Debug, Clone, PartialEq)]
pub struct BsonCellValue {
    pub row_index: usize,
    pub field: String,
    pub old_value: Bson,
    pub new_value: Bson,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MongoDocumentFieldChange {
    pub row_index: usize,
    pub id: Bson,
    pub set_fields: Document,
}

pub struct MongoDocumentTableDelegate {
    columns: Vec<Column>,
    column_names: Vec<String>,
    documents: Vec<Document>,
    original_documents: Vec<Document>,
    cell_changes: HashMap<(usize, String), BsonCellValue>,
    modified_cells: HashSet<(usize, usize)>,
    editable: bool,
    loading: bool,
}

impl MongoDocumentTableDelegate {
    pub fn new(documents: Vec<Document>) -> Self {
        let column_names = collect_table_columns(documents.iter());
        let columns = build_columns(&column_names);
        Self {
            columns,
            column_names,
            original_documents: documents.clone(),
            documents,
            cell_changes: HashMap::new(),
            modified_cells: HashSet::new(),
            editable: true,
            loading: false,
        }
    }

    pub fn set_documents(&mut self, documents: Vec<Document>) {
        self.column_names = collect_table_columns(documents.iter());
        self.columns = build_columns(&self.column_names);
        self.original_documents = documents.clone();
        self.documents = documents;
        self.cell_changes.clear();
        self.modified_cells.clear();
    }

    pub fn documents(&self) -> &[Document] {
        &self.documents
    }

    pub fn has_changes(&self) -> bool {
        !self.cell_changes.is_empty()
    }

    pub fn cell_changes(&self) -> Vec<BsonCellValue> {
        let mut changes = self.cell_changes.values().cloned().collect::<Vec<_>>();
        changes.sort_by(|left, right| {
            left.row_index
                .cmp(&right.row_index)
                .then_with(|| left.field.cmp(&right.field))
        });
        changes
    }

    pub fn field_changes(&self) -> Result<Vec<MongoDocumentFieldChange>, MongoError> {
        let mut rows: HashMap<usize, Document> = HashMap::new();
        for change in self.cell_changes() {
            rows.entry(change.row_index)
                .or_default()
                .insert(change.field, change.new_value);
        }

        let mut result = Vec::with_capacity(rows.len());
        for (row_index, set_fields) in rows {
            let id = self
                .original_documents
                .get(row_index)
                .and_then(|document| document.get("_id"))
                .cloned()
                .ok_or_else(|| MongoError::InvalidFilter("Missing _id for update.".to_string()))?;
            result.push(MongoDocumentFieldChange {
                row_index,
                id,
                set_fields,
            });
        }
        result.sort_by_key(|change| change.row_index);
        Ok(result)
    }

    pub fn revert_changes(&mut self) {
        self.documents = self.original_documents.clone();
        self.cell_changes.clear();
        self.modified_cells.clear();
    }

    pub fn field_name_at(&self, col_ix: usize) -> Option<&str> {
        self.field_name(col_ix)
    }

    pub fn value_at(&self, row_ix: usize, col_ix: usize) -> Option<&Bson> {
        let field = self.field_name(col_ix)?;
        self.documents.get(row_ix)?.get(field)
    }

    pub fn is_detail_editable_field(&self, row_ix: usize, col_ix: usize) -> bool {
        let Some(field) = self.field_name(col_ix) else {
            return false;
        };
        if !is_safe_update_field_name(field) {
            return false;
        }
        self.value_at(row_ix, col_ix)
            .is_some_and(|value| !is_editable_bson_value(value))
    }

    pub fn record_cell_edit(
        &mut self,
        row_index: usize,
        field: &str,
        text: &str,
    ) -> Result<bool, MongoError> {
        let col_index = self
            .column_names
            .iter()
            .position(|column| column == field)
            .ok_or_else(|| MongoError::InvalidFilter(format!("Unknown field: {field}")))?;
        if !is_safe_update_field_name(field) {
            return Err(MongoError::InvalidFilter(
                "This field cannot be edited with $set.".to_string(),
            ));
        }

        let original_value = self
            .original_documents
            .get(row_index)
            .and_then(|document| document.get(field))
            .cloned();
        let new_value = parse_edit_cell_value(original_value.as_ref(), text)?;
        let Some(original_value) = original_value else {
            return Ok(false);
        };
        let Some(document) = self.documents.get_mut(row_index) else {
            return Ok(false);
        };

        document.insert(field, new_value.clone());
        if original_value == new_value {
            self.cell_changes.remove(&(row_index, field.to_string()));
            self.modified_cells.remove(&(row_index, col_index));
        } else {
            self.cell_changes.insert(
                (row_index, field.to_string()),
                BsonCellValue {
                    row_index,
                    field: field.to_string(),
                    old_value: original_value,
                    new_value,
                },
            );
            self.modified_cells.insert((row_index, col_index));
        }
        Ok(true)
    }

    fn field_name(&self, col_ix: usize) -> Option<&str> {
        self.column_names.get(col_ix).map(String::as_str)
    }

    fn is_cell_editable(&self, row_ix: usize, col_ix: usize) -> bool {
        if !self.editable {
            return false;
        }
        let Some(field) = self.field_name(col_ix) else {
            return false;
        };
        if !is_safe_update_field_name(field) {
            return false;
        }
        self.documents
            .get(row_ix)
            .and_then(|document| document.get(field))
            .is_some_and(is_editable_bson_value)
    }
}

impl EditTableDelegate for MongoDocumentTableDelegate {
    fn cell_edit_enabled(&self, _cx: &App) -> bool {
        true
    }

    fn single_click_to_edit(&self, _cx: &App) -> bool {
        true
    }

    fn row_number_enabled(&self, _cx: &App) -> bool {
        true
    }

    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.documents.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<EditTableState<Self>>,
    ) -> impl IntoElement {
        let value = self.field_name(col_ix).and_then(|field| {
            self.documents
                .get(row_ix)
                .and_then(|document| document.get(field))
        });
        let text = table_cell_text(value);

        div()
            .size_full()
            .px_2()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .when(value.is_none(), |this| {
                this.text_color(cx.theme().muted_foreground.opacity(0.55))
            })
            .child(text)
    }

    fn build_input(
        &self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<EditTableState<Self>>,
    ) -> Option<(CellEditor, Vec<Subscription>)> {
        if !self.is_cell_editable(row_ix, col_ix) {
            return None;
        }

        let field = self.field_name(col_ix)?;
        let value = self
            .documents
            .get(row_ix)
            .and_then(|document| document.get(field))
            .map(edit_input_text)
            .unwrap_or_default();

        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).multi_line(true).rows(1);
            state.set_value(value, window, cx);
            state.focus(window, cx);
            state
        });
        let input_subscription = cx.subscribe_in(
            &input,
            window,
            move |table, _, evt: &InputEvent, window, cx| match evt {
                InputEvent::Blur | InputEvent::PressEnter { .. } => {
                    table.commit_cell_edit(window, cx);
                }
                _ => {}
            },
        );
        Some((CellEditor::Input(input), vec![input_subscription]))
    }

    fn on_cell_edited(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        new_value: String,
        _window: &mut Window,
        _cx: &mut Context<EditTableState<Self>>,
    ) -> bool {
        let Some(field) = self.field_name(col_ix).map(str::to_string) else {
            return false;
        };
        self.record_cell_edit(row_ix, &field, &new_value)
            .unwrap_or(false)
    }

    fn is_cell_modified(&self, row_ix: usize, col_ix: usize, _cx: &App) -> bool {
        self.modified_cells.contains(&(row_ix, col_ix))
    }

    fn loading(&self, _cx: &App) -> bool {
        self.loading
    }

    fn get_cell_value(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        self.field_name(col_ix)
            .and_then(|field| {
                self.documents
                    .get(row_ix)
                    .and_then(|document| document.get(field))
            })
            .map(|value| table_cell_text(Some(value)))
            .unwrap_or_default()
    }

    fn set_cell_values(
        &mut self,
        changes: Vec<(usize, usize, String)>,
        _window: &mut Window,
        _cx: &mut Context<EditTableState<Self>>,
    ) -> bool {
        let mut changed = false;
        for (row_ix, col_ix, value) in changes {
            let Some(field) = self.field_name(col_ix).map(str::to_string) else {
                continue;
            };
            if self
                .record_cell_edit(row_ix, &field, &value)
                .unwrap_or(false)
            {
                changed = true;
            }
        }
        changed
    }
}

pub fn collect_table_columns<'a>(documents: impl Iterator<Item = &'a Document>) -> Vec<String> {
    let mut columns = Vec::new();
    for document in documents {
        for key in document.keys() {
            if !columns.iter().any(|column| column == key) {
                columns.push(key.clone());
            }
        }
    }

    if let Some(index) = columns.iter().position(|column| column == "_id") {
        columns.remove(index);
    }
    columns.insert(0, "_id".to_string());
    columns.truncate(TABLE_MAX_COLUMNS);
    columns
}

pub fn table_cell_text(value: Option<&Bson>) -> String {
    let Some(value) = value else {
        return String::new();
    };

    match value {
        Bson::String(value) => value.clone(),
        Bson::Int32(_)
        | Bson::Int64(_)
        | Bson::Double(_)
        | Bson::Boolean(_)
        | Bson::ObjectId(_) => bson_to_string(value),
        Bson::DateTime(value) => value
            .try_to_rfc3339_string()
            .unwrap_or_else(|_| value.timestamp_millis().to_string()),
        _ => bson_to_compact_json(value).unwrap_or_else(|_| format!("{:?}", value)),
    }
}

pub fn parse_edit_cell_value(original: Option<&Bson>, text: &str) -> Result<Bson, MongoError> {
    let Some(original) = original else {
        return Err(MongoError::InvalidFilter(
            "Missing field cannot be edited inline.".to_string(),
        ));
    };
    let trimmed = text.trim();
    match original {
        Bson::String(_) => Ok(Bson::String(text.to_string())),
        Bson::Int32(_) => trimmed
            .parse::<i32>()
            .map(Bson::Int32)
            .map_err(|error| MongoError::InvalidFilter(error.to_string())),
        Bson::Int64(_) => trimmed
            .parse::<i64>()
            .map(Bson::Int64)
            .map_err(|error| MongoError::InvalidFilter(error.to_string())),
        Bson::Double(_) => trimmed
            .parse::<f64>()
            .map(Bson::Double)
            .map_err(|error| MongoError::InvalidFilter(error.to_string())),
        Bson::Boolean(_) => trimmed
            .parse::<bool>()
            .map(Bson::Boolean)
            .map_err(|error| MongoError::InvalidFilter(error.to_string())),
        Bson::DateTime(_) => bson::DateTime::parse_rfc3339_str(trimmed)
            .map(Bson::DateTime)
            .map_err(|error| MongoError::InvalidFilter(error.to_string())),
        Bson::Null => {
            if trimmed.is_empty() {
                Ok(Bson::Null)
            } else {
                Ok(Bson::String(text.to_string()))
            }
        }
        _ => Err(MongoError::InvalidFilter(
            "This BSON value cannot be edited inline.".to_string(),
        )),
    }
}

pub fn build_set_update_document(field: &str, value: Bson) -> Document {
    doc! { "$set": { field: value } }
}

pub fn build_set_update_document_from_fields(set_fields: Document) -> Document {
    doc! { "$set": set_fields }
}

fn build_columns(column_names: &[String]) -> Vec<Column> {
    column_names
        .iter()
        .map(|column| {
            let col = Column::new(
                SharedString::from(column.clone()),
                SharedString::from(column.clone()),
            )
            .width(table_column_width(column));
            if column == "_id" {
                col.fixed_left()
            } else {
                col
            }
        })
        .collect()
}

fn table_column_width(column: &str) -> gpui::Pixels {
    if column == "_id" {
        px(TABLE_ID_COLUMN_WIDTH)
    } else {
        px(TABLE_FIELD_COLUMN_WIDTH)
    }
}

fn is_editable_bson_value(value: &Bson) -> bool {
    matches!(
        value,
        Bson::String(_)
            | Bson::Int32(_)
            | Bson::Int64(_)
            | Bson::Double(_)
            | Bson::Boolean(_)
            | Bson::DateTime(_)
            | Bson::Null
    )
}

fn is_safe_update_field_name(field: &str) -> bool {
    field != "_id" && !field.contains('.') && !field.starts_with('$')
}

fn edit_input_text(value: &Bson) -> String {
    match value {
        Bson::Null => String::new(),
        _ => table_cell_text(Some(value)),
    }
}

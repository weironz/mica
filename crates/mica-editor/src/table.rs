//! Table block model backed directly by `mica_md::decode_cell` / `encode_cell`.
//!
//! Mirrors `clients/mica_flutter/lib/editor/table.dart` and `table_copy.dart`,
//! keeping zero Markdown syntax logic inside `mica-editor`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::marks::{EditorMark, marks_equal};
use crate::model::ClipboardPayload;

/// One table cell: plain text plus UTF-16 inline marks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableCell {
  pub text: String,
  #[serde(default)]
  pub marks: Vec<EditorMark>,
}

impl TableCell {
  pub fn new(text: impl Into<String>, marks: Vec<EditorMark>) -> Self {
    Self {
      text: text.into(),
      marks,
    }
  }

  pub fn plain(text: impl Into<String>) -> Self {
    Self {
      text: text.into(),
      marks: Vec::new(),
    }
  }

  pub fn empty() -> Self {
    Self::default()
  }

  pub fn is_empty(&self) -> bool {
    self.text.is_empty()
  }
}

impl PartialEq for TableCell {
  fn eq(&self, other: &Self) -> bool {
    self.text == other.text && marks_equal(&self.marks, &other.marks)
  }
}

impl Eq for TableCell {}

/// Cell navigation direction inside a table grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableNavDirection {
  Tab,
  ShiftTab,
  Up,
  Down,
  Left,
  Right,
}

/// Outcome of navigating from `(row, col)` in a [`TableModel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum TableNavResult {
  Cell {
    row: usize,
    col: usize,
  },
  /// Pressing Tab in the bottom-right cell inserts a new trailing row and focuses `(row, 0)`.
  InsertRowAndFocus {
    row: usize,
    col: usize,
  },
  /// Left/Up/Shift+Tab past `(0, 0)` exits to the block above.
  ExitBefore,
  /// Right/Down past the bottom edge exits to (or creates) the block below.
  ExitAfter,
}

/// Table manipulation operations dispatched via `EditorCommand::TableOp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TableOp {
  SetCell {
    row: usize,
    col: usize,
    cell: TableCell,
  },
  ClearCells {
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
  },
  InsertRow {
    at: usize,
  },
  DeleteRow {
    at: usize,
  },
  MoveRow {
    row: usize,
    delta: isize,
  },
  InsertColumn {
    at: usize,
  },
  DeleteColumn {
    at: usize,
  },
  MoveColumn {
    col: usize,
    delta: isize,
  },
  SetAlign {
    align: String,
  },
  SetColumnAlign {
    col: usize,
    align: String,
  },
  SetColumnWidths {
    widths: Vec<f64>,
  },
  ResetColumnWidths,
  SetTableWidth {
    width: f64,
  },
  SetLayout {
    widths: Vec<f64>,
    width: f64,
  },
  Navigate {
    row: usize,
    col: usize,
    direction: TableNavDirection,
  },
  CopyArea {
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
  },
  CopyWhole,
}

/// Decoded in-memory table model matching `TableData` in `table.dart`.
#[derive(Debug, Clone, PartialEq)]
pub struct TableModel {
  pub rows: Vec<Vec<TableCell>>,
  pub header: bool,
  pub align: String,
  pub table_width: f64,
  pub widths: Vec<f64>,
  pub aligns: Option<Vec<String>>,
}

impl TableModel {
  pub fn new(
    mut rows: Vec<Vec<TableCell>>,
    header: bool,
    align: impl Into<String>,
    table_width: f64,
    widths: Option<Vec<f64>>,
    aligns: Option<Vec<String>>,
  ) -> Self {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
      while row.len() < cols {
        row.push(TableCell::empty());
      }
    }
    let normalized_widths = normalize_widths(widths, cols);
    let normalized_align = match align.into().as_str() {
      "center" => "center".to_string(),
      "right" => "right".to_string(),
      _ => "left".to_string(),
    };
    Self {
      rows,
      header,
      align: normalized_align,
      table_width: normalize_table_width(table_width),
      widths: normalized_widths,
      aligns,
    }
  }

  /// Default 2×3 empty table (matches `TableData.empty()`).
  pub fn empty() -> Self {
    Self::new(
      vec![
        vec![
          TableCell::plain("Column 1"),
          TableCell::plain("Column 2"),
          TableCell::plain("Column 3"),
        ],
        vec![TableCell::empty(), TableCell::empty(), TableCell::empty()],
      ],
      true,
      "left",
      1.0,
      None,
      None,
    )
  }

  /// Decode from a `table` block's `data` JSON using `mica_md::decode_cell`.
  pub fn from_block_data(data: &Value) -> Self {
    let Some(obj) = data.as_object() else {
      return Self::empty();
    };
    let mut rows: Vec<Vec<TableCell>> = Vec::new();
    if let Some(raw_rows) = obj.get("rows").and_then(Value::as_array) {
      for raw_row in raw_rows {
        let Some(cells) = raw_row.as_array() else {
          continue;
        };
        let row = cells
          .iter()
          .map(|raw_cell| {
            let decoded = mica_md::decode_cell(raw_cell);
            TableCell {
              text: decoded.text,
              marks: decoded.marks.into_iter().map(EditorMark::from).collect(),
            }
          })
          .collect();
        rows.push(row);
      }
    }
    if rows.is_empty() {
      return Self::empty();
    }
    let header = obj.get("header").and_then(Value::as_bool).unwrap_or(true);
    let align = obj
      .get("align")
      .and_then(Value::as_str)
      .unwrap_or("left")
      .to_string();
    let table_width =
      normalize_table_width(obj.get("width").and_then(Value::as_f64).unwrap_or(1.0));
    let widths = obj.get("widths").and_then(Value::as_array).map(|arr| {
      arr
        .iter()
        .map(|v| v.as_f64().unwrap_or(1.0))
        .collect::<Vec<_>>()
    });
    let aligns = obj.get("aligns").and_then(Value::as_array).map(|arr| {
      arr
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
    });
    Self::new(rows, header, align, table_width, widths, aligns)
  }

  /// Encode back into block `data` JSON using `mica_md::encode_cell`.
  pub fn to_block_data(&self) -> Value {
    let out_rows: Vec<Value> = self
      .rows
      .iter()
      .map(|row| {
        let cells: Vec<Value> = row
          .iter()
          .map(|cell| {
            let inline = mica_md::InlineText {
              text: cell.text.clone(),
              marks: cell.marks.iter().map(mica_md::Mark::from).collect(),
            };
            mica_md::encode_cell(&inline)
          })
          .collect();
        Value::Array(cells)
      })
      .collect();
    let mut map = Map::new();
    map.insert("rows".to_string(), Value::Array(out_rows));
    map.insert("header".to_string(), Value::Bool(self.header));
    map.insert("align".to_string(), Value::String(self.align.clone()));
    map.insert("widths".to_string(), json!(self.widths));
    if let Some(aligns) = &self.aligns
      && aligns.iter().any(|a| !a.is_empty())
    {
      map.insert("aligns".to_string(), json!(aligns));
    }
    if (self.table_width - 1.0).abs() > f64::EPSILON {
      map.insert("width".to_string(), json!(self.table_width));
    }
    Value::Object(map)
  }

  pub fn row_count(&self) -> usize {
    self.rows.len()
  }

  pub fn columns(&self) -> usize {
    self.rows.first().map_or(0, Vec::len)
  }

  /// Effective alignment for column `col`.
  pub fn align_for(&self, col: usize) -> &str {
    if let Some(aligns) = &self.aligns
      && let Some(a) = aligns.get(col)
      && !a.is_empty()
    {
      return a.as_str();
    }
    self.align.as_str()
  }

  pub fn set_align(&mut self, align: &str) {
    self.align = match align {
      "center" => "center".to_string(),
      "right" => "right".to_string(),
      _ => "left".to_string(),
    };
    self.aligns = None;
  }

  pub fn set_column_align(&mut self, col: usize, align: &str) -> bool {
    let cols = self.columns();
    if col >= cols {
      return false;
    }
    let normalized = match align {
      "left" => "left",
      "center" => "center",
      "right" => "right",
      _ => "",
    };
    let aligns = self.aligns.get_or_insert_with(|| vec![String::new(); cols]);
    while aligns.len() < cols {
      aligns.push(String::new());
    }
    aligns[col] = normalized.to_string();
    true
  }

  pub fn set_cell(&mut self, row: usize, col: usize, cell: TableCell) -> bool {
    if row >= self.rows.len() || col >= self.rows[row].len() {
      return false;
    }
    if self.rows[row][col] == cell {
      return false;
    }
    self.rows[row][col] = cell;
    true
  }

  pub fn clear_cells(&mut self, r0: usize, c0: usize, r1: usize, c1: usize) -> bool {
    let mut changed = false;
    let r_lo = r0.min(r1);
    let r_hi = r0.max(r1);
    let c_lo = c0.min(c1);
    let c_hi = c0.max(c1);
    for r in r_lo..=r_hi.min(self.rows.len().saturating_sub(1)) {
      let row_len = self.rows[r].len();
      if row_len == 0 {
        continue;
      }
      for cell in self.rows[r]
        .iter_mut()
        .take(c_hi.min(row_len - 1) + 1)
        .skip(c_lo)
      {
        if !cell.is_empty() {
          *cell = TableCell::empty();
          changed = true;
        }
      }
    }
    changed
  }

  pub fn insert_row(&mut self, at: usize) {
    let cols = self.columns().max(1);
    let idx = at.min(self.rows.len());
    self.rows.insert(idx, vec![TableCell::empty(); cols]);
  }

  pub fn delete_row(&mut self, at: usize) -> bool {
    if self.rows.len() <= 1 || at >= self.rows.len() {
      return false;
    }
    self.rows.remove(at);
    true
  }

  /// Move a body row by `delta` (header row stays put when `header == true`).
  pub fn move_row(&mut self, row: usize, delta: isize) -> bool {
    let first = usize::from(self.header);
    let target = row as isize + delta;
    if row < first || target < first as isize {
      return false;
    }
    let to = target as usize;
    if row >= self.rows.len() || to >= self.rows.len() || row == to {
      return false;
    }
    let moved = self.rows.remove(row);
    self.rows.insert(to, moved);
    true
  }

  pub fn insert_column(&mut self, at: usize) {
    let cols = self.columns();
    let idx = at.min(cols);
    for row in &mut self.rows {
      let ridx = idx.min(row.len());
      row.insert(ridx, TableCell::empty());
    }
    let widx = idx.min(self.widths.len());
    self.widths.insert(widx, 1.0);
    if let Some(aligns) = &mut self.aligns {
      let aidx = idx.min(aligns.len());
      aligns.insert(aidx, String::new());
    }
  }

  pub fn delete_column(&mut self, at: usize) -> bool {
    if self.columns() <= 1 || at >= self.columns() {
      return false;
    }
    for row in &mut self.rows {
      if at < row.len() {
        row.remove(at);
      }
    }
    if at < self.widths.len() {
      self.widths.remove(at);
    }
    if let Some(aligns) = &mut self.aligns
      && at < aligns.len()
    {
      aligns.remove(at);
    }
    true
  }

  /// Move column `col` by `delta`; cell contents, column width, and per-column
  /// alignment move together.
  pub fn move_column(&mut self, col: usize, delta: isize) -> bool {
    let cols = self.columns();
    let target = col as isize + delta;
    if target < 0 {
      return false;
    }
    let to = target as usize;
    if col >= cols || to >= cols || col == to {
      return false;
    }
    for row in &mut self.rows {
      if col < row.len() && to < row.len() {
        let v = row.remove(col);
        row.insert(to, v);
      }
    }
    if col < self.widths.len() && to < self.widths.len() {
      let w = self.widths.remove(col);
      self.widths.insert(to, w);
    }
    if let Some(aligns) = &mut self.aligns
      && col < aligns.len()
      && to < aligns.len()
    {
      let a = aligns.remove(col);
      aligns.insert(to, a);
    }
    true
  }

  pub fn set_column_widths(&mut self, widths: &[f64]) {
    for (dst, &src) in self.widths.iter_mut().zip(widths.iter()) {
      *dst = if src <= 0.0 || !src.is_finite() {
        0.05
      } else {
        src
      };
    }
  }

  pub fn reset_column_widths(&mut self) {
    self.widths.fill(1.0);
  }

  pub fn set_table_width(&mut self, width: f64) {
    if width.is_finite() {
      self.table_width = normalize_table_width(width);
    }
  }

  pub fn set_layout(&mut self, widths: &[f64], width: f64) {
    self.set_column_widths(widths);
    self.set_table_width(width);
  }

  /// Compute Tab / Shift+Tab / Arrow navigation from `(row, col)`.
  pub fn navigate(&self, row: usize, col: usize, direction: TableNavDirection) -> TableNavResult {
    let rows = self.row_count();
    let cols = self.columns();
    if rows == 0 || cols == 0 {
      return TableNavResult::ExitAfter;
    }
    let r = row.min(rows - 1);
    let c = col.min(cols - 1);
    match direction {
      TableNavDirection::Tab => {
        if c + 1 < cols {
          TableNavResult::Cell { row: r, col: c + 1 }
        } else if r + 1 < rows {
          TableNavResult::Cell { row: r + 1, col: 0 }
        } else {
          TableNavResult::InsertRowAndFocus { row: rows, col: 0 }
        }
      }
      TableNavDirection::ShiftTab => {
        if c > 0 {
          TableNavResult::Cell { row: r, col: c - 1 }
        } else if r > 0 {
          TableNavResult::Cell {
            row: r - 1,
            col: cols - 1,
          }
        } else {
          TableNavResult::ExitBefore
        }
      }
      TableNavDirection::Right => {
        if c + 1 < cols {
          TableNavResult::Cell { row: r, col: c + 1 }
        } else if r + 1 < rows {
          TableNavResult::Cell { row: r + 1, col: 0 }
        } else {
          TableNavResult::ExitAfter
        }
      }
      TableNavDirection::Left => {
        if c > 0 {
          TableNavResult::Cell { row: r, col: c - 1 }
        } else if r > 0 {
          TableNavResult::Cell {
            row: r - 1,
            col: cols - 1,
          }
        } else {
          TableNavResult::ExitBefore
        }
      }
      TableNavDirection::Up => {
        if r > 0 {
          TableNavResult::Cell { row: r - 1, col: c }
        } else {
          TableNavResult::ExitBefore
        }
      }
      TableNavDirection::Down => {
        if r + 1 < rows {
          TableNavResult::Cell { row: r + 1, col: c }
        } else {
          TableNavResult::ExitAfter
        }
      }
    }
  }

  /// Copy an inclusive rectangular cell selection `[r0..=r1] × [c0..=c1]` as
  /// TSV plain text, GFM Markdown, and Clipboard-profile HTML via `mica_md`.
  pub fn copy_area(&self, r0: usize, c0: usize, r1: usize, c1: usize) -> ClipboardPayload {
    if self.rows.is_empty() || self.columns() == 0 {
      return ClipboardPayload::default();
    }
    let r_lo = r0.min(r1).min(self.rows.len() - 1);
    let r_hi = r0.max(r1).min(self.rows.len() - 1);
    let c_lo = c0.min(c1).min(self.columns() - 1);
    let c_hi = c0.max(c1).min(self.columns() - 1);

    let plain_rows: Vec<String> = (r_lo..=r_hi)
      .map(|r| {
        (c_lo..=c_hi)
          .map(|c| self.rows[r][c].text.replace(['\t', '\n'], " "))
          .collect::<Vec<_>>()
          .join("\t")
      })
      .collect();
    let plain = plain_rows.join("\n");

    let sub_rows: Vec<Vec<TableCell>> = (r_lo..=r_hi)
      .map(|r| self.rows[r][c_lo..=c_hi].to_vec())
      .collect();
    let sub_aligns = self.aligns.as_ref().map(|a| {
      (c_lo..=c_hi)
        .map(|c| a.get(c).cloned().unwrap_or_default())
        .collect::<Vec<_>>()
    });
    let sub_table = TableModel::new(
      sub_rows,
      self.header,
      self.align.clone(),
      1.0,
      None,
      sub_aligns,
    );
    let block = mica_md::Block {
      id: String::new(),
      kind: "table".to_string(),
      text: String::new(),
      data: sub_table.to_block_data(),
      children: Vec::new(),
    };
    let markdown = mica_md::serialize_body(
      std::slice::from_ref(&block),
      &mica_md::SerializeOptions::default(),
    );
    let html = mica_md::render_html_body(
      std::slice::from_ref(&block),
      &mica_md::HtmlOptions {
        profile: mica_md::HtmlProfile::Clipboard,
        ..Default::default()
      },
    );
    ClipboardPayload {
      plain,
      markdown,
      html,
    }
  }

  /// Copy the entire table.
  pub fn copy_whole(&self) -> ClipboardPayload {
    if self.rows.is_empty() || self.columns() == 0 {
      return ClipboardPayload::default();
    }
    self.copy_area(0, 0, self.rows.len() - 1, self.columns() - 1)
  }
}

fn normalize_table_width(width: f64) -> f64 {
  if !width.is_finite() || width <= 0.0 {
    1.0
  } else {
    width.clamp(0.15, 4.0)
  }
}

fn normalize_widths(widths: Option<Vec<f64>>, columns: usize) -> Vec<f64> {
  if columns == 0 {
    return Vec::new();
  }
  match widths {
    Some(w) if w.len() == columns => w
      .into_iter()
      .map(|v| if v <= 0.0 || !v.is_finite() { 1.0 } else { v })
      .collect(),
    _ => vec![1.0; columns],
  }
}

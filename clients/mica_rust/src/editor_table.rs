//! Interactive table block view backed directly by `mica_editor::TableModel` & `TableOp`.

use std::rc::Rc;

use mica_editor::{EditorCommand, TableCell, TableModel, TableOp};
use nsh_base::{button, input, theme};
use nsh_core::prelude::*;

/// Renders a table block with live cell editing and row/column mutations.
pub fn table_block_view(
  block_index: usize,
  data: &serde_json::Value,
  dispatch: Rc<dyn Fn(EditorCommand)>,
) -> impl IntoElement + use<> {
  let model = TableModel::from_block_data(data);
  let row_count = model.row_count();
  let col_count = model.rows.first().map(|r| r.len()).unwrap_or(0);

  let d_add_row = dispatch.clone();
  let d_add_col = dispatch.clone();
  let d_del_row = dispatch.clone();
  let d_del_col = dispatch.clone();

  let mut container = view()
    .w_full()
    .p_2()
    .gap_2()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().card)
    .child(
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1()
        .child(
          text(format!("Table ({row_count}×{col_count})"))
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
        .child(table_btn("+ Row", "table-add-row", move || {
          d_add_row(EditorCommand::TableOp {
            block_index,
            op: TableOp::InsertRow { at: row_count },
          });
        }))
        .child(table_btn("+ Col", "table-add-col", move || {
          d_add_col(EditorCommand::TableOp {
            block_index,
            op: TableOp::InsertColumn { at: col_count },
          });
        }))
        .child(table_btn("- Row", "table-del-row", move || {
          if row_count > 1 {
            d_del_row(EditorCommand::TableOp {
              block_index,
              op: TableOp::DeleteRow { at: row_count - 1 },
            });
          }
        }))
        .child(table_btn("- Col", "table-del-col", move || {
          if col_count > 1 {
            d_del_col(EditorCommand::TableOp {
              block_index,
              op: TableOp::DeleteColumn { at: col_count - 1 },
            });
          }
        })),
    );

  for (r, row) in model.rows.iter().enumerate() {
    let mut row_el = view().flex_row().w_full().gap_1();
    for (c, cell) in row.iter().enumerate() {
      let d_cell = dispatch.clone();
      let initial = cell.text.clone();
      let cell_tid = format!("table-cell-{r}-{c}");
      row_el = row_el.child(
        input()
          .test_id(&cell_tid)
          .flex_1()
          .px_2()
          .py_1()
          .border_1()
          .border_color(move || theme().border)
          .rounded(move || theme().radius)
          .text_color(move || theme().foreground)
          .value(initial)
          .on_change(move |val: &str| {
            d_cell(EditorCommand::TableOp {
              block_index,
              op: TableOp::SetCell {
                row: r,
                col: c,
                cell: TableCell::plain(val),
              },
            });
          }),
      );
    }
    container = container.child(row_el);
  }

  container
}

fn table_btn(label: &'static str, tid: &'static str, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid)
    .px_2()
    .py_1()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().secondary)
    .on_press(on_click)
    .child(
      text(label)
        .text_xs()
        .text_color(move || theme().secondary_foreground),
    )
}

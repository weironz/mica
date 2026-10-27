//! Raw wire bytes must not depend on serde_json's preserve_order feature.
use mica_md::wire;

#[test]
fn block_data_objects_use_default_key_order_recursively() {
  assert_eq!(
    wire::parse_body_json("**x**", false).unwrap(),
    r#"[{"type":"paragraph","text":"x","data":{"marks":[{"end":1,"start":0,"type":"bold"}]}}]"#
  );
}

#[test]
fn encoded_cells_keep_struct_and_value_object_orders_distinct() {
  let input =
    r#"[{"text":"😀x","marks":[{"start":2,"end":3,"type":"link","href":"/x","title":"t"}]}]"#;
  assert_eq!(
    wire::encode_cells_json(input).unwrap(),
    r#"[{"marks":[{"end":3,"href":"/x","start":2,"title":"t","type":"link"}],"text":"😀x"}]"#
  );
  // InlineJson/Mark are structs: their existing field declaration order stays.
  assert_eq!(
    wire::decode_cells_json(&wire::encode_cells_json(input).unwrap()).unwrap(),
    r#"[{"text":"😀x","marks":[{"start":2,"end":3,"type":"link","href":"/x","title":"t"}]}]"#
  );
}

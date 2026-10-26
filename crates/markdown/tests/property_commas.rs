use mica_markdown::properties::{PropertyValue, parse_properties, upsert_property};

#[test]
fn list_items_with_any_comma_remain_single_values() {
  for item in ["a,b", "a, b", "a,,b", ",a", "a,"] {
    let value = PropertyValue::List(vec![item.into()]);
    let written = upsert_property("", "tags", &value);
    assert_eq!(parse_properties(&written)[0].value, value, "{written}");
  }
  assert_eq!(
    upsert_property(
      "title: Page",
      "tags",
      &PropertyValue::List(vec!["a".into(), "b".into()])
    ),
    "title: Page\ntags: [a, b]"
  );
}

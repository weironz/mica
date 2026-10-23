//! Limits for short names stored in API metadata and broadcast to workspace members.
use mica_infra::{ApiError, ApiResult};

const MAX_NAME_CHARS: usize = 255;

pub(super) fn normalize_name(value: &str, field: &str) -> ApiResult<String> {
  let value = value.trim();
  if value.is_empty() {
    return Err(ApiError::BadRequest(format!("{field} is required")));
  }
  if value.chars().take(MAX_NAME_CHARS + 1).count() > MAX_NAME_CHARS {
    return Err(ApiError::BadRequest(format!(
      "{field} must be at most {MAX_NAME_CHARS} characters"
    )));
  }
  Ok(value.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_are_trimmed_and_bounded_by_unicode_characters() {
    for field in ["display_name", "workspace name", "name"] {
      assert_eq!(normalize_name("  名称  ", field).unwrap(), "名称");
      assert!(normalize_name(" \n ", field).is_err());
      assert!(normalize_name(&"名".repeat(255), field).is_ok());
      assert!(normalize_name(&"名".repeat(256), field).is_err());
      assert!(normalize_name(&"a".repeat(2 * 1024 * 1024), field).is_err());
    }
  }
}

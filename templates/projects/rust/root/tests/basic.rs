#[test]
fn binary_name_is_not_empty() {
    assert!(!env!("CARGO_PKG_NAME").is_empty());
}

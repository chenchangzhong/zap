use crate::{
    cloud_object::{Owner, Space},
    search::notebook_embedding::is_embed_accessible,
};

#[test]
fn test_embed_in_personal_object() {
    assert!(is_embed_accessible(
        Space::Personal,
        Owner::mock_current_user()
    ));
}

#[test]
fn test_embed_in_shared_object() {
    assert!(!is_embed_accessible(
        Space::Shared,
        Owner::mock_current_user()
    ));
}

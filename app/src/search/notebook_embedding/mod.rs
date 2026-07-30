use crate::cloud_object::{Owner, Space};

mod embedded_fuzzy_match;
mod notebooks;
pub mod searcher;
pub mod view;
mod workflows;

/// Tests if an object owned by `object_owner` is accessible to all users with permissions to
/// `embedding_space`.
fn is_embed_accessible(embedding_space: Space, _object_owner: Owner) -> bool {
    match embedding_space {
        // If embedding in a personal object, _all_ objects accessible to the client are visible.
        Space::Personal => true,
        // TODO: Revisit the UX here, as the user doesn't know who else can see the object.
        Space::Shared => false,
    }
}

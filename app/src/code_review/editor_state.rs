use std::rc::Rc;

use warp_util::file::FileLoadError;
use warpui::elements::MouseStateHandle;
use warpui::{AppContext, ViewHandle};

use crate::code::local_code_editor::LocalCodeEditorView;

pub struct CodeReviewEditorState {
    pub editor: ViewHandle<LocalCodeEditorView>,
    unsaved_changes_mouse_state: MouseStateHandle,
    pub(super) editor_mouse_state: MouseStateHandle,
    /// Whether the buffer content has been loaded from disk (for global buffer mode).
    /// This is set to true when LocalCodeEditorEvent::DelayedRenderingFlushed or FailedToLoad fires.
    is_loaded: bool,
    /// The load error, if the buffer failed to load (e.g. an oversized file).
    load_error: Option<Rc<FileLoadError>>,
}

impl CodeReviewEditorState {
    #[cfg(not(target_family = "wasm"))]
    pub fn new(editor: ViewHandle<LocalCodeEditorView>) -> Self {
        Self {
            editor,
            unsaved_changes_mouse_state: MouseStateHandle::default(),
            editor_mouse_state: MouseStateHandle::default(),
            is_loaded: false,
            load_error: None,
        }
    }

    /// Creates a new editor state that is already marked as loaded.
    /// Used for non-global buffer mode where content is loaded synchronously.
    pub fn new_loaded(editor: ViewHandle<LocalCodeEditorView>) -> Self {
        Self {
            editor,
            unsaved_changes_mouse_state: MouseStateHandle::default(),
            editor_mouse_state: MouseStateHandle::default(),
            is_loaded: true,
            load_error: None,
        }
    }

    /// Returns whether the buffer content has been loaded.
    pub fn is_loaded(&self) -> bool {
        self.is_loaded
    }

    /// Marks the editor as loaded.
    pub fn set_loaded(&mut self) {
        self.is_loaded = true;
    }

    /// Records the load outcome, including the error when the buffer failed to load.
    pub fn set_load_result(&mut self, error: Option<Rc<FileLoadError>>) {
        self.is_loaded = true;
        self.load_error = error;
    }

    pub fn load_error(&self) -> Option<&FileLoadError> {
        self.load_error.as_deref()
    }

    pub fn editor(&self) -> &ViewHandle<LocalCodeEditorView> {
        &self.editor
    }

    pub fn unsaved_changes_mouse_state(&self) -> MouseStateHandle {
        self.unsaved_changes_mouse_state.clone()
    }

    pub fn has_unsaved_changes(&self, ctx: &AppContext) -> bool {
        self.editor.as_ref(ctx).has_unsaved_changes(ctx)
    }
}

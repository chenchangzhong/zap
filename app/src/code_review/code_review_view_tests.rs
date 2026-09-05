use super::*;
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::code::local_code_editor::LocalCodeEditorView;
use crate::code_review::comments::{
    attach_pending_imported_comments, AttachedReviewComment, AttachedReviewCommentTarget,
    CommentId, CommentOrigin, LineDiffContent, PendingImportedReviewComment,
    PendingImportedReviewCommentTarget,
};
use crate::code_review::diff_size_limits::DiffSize;
use crate::code_review::diff_state::{DiffStateModel, FileDiff, GitFileStatus};
use crate::code_review::editor_state::CodeReviewEditorState;
use crate::code_review::GlobalCodeReviewModel;
use crate::pane_group::WorkingDirectoriesModel;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::local_shell::LocalShellState;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::ActiveSession;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::NotebookKeybindings;
use ai::agent::action::InsertReviewComment;
use chrono::Local;
use repo_metadata::repositories::DetectedRepositories;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use string_offset::CharOffset;
use warp_core::ui::appearance::Appearance;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_editor::render::model::LineCount;
use warpui::elements::{Empty, MouseStateHandle};
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle};

#[derive(Default)]
struct TestView;

impl warpui::Entity for TestView {
    type Event = ();
}

impl warpui::View for TestView {
    fn render(&self, _: &warpui::AppContext) -> Box<dyn warpui::Element> {
        Empty::new().finish()
    }

    fn ui_name() -> &'static str {
        "TestView"
    }
}

impl warpui::TypedActionView for TestView {
    type Action = ();
}

/// Initialize required singletons for testing
fn initialize_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(|_| LocalShellState::NotLoaded);
    app.add_singleton_model(|_| GlobalCodeReviewModel);
    app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![], ctx));

    // Add mocks required by rich text editor (used in the CommentEditor)
    app.add_singleton_model(ObjectStoreModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(AIRequestUsageModel::new_for_test);
}

/// Creates a LocalCodeEditorView with the given content
fn create_editor_with_content(app: &mut App, content: &str) -> ViewHandle<LocalCodeEditorView> {
    let content = content.to_string();
    let (_, local_editor) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        let code_editor_view = ctx.add_typed_action_view(|ctx| {
            CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
                ctx,
            )
        });

        code_editor_view.update(ctx, |editor, ctx| {
            editor.reset(InitialBufferState::plain_text(&content), ctx);
        });

        LocalCodeEditorView::new(code_editor_view, None, false, None, ctx)
    });

    local_editor
}

/// Creates a LocalCodeEditorView with base and current content for diff testing
#[allow(dead_code)]
fn create_editor_with_diff(
    app: &mut App,
    base_content: &str,
    current_content: &str,
) -> ViewHandle<LocalCodeEditorView> {
    let current = current_content.to_string();
    let base = base_content.to_string();
    let (_, local_editor) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
        let code_editor_view = ctx.add_typed_action_view(|ctx| {
            CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
                ctx,
            )
        });

        code_editor_view.update(ctx, |editor, ctx| {
            editor.reset(InitialBufferState::plain_text(&current), ctx);
            editor.set_base(&base, true, ctx);
        });

        LocalCodeEditorView::new(code_editor_view, None, false, None, ctx)
    });

    local_editor
}

/// Creates an attached review comment with a Line target
fn create_line_comment(
    file_path: impl Into<PathBuf>,
    line_number: usize,
    line_text: &str,
    comment_content: &str,
) -> AttachedReviewComment {
    let line_count = LineCount::from(line_number);
    AttachedReviewComment {
        id: CommentId::new(),
        content: comment_content.to_string(),
        target: AttachedReviewCommentTarget::Line {
            absolute_file_path: file_path.into(),
            line: EditorLineLocation::Current {
                line_number: line_count,
                line_range: line_count..LineCount::from(line_number + 1),
            },
            content: LineDiffContent {
                content: format!("+{line_text}"),
                lines_added: LineCount::from(1),
                lines_removed: LineCount::from(0),
            },
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    }
}

/// Creates an attached review comment with a File target
fn create_file_comment(
    file_path: impl Into<PathBuf>,
    comment_content: &str,
) -> AttachedReviewComment {
    AttachedReviewComment {
        id: CommentId::new(),
        content: comment_content.to_string(),
        target: AttachedReviewCommentTarget::File {
            absolute_file_path: file_path.into(),
        },
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    }
}

/// Creates an attached review comment with a General target
fn create_general_comment(comment_content: &str) -> AttachedReviewComment {
    AttachedReviewComment {
        id: CommentId::new(),
        content: comment_content.to_string(),
        target: AttachedReviewCommentTarget::General,
        last_update_time: Local::now(),
        base: None,
        head: None,
        outdated: false,
        origin: CommentOrigin::Native,
    }
}

fn make_pending_comment(
    id: &str,
    author: &str,
    body: &str,
    parent_id: Option<&str>,
    timestamp: &str,
    target: PendingImportedReviewCommentTarget,
) -> PendingImportedReviewComment {
    let mut pending = PendingImportedReviewComment::try_from(InsertReviewComment {
        comment_id: id.to_string(),
        author: author.to_string(),
        comment_body: body.to_string(),
        parent_comment_id: parent_id.map(|s| s.to_string()),
        last_modified_timestamp: timestamp.to_string(),
        comment_location: None,
        html_url: None,
    })
    .expect("valid pending import conversion");

    // Override the location target since we intentionally use `comment_location: None` above.
    pending.target = target;

    pending
}

use crate::view_components::action_button::{ActionButton, NakedTheme};

/// Test context that holds all common test state
struct TestContext {
    repo_path: PathBuf,
    #[allow(dead_code)]
    window_id: warpui::WindowId,
    state: LoadedState,
    code_review_view: ViewHandle<CodeReviewView>,
}

impl TestContext {
    /// Initialize common test state with a single file editor
    fn new(app: &mut App, file_path: PathBuf, editor_content: &str) -> Self {
        initialize_test_app(app);

        let editor = create_editor_with_content(app, editor_content);
        let repo_path = PathBuf::from("/repo");

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| TestView);
        let state = create_loaded_state_with_editors(app, window_id, vec![(file_path, editor)]);

        let diff_state_model = app.add_model(|ctx| DiffStateModel::new(None, ctx));

        let working_directories_model = app.add_model(|_| WorkingDirectoriesModel::new());
        let code_review_comment_batch =
            working_directories_model.update(app, |working_directories, ctx| {
                working_directories.get_or_create_code_review_comments(repo_path.as_path(), ctx)
            });

        let code_review_view = app.add_view(window_id, |ctx| {
            CodeReviewView::new(
                Some(repo_path.clone()),
                diff_state_model,
                code_review_comment_batch,
                None,
                false,
                ctx,
            )
        });

        Self {
            repo_path,
            window_id,
            state,
            code_review_view,
        }
    }
}

/// Creates a minimal LoadedState with file states containing editors.
/// Must be called within an App context.
fn create_loaded_state_with_editors(
    app: &mut App,
    window_id: warpui::WindowId,
    file_editors: Vec<(PathBuf, ViewHandle<LocalCodeEditorView>)>,
) -> LoadedState {
    let file_states = file_editors
        .into_iter()
        .map(|(file_path, editor)| {
            let chevron_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let open_in_tab_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let discard_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let add_context_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let copy_path_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let prev_hunk_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));
            let next_hunk_button = app.add_view(window_id, |_| ActionButton::new("", NakedTheme));

            let state = FileState {
                file_diff: FileDiff {
                    file_path: file_path.clone(),
                    status: GitFileStatus::Modified,
                    hunks: Arc::new(vec![]),
                    is_binary: false,
                    is_autogenerated: false,
                    max_line_number: 0,
                    has_hidden_bidi_chars: false,
                    size: DiffSize::Normal,
                },
                editor_state: Some(CodeReviewEditorState::new_loaded(editor)),
                is_expanded: true,
                sidebar_mouse_state: MouseStateHandle::default(),
                header_mouse_state: MouseStateHandle::default(),
                chevron_button,
                open_in_tab_button,
                discard_button,
                add_context_button,
                copy_path_button,
                prev_hunk_button,
                next_hunk_button,
                side_by_side_state: None,
                content_at_head: None,
                side_by_side_diff_token: 0,
                pending_diff_abort: None,
            };
            (file_path, state)
        })
        .collect();

    LoadedState {
        file_states,
        total_additions: 0,
        total_deletions: 0,
        files_changed: 0,
        side_by_side_diff_cache: HashMap::new(),
    }
}

#[test]
fn test_relocate_comments_empty_input() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(vec![], &ctx.state, &ctx.repo_path, view_ctx);

            assert!(
                relocated.is_empty(),
                "Empty input should return empty output"
            );
            assert_eq!(fallbacks, 0, "Empty input should have no fallbacks");
        });
    });
}

#[test]
fn test_relocate_comments_general_comment_passes_through() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        let general_comment = create_general_comment("This is a general comment");
        let original_id = general_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![general_comment],
                &ctx.state,
                &ctx.repo_path,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Should return the comment");
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                matches!(relocated[0].target, AttachedReviewCommentTarget::General),
                "General comment should remain General"
            );
            assert_eq!(
                fallbacks, 0,
                "General comments should not count as fallbacks"
            );
        });
    });
}

#[test]
fn test_relocate_comments_file_comment_passes_through() {
    App::test((), |mut app| async move {
        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 2\nline 3");

        let file_comment =
            create_file_comment(ctx.repo_path.join(&file_path), "This is a file comment");
        let original_id = file_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![file_comment],
                &ctx.state,
                &ctx.repo_path,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Should return the comment");
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                matches!(
                    relocated[0].target,
                    AttachedReviewCommentTarget::File { .. }
                ),
                "File comment should remain File"
            );
            assert_eq!(fallbacks, 0, "File comments should not count as fallbacks");
        });
    });
}

#[test]
fn test_relocate_comments_line_comment_no_matching_editor_marked_outdated() {
    App::test((), |mut app| async move {
                // Editor is for "test.txt" but comment is for "other.txt"
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        let line_comment =
            create_line_comment("/repo/other.txt", 1, "line 1", "Comment on other file");
        let original_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![line_comment],
                &ctx.state,
                &ctx.repo_path,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                1,
                "Comment with no matching editor should be kept but marked outdated"
            );
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                relocated[0].outdated,
                "Comment should be marked as outdated"
            );
            assert_eq!(
                fallbacks, 0,
                "Outdated comments should not count as fallbacks"
            );
        });
    });
}

#[test]
fn test_relocate_comments_multiple_comment_types() {
    App::test((), |mut app| async move {
        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 2\nline 3");

        let general_comment = create_general_comment("General comment");
        let file_comment = create_file_comment(ctx.repo_path.join(&file_path), "File comment");
        let line_comment = create_line_comment("/repo/test.txt", 1, "line 1", "Line comment");

        let general_id = general_comment.id;
        let file_id = file_comment.id;
        let line_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let comments = vec![general_comment, file_comment, line_comment];
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: _,
            } = CodeReviewView::relocate_comments(comments, &ctx.state, &ctx.repo_path, view_ctx);

            assert_eq!(
                relocated.len(),
                3,
                "Should return all comments (general, file, and line)"
            );

            // Find each comment by ID
            let relocated_general = relocated.iter().find(|c| c.id == general_id).unwrap();
            let relocated_file = relocated.iter().find(|c| c.id == file_id).unwrap();
            let relocated_line = relocated.iter().find(|c| c.id == line_id).unwrap();

            assert!(matches!(
                relocated_general.target,
                AttachedReviewCommentTarget::General
            ));
            assert!(matches!(
                relocated_file.target,
                AttachedReviewCommentTarget::File { .. }
            ));
            assert!(matches!(
                relocated_line.target,
                AttachedReviewCommentTarget::Line { .. }
            ));
        });
    });
}

#[test]
fn test_relocate_comments_line_comment_with_absolute_path() {
    App::test((), |mut app| async move {
        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 2\nline 3");

        // Comment with absolute path matching the editor's file
        let line_comment = create_line_comment("/repo/test.txt", 1, "line 1", "Line comment");
        let original_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: _,
            } = CodeReviewView::relocate_comments(
                vec![line_comment],
                &ctx.state,
                &ctx.repo_path,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                1,
                "Comment with absolute path should be relocated"
            );
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                matches!(
                    relocated[0].target,
                    AttachedReviewCommentTarget::Line { .. }
                ),
                "Line comment should remain Line"
            );
        });
    });
}

#[test]
fn test_attach_pending_imported_comment_formats_body_and_uses_absolute_path() {
    let repo_path = PathBuf::from("/repo");

    let pending = make_pending_comment(
        "1",
        "alice",
        "Hello world",
        None,
        "2024-01-01T00:00:00Z",
        PendingImportedReviewCommentTarget::Line {
            relative_file_path: PathBuf::from("test.txt"),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(1),
                line_range: LineCount::from(1)..LineCount::from(2),
            },
            diff_content: LineDiffContent {
                content: "+line 1".to_string(),
                lines_added: LineCount::from(1),
                lines_removed: LineCount::from(0),
            },
        },
    );

    let attached = attach_pending_imported_comments(vec![pending], repo_path.as_path());

    assert_eq!(attached.len(), 1);
    assert_eq!(attached[0].content, "**@alice**:\nHello world");

    match &attached[0].target {
        AttachedReviewCommentTarget::Line {
            absolute_file_path, ..
        } => {
            assert_eq!(*absolute_file_path, repo_path.join("test.txt"));
        }
        _ => panic!("expected line comment target"),
    }

    match &attached[0].origin {
        CommentOrigin::ImportedFromGitHub(details) => {
            assert_eq!(details.author, "alice");
            assert_eq!(details.github_comment_id, "1");
            assert!(details.github_parent_id.is_none());
        }
        _ => panic!("expected imported origin"),
    }
}

#[test]
fn test_attach_pending_imported_thread_flattens_depth_first_sorted_by_timestamp() {
    let repo_path = PathBuf::from("/repo");

    let root = make_pending_comment(
        "1",
        "alice",
        "Root",
        None,
        "2024-01-01T00:00:00Z",
        PendingImportedReviewCommentTarget::Line {
            relative_file_path: PathBuf::from("test.txt"),
            line: EditorLineLocation::Current {
                line_number: LineCount::from(1),
                line_range: LineCount::from(1)..LineCount::from(2),
            },
            diff_content: LineDiffContent {
                content: "+line 1".to_string(),
                lines_added: LineCount::from(1),
                lines_removed: LineCount::from(0),
            },
        },
    );

    // Earlier reply to the root.
    let reply_early = make_pending_comment(
        "4",
        "dana",
        "Reply early",
        Some("1"),
        "2024-01-01T00:30:00Z",
        PendingImportedReviewCommentTarget::General,
    );

    // Later reply to the root.
    let reply_late = make_pending_comment(
        "2",
        "bob",
        "Reply later",
        Some("1"),
        "2024-01-01T01:00:00Z",
        PendingImportedReviewCommentTarget::General,
    );

    // Reply to the later reply.
    let reply_nested = make_pending_comment(
        "3",
        "charlie",
        "Nested reply",
        Some("2"),
        "2024-01-01T02:00:00Z",
        PendingImportedReviewCommentTarget::General,
    );

    let latest_timestamp = reply_nested.last_update_time;

    let attached = attach_pending_imported_comments(
        vec![reply_late, root, reply_nested, reply_early],
        repo_path.as_path(),
    );

    assert_eq!(attached.len(), 1);
    assert_eq!(
        attached[0].content,
        "**@alice**:\nRoot\n---\n**@dana**:\nReply early\n---\n**@bob**:\nReply later\n---\n**@charlie**:\nNested reply"
    );
    assert_eq!(attached[0].last_update_time, latest_timestamp);

    match &attached[0].target {
        AttachedReviewCommentTarget::Line {
            absolute_file_path, ..
        } => {
            assert_eq!(*absolute_file_path, repo_path.join("test.txt"));
        }
        _ => panic!("expected root line target to be preserved"),
    }
}

#[test]
fn test_relocate_comments_file_comment_no_matching_editor_marked_outdated() {
    App::test((), |mut app| async move {
                // Editor is for "test.txt" but comment is for "other.txt"
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        let file_comment = create_file_comment("/repo/other.txt", "Comment on other file");
        let original_id = file_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![file_comment],
                &ctx.state,
                &ctx.repo_path,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                1,
                "File comment with no matching editor should be kept but marked outdated"
            );
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                relocated[0].outdated,
                "Comment should be marked as outdated"
            );
            assert_eq!(
                fallbacks, 0,
                "Outdated file comments should not count as fallbacks"
            );
        });
    });
}

#[test]
fn test_relocate_comments_line_removed_marked_outdated() {
    App::test((), |mut app| async move {
                // Editor has "line 1\nline 3" (line 2 was removed)
        // Comment was attached to "line 2" which no longer exists
        let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 3");

        // Create a comment that was attached to "line 2" at line index 1
        let line_comment =
            create_line_comment("/repo/test.txt", 1, "line 2", "Comment on removed line");
        let original_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![line_comment],
                &ctx.state,
                &ctx.repo_path,
                view_ctx,
            );

            assert_eq!(
                relocated.len(),
                1,
                "Comment should be kept even when line content is removed"
            );
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                relocated[0].outdated,
                "Comment should be marked as outdated when line content cannot be found"
            );
            assert_eq!(
                fallbacks, 1,
                "Should count as a fallback when line content cannot be matched"
            );
        });
    });
}

#[test]
fn test_setup_dropdown_with_branches_includes_all_items() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        // Populate branches and compute targets via the selector's build method.
        let target_count = ctx.code_review_view.update(&mut app, |view, view_ctx| {
            if let Some(repo) = view.active_repo.as_mut() {
                repo.available_branches = vec![
                    ("main".to_string(), true),
                    ("feature-1".to_string(), false),
                    ("feature-2".to_string(), false),
                ];
            }
            view.build_diff_targets(view_ctx).len()
        });

        // Verify the selector surfaces all expected items:
        // 1. "Uncommitted changes" (always first)
        // 2. "main" (main branch)
        // 3. "feature-1"
        // 4. "feature-2"
        assert_eq!(
            target_count, 4,
            "Diff selector should have 4 targets: Uncommitted changes + main + 2 feature branches"
        );
    });
}

#[test]
fn test_setup_dropdown_without_branches_only_has_uncommitted_changes() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );

        // Ensure branches are empty (simulates the bug state) and count targets.
        let target_count = ctx.code_review_view.update(&mut app, |view, view_ctx| {
            if let Some(repo) = view.active_repo.as_mut() {
                repo.available_branches = vec![];
            }
            view.build_diff_targets(view_ctx).len()
        });

        assert_eq!(
            target_count, 1,
            "Diff selector should only have 'Uncommitted changes' when no branches are available"
        );
    });
}

#[test]
fn test_on_close_then_on_open_reinitializes_repo_state() {
    App::test((), |mut app| async move {
        let ctx = TestContext::new(
            &mut app,
            PathBuf::from("test.txt"),
            "line 1\nline 2\nline 3",
        );
        let repo_path = ctx.repo_path.clone();

        // Populate branches to simulate a working state
        let target_count_before = ctx.code_review_view.update(&mut app, |view, view_ctx| {
            if let Some(repo) = view.active_repo.as_mut() {
                repo.available_branches =
                    vec![("main".to_string(), true), ("feature-1".to_string(), false)];
            }
            view.build_diff_targets(view_ctx).len()
        });
        assert_eq!(target_count_before, 3, "Should have 3 targets before close");

        // Close the view
        ctx.code_review_view.update(&mut app, |view, view_ctx| {
            view.on_close(view_ctx);
            assert!(!view.is_open, "View should be closed after on_close");
        });

        // Re-open the view
        ctx.code_review_view.update(&mut app, |view, view_ctx| {
            view.on_open(Some(repo_path.clone()), view_ctx);

            assert!(view.is_open, "View should be open after on_open");
            assert_eq!(
                view.repo_path(),
                Some(&repo_path),
                "Repo path should be set after on_open"
            );

            // available_branches should be empty after on_open resets the repo state,
            // because update_current_repo creates a fresh RepositoryState.
            // The async fetch_branches_and_rebuild_diff_selector has been initiated
            // but hasn't completed yet (git command will fail in test env).
            let branches_count = view
                .active_repo
                .as_ref()
                .map(|repo| repo.available_branches.len())
                .unwrap_or(0);
            assert_eq!(
                branches_count, 0,
                "Branches should be empty immediately after on_open (async fetch pending)"
            );
        });
    });
}

#[test]
fn test_handle_edit_comment_scrolls_with_buffer() {
    App::test((), |mut app| async move {
        let file_path = PathBuf::from("test.txt");
        let content = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let ctx = TestContext::new(&mut app, file_path.clone(), &content);

        // Create a line comment targeting this file
        let line_comment = create_line_comment("/repo/test.txt", 5, "line 5", "Review comment");
        let comment_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |view, view_ctx| {
            // Inject the loaded state into the view's active repo
            if let Some(repo) = view.active_repo.as_mut() {
                repo.state = CodeReviewViewState::Loaded(ctx.state);
            }

            // Add the comment to the active comment model so get_comment_by_id can find it
            if let Some(model) = view.active_comment_model.clone() {
                model.update(view_ctx, |batch, ctx| {
                    batch.upsert_comment(line_comment, ctx);
                });
            }

            // Record scroll offset before the edit-comment scroll
            let offset_before = view.viewported_list_state.get_scroll_offset();

            // Call handle_edit_comment — should call scroll_to_line with COMMENT_EDITOR_SCROLL_BUFFER
            view.handle_edit_comment(&comment_id, view_ctx);

            // handle_edit_comment scrolls to the comment line. The scroll offset should
            // include COMMENT_EDITOR_SCROLL_BUFFER (200px) to account for the comment
            // editor that opens below the line.
            // Before the buffer fix, scroll_to_line passed buffer=0.0, so the offset
            // would be smaller. After the fix, it passes COMMENT_EDITOR_SCROLL_BUFFER.
            let offset_after = view.viewported_list_state.get_scroll_offset();
            let scroll_delta = offset_after - offset_before;

            // The scroll delta should include the COMMENT_EDITOR_SCROLL_BUFFER.
            // Without the buffer fix, scroll_delta would be smaller by 200px.
            assert!(
                scroll_delta >= Pixels::new(COMMENT_EDITOR_SCROLL_BUFFER),
                "Scroll delta ({scroll_delta:?}) should be >= COMMENT_EDITOR_SCROLL_BUFFER ({COMMENT_EDITOR_SCROLL_BUFFER}px) to account for the comment editor"
            );
        });
    });
}

#[test]
fn test_active_comments_not_marked_outdated() {
    App::test((), |mut app| async move {
                let file_path = PathBuf::from("test.txt");
        let ctx = TestContext::new(&mut app, file_path.clone(), "line 1\nline 2\nline 3");

        // Comment attached to "line 2" which exists in the editor
        let line_comment =
            create_line_comment("/repo/test.txt", 1, "line 2", "Comment on existing line");
        let original_id = line_comment.id;

        ctx.code_review_view.update(&mut app, |_view, view_ctx| {
            let RelocateCommentsResult {
                comments: relocated,
                fallback_count: fallbacks,
            } = CodeReviewView::relocate_comments(
                vec![line_comment],
                &ctx.state,
                &ctx.repo_path,
                view_ctx,
            );

            assert_eq!(relocated.len(), 1, "Comment should be relocated");
            assert_eq!(relocated[0].id, original_id, "Should preserve comment ID");
            assert!(
                !relocated[0].outdated,
                "Comment should NOT be marked as outdated when line content is found"
            );
            assert_eq!(
                fallbacks, 0,
                "Should have no fallbacks when content matches"
            );
        });
    });
}

// ---------------------------------------------------------------------------
// H-B 优化:word diff 字节 range → CharOffset 的顺序递增游标转换
// ---------------------------------------------------------------------------

/// 顺序推进的游标计数必须与旧实现 `text[..target].chars().count()` 完全一致,
/// 且包含多字节 UTF-8 字符(证明是字符偏移而非字节偏移)。
#[test]
fn test_advance_char_cursor_matches_full_prefix_count() {
    // "héllo 日本語 world":字节边界与字符边界不一致(é 占 2 字节、每个汉字占 3 字节),
    // 目标字节全部落在字符边界上,且单调递增。
    let text = "héllo 日本語 world";
    let boundaries = [1usize, 3, 7, 10, 16, 22];
    let mut byte_cursor = 0usize;
    let mut char_cursor = CharOffset::zero();
    for &target in &boundaries {
        let off = CodeReviewView::advance_char_cursor(text, &mut byte_cursor, char_cursor, target);
        assert_eq!(off, CharOffset::from(text[..target].chars().count()));
        assert_eq!(byte_cursor, target, "游标字节位置应推进到 target");
        char_cursor = off;
    }
    // 推进到文本末尾后,游标等于整串字符数(15,远小于字节数 22)。
    assert_eq!(char_cursor, CharOffset::from(text.chars().count()));
    assert!(char_cursor.as_usize() < text.len(), "多字节字符使字节数大于字符数");
}

/// 多 hunk + 每侧多个 range + 多字节 UTF-8 字符场景:游标增量转换(H-B 优化)的产出
/// 必须与旧实现(逐 range 从文本头 `chars().count()`)完全一致。
#[test]
fn test_side_by_side_word_diff_char_offsets_match_naive_reference() {
    // 两个修改 hunk,每侧多个 range;首处修改 "foo"→"bar" 之前有 `café`(4 字符 5 字节),
    // 用于区分字符偏移与字节偏移。
    let old_content = "\
let x = café + foo;
let same_1 = 1;
let a = alpha + beta;
let b = gamma + delta;
let same_2 = 2;
let z = tail;
";
    let new_content = "\
let x = café + bar;
let same_1 = 1;
let a = alpha + bata;
let b = gamma + delt;
let same_2 = 2;
let z = tail;
";
    let appearance = Appearance::mock();
    let data = build_side_by_side_diff_data(
        remove_overlay_color(&appearance),
        add_overlay_color(&appearance),
        remove_inline_overlay_color(&appearance),
        add_inline_overlay_color(&appearance),
        old_content,
        new_content,
    );

    // 参考实现:与旧代码完全相同的 O(ranges × len) 转换(只比对 CharOffset 区间,
    // background 颜色与本次优化无关)。
    let diff_hunks = warp_editor::content::diff::diff_lines(old_content, new_content);
    let mut left_ref: Vec<(CharOffset, CharOffset)> = Vec::new();
    let mut right_ref: Vec<(CharOffset, CharOffset)> = Vec::new();
    for hunk in &diff_hunks {
        for r in &hunk.old_word_diffs {
            left_ref.push((
                CharOffset::from(old_content[..r.start].chars().count()),
                CharOffset::from(old_content[..r.end].chars().count()),
            ));
        }
        for r in &hunk.new_word_diffs {
            right_ref.push((
                CharOffset::from(new_content[..r.start].chars().count()),
                CharOffset::from(new_content[..r.end].chars().count()),
            ));
        }
    }

    let left_actual: Vec<(CharOffset, CharOffset)> =
        data.left_text_decorations.iter().map(|d| (d.start, d.end)).collect();
    let right_actual: Vec<(CharOffset, CharOffset)> =
        data.right_text_decorations.iter().map(|d| (d.start, d.end)).collect();

    // 场景必须真实产生 word diff(多 hunk、多 range),否则测试无意义。
    assert!(!left_ref.is_empty() && !right_ref.is_empty(), "场景应产生 word diff range");
    assert!(
        diff_hunks
            .iter()
            .filter(|h| !h.old_word_diffs.is_empty() || !h.new_word_diffs.is_empty())
            .count()
            >= 2,
        "场景应覆盖至少两个含 word diff 的 hunk"
    );
    assert_eq!(left_actual, left_ref, "左列 CharOffset 必须与旧实现逐值一致");
    assert_eq!(right_actual, right_ref, "右列 CharOffset 必须与旧实现逐值一致");

    // 多字节校验:首处修改的旧侧 range 之前有 `café`,其字符偏移必须小于字节偏移,
    // 证明输出按字符计数而非字节计数。
    let first_old_range = diff_hunks
        .iter()
        .flat_map(|h| &h.old_word_diffs)
        .next()
        .expect("应有 old word diff range");
    let first_char = old_content[..first_old_range.start].chars().count();
    assert!(first_char < first_old_range.start, "café 使该处字符偏移小于字节偏移");
    assert_eq!(
        left_actual[0].0,
        CharOffset::from(first_char),
        "首个 decoration 起点必须是字符偏移"
    );
}

/// 回归:spacer 不能按"总行数"截断。两侧 spacer 总高必须满足
/// left − right = 新旧行数差(高度不变量),否则同一 scroll_top 下两列错位。
/// 旧实现(limit_spacers)在 spacer 总行数 > 2000 时把超出部分折叠成 1 行
/// 占位块,两侧被折叠高度不同——大 diff 文件(如 3000+ 行、超 2000 spacer 行)
/// 在首个被截断 hunk 之下整体错位。本测试构造超过旧预算的场景锁死不变量。
#[test]
fn test_side_by_side_spacers_keep_height_invariant_beyond_old_line_budget() {
    // 混合 diff:每 3 行一组插入 2 行(左侧 spacer),每 9 行删 1 行(右侧 spacer),
    // spacer 总行数约 2000 + 333 > 旧预算 2000。
    let old_lines: Vec<String> = (0..3000).map(|i| format!("line {i}")).collect();
    let mut new_lines: Vec<String> = Vec::with_capacity(old_lines.len() + 3000);
    for (i, line) in old_lines.iter().enumerate() {
        match i % 3 {
            0 => {
                new_lines.push(format!("head-a-{i}"));
                new_lines.push(format!("head-b-{i}"));
                new_lines.push(line.clone());
            }
            1 if i % 9 == 1 => {}
            _ => new_lines.push(line.clone()),
        }
    }
    let old_content = old_lines.join("\n") + "\n";
    let new_content = new_lines.join("\n") + "\n";

    let appearance = Appearance::mock();
    let data = build_side_by_side_diff_data(
        remove_overlay_color(&appearance),
        add_overlay_color(&appearance),
        remove_inline_overlay_color(&appearance),
        add_inline_overlay_color(&appearance),
        &old_content,
        &new_content,
    );

    let spacer_lines = |blocks: &[warp_editor::content::edit::TemporaryBlock]| -> usize {
        blocks
            .iter()
            .map(|b| b.content.lines().count().max(1))
            .sum()
    };
    let left = spacer_lines(&data.left_spacers);
    let right = spacer_lines(&data.right_spacers);

    // 场景必须超过旧 2000 行预算,否则测试没有覆盖被截断的路径。
    assert!(
        left + right > 2000,
        "场景 spacer 总行数应超过旧预算 2000,实际 left={left} right={right}"
    );
    // 高度不变量:两侧 spacer 行数差 == 新旧行数差。
    assert_eq!(
        left as isize - right as isize,
        new_lines.len() as isize - old_lines.len() as isize,
        "两侧 spacer 行数差必须等于新旧行数差(左={left} 右={right})"
    );
}

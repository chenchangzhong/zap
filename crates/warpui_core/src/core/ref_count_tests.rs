use super::*;

/// Test that a weak handle correctly fails to upgrade after the last strong
/// handle is dropped.
///
/// When the last `ModelHandle` to a model is dropped, the model's ref count
/// hits zero and it is marked for removal, but it is not immediately removed
/// from the store (`remove_dropped_items` runs later). Before the fix,
/// `WeakModelHandle::upgrade` only checked `app.models.contains_key`, so it
/// would succeed during that window and create a stale handle that panics
/// with "circular model reference" when read.
#[test]
fn test_weak_handle_fails_after_last_strong_handle_dropped() {
    struct Emitter;

    impl Entity for Emitter {
        type Event = ();
    }

    App::test((), |mut app| async move {
        // Drop the only strong handle inside a block so it goes out of scope.
        let weak = {
            let emitter = app.add_model(|_| Emitter);
            let weak = emitter.downgrade();
            drop(emitter);
            weak
        };

        // The model may still be in the store (removal is deferred), but the
        // ref count has hit zero: upgrading must return None rather than
        // resurrecting a zombie handle.
        app.update(|ctx| {
            assert!(
                weak.upgrade(&*ctx).is_none(),
                "weak upgrade should return None after the last strong handle is dropped"
            );
        });
    });
}

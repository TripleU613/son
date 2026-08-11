use leptos::prelude::*;

use crate::components::icon::{Ico, LuHeart};

use crate::api::like_son;

/// The "cry over this son" button -- the site's like/favourite action.
///
/// Updates optimistically: the count moves the instant it is clicked rather
/// than after a round trip, then reconciles with whatever the server says. On
/// failure it rolls back, so a dropped request cannot leave a like that was
/// never recorded showing as counted.
#[component]
pub fn LikeButton(
    id: String,
    initial_count: i64,
    initial_liked: bool,
    /// Compact rendering for gallery cards.
    #[prop(default = false)]
    small: bool,
) -> impl IntoView {
    let (count, set_count) = signal(initial_count);
    let (liked, set_liked) = signal(initial_liked);
    let (busy, set_busy) = signal(false);

    let click = move |ev: leptos::ev::MouseEvent| {
        // Cards are wrapped in a link; without this the click navigates instead.
        ev.prevent_default();
        ev.stop_propagation();

        if busy.get_untracked() {
            return;
        }
        set_busy.set(true);

        let was_liked = liked.get_untracked();
        let was_count = count.get_untracked();

        set_liked.set(!was_liked);
        set_count.set(if was_liked {
            (was_count - 1).max(0)
        } else {
            was_count + 1
        });

        let id = id.clone();
        leptos::task::spawn_local(async move {
            match like_son(id).await {
                Ok((server_count, now_liked)) => {
                    set_count.set(server_count);
                    set_liked.set(now_liked);
                }
                Err(e) => {
                    leptos::logging::error!("like failed: {e}");
                    set_count.set(was_count);
                    set_liked.set(was_liked);
                }
            }
            set_busy.set(false);
        });
    };

    view! {
        <button
            class="like"
            class:like-small=small
            class:liked=move || liked.get()
            on:click=click
            aria-label=move || {
                if liked.get() { "Un-cry over this son" } else { "Cry over this son" }
            }
        >
            // A Lucide heart rather than the 😭 glyph: the redesign uses one
            // line-icon language throughout and explicitly rules out emoji
            // standing in for icons. The *wording* keeps the joke -- the
            // accessible label and the metric are still "cry over", not "like".
            <span class="like-emoji">
                <Ico icon=LuHeart size=15/>
            </span>
            <span class="like-count">{move || count.get()}</span>
        </button>
    }
}

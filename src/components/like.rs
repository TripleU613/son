use leptos::prelude::*;

use crate::components::icon::{Ico, LuDroplet};

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
        // Kept as a guard, not as a fix for a live bug: this button no longer
        // sits inside the card's anchor (see `card.rs`), so there is currently
        // no navigation to suppress and nothing above it listening for clicks.
        // It stays because the cost is nothing and the failure it prevents --
        // a like that silently navigates away instead of registering -- is the
        // kind that only shows up once this button is dropped somewhere new.
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

    // One reactive class string rather than five `class:` toggles. Toggling
    // individual utilities means the base class and the toggled one are both
    // present and equal-specificity, so which wins depends on their order in
    // the generated stylesheet -- the exact cascade coin-flip this migration
    // exists to remove. Swapping whole sets keeps one padding and one colour
    // in play at a time. Every literal below is visible to the Tailwind
    // scanner, which reads these .rs sources.
    let size = if small {
        "px-2 py-1 text-xs"
    } else {
        "px-3 py-1.5 text-[0.85rem]"
    };
    let class = move || {
        let state = if liked.get() {
            "border-accent-border bg-accent-soft text-accent"
        } else {
            "border-line bg-transparent text-ink-2 hover:border-accent-border hover:text-ink"
        };
        format!("inline-flex flex-none items-center gap-1.5 rounded-full border transition-colors {size} {state}")
    };

    view! {
        <button
            class=class
            on:click=click
            aria-label=move || {
                if liked.get() { "Un-cry over this son" } else { "Cry over this son" }
            }
        >
            // A teardrop, not a heart: the metric is "cries", and a heart said
            // "like" while every label around it said cry. Lucide has no crying
            // face, and an 😭 glyph here would be an emoji standing in for an
            // icon, which the rest of this UI does not do.
            <span class="inline-flex">
                <Ico icon=LuDroplet size=15/>
            </span>
            <span class="tabular-nums">{move || count.get()}</span>
        </button>
    }
}

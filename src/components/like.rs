use leptos::prelude::*;

use crate::components::icon::{Ico, LuDroplet};

use crate::api::{like_son, LikeOutcome};
use crate::components::sign_in::SignInLink;

/// The "cry over this son" button -- the site's like/favourite action.
///
/// Updates optimistically: the count moves the instant it is clicked rather
/// than after a round trip, then reconciles with whatever the server says. On
/// failure it rolls back, so a dropped request cannot leave a like that was
/// never recorded showing as counted.
///
/// Likes require an account. That is checked on the server (`api::like_son`),
/// never here, so this component only has to deal with the answer: when it comes
/// back `SignInRequired` the button is replaced by a link to the sign-in flow
/// that returns to this page. Deliberately no up-front "are you signed in?"
/// query -- this button appears once per card, so a per-instance resource would
/// be 24 extra requests to render one gallery page, to save a round trip on a
/// click most visitors never make.
#[component]
pub fn LikeButton(
    id: String,
    initial_count: i64,
    initial_liked: bool,
    /// Compact rendering for gallery cards.
    #[prop(default = false)]
    small: bool,
    /// The detail page's primary action: bigger target, filled when liked.
    /// Ignored when `small` is also set -- a card tile has no room for it, and
    /// silently sizing one up would push the tear off the corner of the image.
    #[prop(default = false)]
    prominent: bool,
) -> impl IntoView {
    let (count, set_count) = signal(initial_count);
    let (liked, set_liked) = signal(initial_liked);
    let (busy, set_busy) = signal(false);
    // Latched once the server says this needs an account, and never cleared:
    // the way back from here is the sign-in redirect, which reloads the page
    // and rebuilds this component from scratch.
    let (needs_account, set_needs_account) = signal(false);

    // StoredValue, not the `String` itself: the button and the sign-in link are
    // two branches of a reactive view, so the closure that builds them re-runs
    // and has to be `Fn`. A captured `String` makes the click handler `FnOnce`
    // and the whole thing stops compiling with an error that points at the view
    // macro rather than at this line.
    let son_id = StoredValue::new(id);

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

        leptos::task::spawn_local(async move {
            match like_son(son_id.get_value()).await {
                Ok(LikeOutcome::Toggled { count, liked }) => {
                    set_count.set(count);
                    set_liked.set(liked);
                }
                // Not a failure: nothing was written, so the optimistic bump has
                // to come back off, and then the control becomes the sign-in
                // link that would have made this click work. Rolling back
                // without that swap is what the old anonymous path effectively
                // did -- a tear that flickered and reverted, with no way to
                // learn why.
                Ok(LikeOutcome::SignInRequired) => {
                    set_count.set(was_count);
                    set_liked.set(was_liked);
                    set_needs_account.set(true);
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
    //
    // `prominent` is a third whole size, not `px-4 py-2` layered over the
    // default's `px-3 py-1.5`: layered, both land at equal specificity and the
    // padding is decided by stylesheet order.
    let size = if small {
        "px-2 py-1 text-xs"
    } else if prominent {
        "px-4 py-2 text-[0.9rem] font-semibold"
    } else {
        "px-3 py-1.5 text-[0.85rem]"
    };
    // Six whole strings, not three crossed with a modifier, for the same
    // reason. The small variant sits on top of a photograph instead of on
    // `bg-surface`: a transparent chip behind a `border-line` hairline simply
    // disappears over a bright son, and `bg-accent-soft` is 10% opacity --
    // enough to tint a dark panel, invisible over an image. On-image states
    // carry their own dark backing so the count stays readable whatever is
    // behind it. The prominent liked state is the only filled one anywhere in
    // this component, which is what makes it read as the page's main action.
    let class = move || {
        let state = match (small, prominent, liked.get()) {
            (true, _, true) => "border-accent-border bg-black/60 text-accent backdrop-blur-sm",
            (true, _, false) => {
                "border-accent-line bg-black/60 text-ink backdrop-blur-sm hover:border-accent-border hover:text-accent"
            }
            (false, true, true) => {
                "border-transparent bg-accent text-accent-ink hover:bg-accent-hover"
            }
            (false, true, false) => "border-accent-border bg-accent-soft text-accent",
            (false, false, true) => "border-accent-border bg-accent-soft text-accent",
            (false, false, false) => {
                "border-line bg-transparent text-ink-2 hover:border-accent-border hover:text-ink"
            }
        };
        format!("inline-flex flex-none items-center gap-1.5 rounded-full border transition-[color,border-color,background-color,transform] duration-150 ease-out active:scale-95 {size} {state}")
    };

    // aria-label and title carry the same words: the label is the accessible
    // name, the title is the hover tooltip, and this was the one control in the
    // detail page's action bar with the first and not the second.
    let label = move || {
        if liked.get() {
            "Un-cry over this son"
        } else {
            "Cry over this son"
        }
    };

    view! {
        <Show
            when=move || needs_account.get()
            fallback=move || {
                view! {
                    <button class=class on:click=click aria-label=label title=label>
                        // A teardrop, not a heart: the metric is "cries", and a
                        // heart said "like" while every label around it said
                        // cry. Lucide has no crying face, and an 😭 glyph here
                        // would be an emoji standing in for an icon, which the
                        // rest of this UI does not do.
                        <span class="inline-flex">
                            <Ico icon=LuDroplet size=15/>
                        </span>
                        <span class="tabular-nums">{move || count.get()}</span>
                    </button>
                }
            }
        >
            // An anchor, not a button that navigates: this is a link to another
            // page, sign-in is a full redirect, and the middle-click and
            // open-in-new-tab that visitors expect of one come free. Same class
            // string, so the control does not jump when it changes job.
            //
            // Via SignInLink because this anchor was missing rel="external",
            // which meant the router intercepted the click and rendered the 404
            // page: the one control offering a way out of "you need an account"
            // led nowhere.
            <SignInLink label="Sign in to cry over this son" attr:class=class>
                <span class="inline-flex">
                    <Ico icon=LuDroplet size=15/>
                </span>
                <span class="tabular-nums">{move || count.get()}</span>
            </SignInLink>
        </Show>
    }
}

use leptos::prelude::*;
use leptos_router::components::A;

use crate::components::like::LikeButton;
use crate::models::Son;

#[component]
pub fn SonCard(son: Son) -> impl IntoView {
    let href = format!("/son/{}", son.id);

    // The son's true aspect, published as a custom property rather than set
    // directly as `aspect-ratio`. Both matter: reserving the box keeps the
    // grid from reflowing as thumbnails stream in, but applying each image's
    // own ratio in a two-up grid made every row stagger to its tallest card
    // and left dead gaps. CSS now decides -- uniform tiles in the grid views,
    // the real ratio in list view and on the detail page.
    let ratio = format!("{} / {}", son.width.max(1), son.height.max(1));

    // A plain one-time conditional, not <Show>: `when` there expects a
    // reactive closure for a condition that can change after the initial
    // render, and tags don't change once a card has mounted -- using it
    // anyway produced a real Fn-vs-FnOnce compile error, since the nested
    // <For> closure had to move its own `tags` clone out of the Show
    // children's environment.
    let tag_chips = (!son.tags.is_empty()).then(|| {
        let tags = son.tags.clone();
        view! {
            <div class="flex flex-wrap gap-1.5">
                <For each=move || tags.clone() key=|t| t.slug.clone() let:tag>
                    <A href=format!("/tag/{}", tag.slug) attr:class="chip px-2.5 py-0.5 text-[0.7rem]">
                        {tag.name.clone()}
                    </A>
                </For>
            </div>
        }
    });

    view! {
        // A plain div, not one giant <A>: tag chips below need their own
        // links to /tag/:slug, and a nested <a> inside an <a> is invalid HTML
        // (and browsers resolve the click ambiguously). The image/title
        // block is its own anchor instead.
        <div class="card">
            <A href=href attr:class="flex flex-none flex-col text-inherit no-underline">
                <div class="card-frame relative aspect-[4/5] w-full flex-none overflow-hidden bg-surface-raised" style=format!("--son-ratio: {ratio}")>
                    <img
                        src=son.thumb_url.clone()
                        alt=son.title.clone()
                        loading="lazy"
                        decoding="async"
                        class="absolute inset-0 block h-full w-full object-cover"
                    />
                </div>
                <div class="flex items-center justify-between gap-2 px-2.5 pb-1.5 pt-2 text-[0.8rem] sm:gap-2.5 sm:px-3 sm:py-2.5">
                    <span class="overflow-hidden text-ellipsis whitespace-nowrap">{son.title.clone()}</span>
                    <LikeButton
                        id=son.id.clone()
                        initial_count=son.likes
                        initial_liked=son.liked_by_me
                        small=true
                    />
                </div>
            </A>
            <div class="mt-auto flex flex-col gap-1.5 px-2.5 pb-2.5 sm:px-3 sm:pb-3">
                <span class="text-[0.78rem] text-ink-3">
                    {match &son.uploader {
                        Some(u) => format!("by {}", u.display_name),
                        None => "anonymous".to_string(),
                    }}
                </span>
                {tag_chips}
            </div>
        </div>
    }
}

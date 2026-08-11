use leptos::prelude::*;
use leptos_router::components::A;

use crate::components::like::LikeButton;
use crate::models::Son;

#[component]
pub fn SonCard(son: Son) -> impl IntoView {
    let href = format!("/son/{}", son.slug);

    // The son's true aspect, published as a custom property rather than set
    // directly as `aspect-ratio`. Both matter: reserving the box keeps the
    // grid from reflowing as thumbnails stream in, but applying each image's
    // own ratio in a two-up grid made every row stagger to its tallest card
    // and left dead gaps. CSS now decides -- uniform tiles in the two grid
    // views, the real ratio in masonry and on the detail page.
    let ratio = format!("{} / {}", son.width.max(1), son.height.max(1));

    view! {
        // A plain div, not one giant <A>: the like button below is interactive
        // and a <button> inside an <a> resolves ambiguously on click. The
        // image/title block is its own anchor instead.
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
            </div>
        </div>
    }
}

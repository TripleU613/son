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
        //
        // `group` so the thumbnail can react to a hover anywhere on the card,
        // including the byline that sits outside the anchor -- a zoom that only
        // fired over the image itself would flicker as the pointer crossed into
        // the caption.
        //
        // The focus ring is moved from the inner anchor onto the whole card.
        // Left on the anchor it drew a rectangle around the image-plus-title
        // *inside* the card's border, cutting the tile in half; `has-` is the
        // same mechanism the upload drop zone already uses for the same reason.
        // Outline, not border-colour: `.card` styles its own border on hover,
        // and two rules setting one property is the cascade coin-flip this
        // project banned.
        <div class="card group has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-accent">
            <A
                href=href
                attr:class="flex flex-none flex-col text-inherit no-underline focus-visible:outline-none"
            >
                <div class="card-frame relative aspect-[4/5] w-full flex-none overflow-hidden bg-surface-raised" style=format!("--son-ratio: {ratio}")>
                    <img
                        src=son.thumb_url.clone()
                        alt=son.title.clone()
                        loading="lazy"
                        decoding="async"
                        // Clipped by the frame's own overflow-hidden, so the
                        // scale reads as the crop opening up rather than the
                        // tile growing and shoving the grid around.
                        class="absolute inset-0 block h-full w-full object-cover transition-transform duration-300 ease-out group-hover:scale-[1.03]"
                    />
                </div>
                // The title gets the whole width. It used to share this row with
                // the like button, which is 47px of a 138px card at 320px wide
                // -- the title was left 61px and every son past two syllables
                // rendered as "Makaau…". Measured, not guessed.
                <div class="px-2.5 pt-2.5 sm:px-3 sm:pt-3">
                    <span class="block truncate text-[0.8125rem] font-medium leading-snug text-ink">
                        {son.title.clone()}
                    </span>
                </div>
            </A>
            // Byline and like button share the meta row, outside the anchor.
            //
            // Outside is the point: a <button> inside an <a> is invalid HTML and
            // resolves ambiguously on click, which is what the comment at the
            // top of this component always said -- but the like button was in
            // the anchor anyway, and `like.rs` carried a prevent_default purely
            // to undo the navigation that nesting caused. Now the anchor holds
            // only the image and the title, and nothing has to be undone.
            //
            // The byline is also two steps down from the title rather than one:
            // at 0.8rem against 0.78rem the pair read as a single grey smear
            // with nothing for the eye to land on first.
            <div class="mt-auto flex items-center justify-between gap-2 px-2.5 pb-2.5 pt-1 sm:gap-2.5 sm:px-3 sm:pb-3">
                <span class="min-w-0 truncate text-[0.6875rem] leading-normal text-ink-3">
                    {match &son.uploader {
                        Some(u) => format!("by {}", u.display_name),
                        None => "anonymous".to_string(),
                    }}
                </span>
                <LikeButton
                    id=son.id.clone()
                    initial_count=son.likes
                    initial_liked=son.liked_by_me
                    small=true
                />
            </div>
        </div>
    }
}

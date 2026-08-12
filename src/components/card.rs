use leptos::prelude::*;
use leptos_router::components::A;

use crate::components::like::LikeButton;
use crate::models::Son;

#[component]
pub fn SonCard(
    son: Son,
    /// Set on the handful of tiles that are above the fold on first paint.
    ///
    /// `loading="lazy"` on every image sounds strictly better and is not: the
    /// browser deprioritises lazy images, so the one that decides this page's
    /// LCP -- a tile in the first row -- queues behind work it should have led.
    /// The gallery marks its opening row and nothing else; everything below is
    /// lazy, which at 10k sons is the only reason the grid is affordable at all.
    #[prop(default = false)]
    priority: bool,
) -> impl IntoView {
    let href = format!("/son/{}", son.slug);
    // One binding per use. The title is needed three times -- as the link's
    // accessible name, as the image's alt text, and as the visible caption --
    // and the view macro captures each of those by move, so a `.clone()` written
    // inside the macro is already too late for the use after it.
    let title = son.title.clone();
    let link_label = title.clone();
    let alt_text = title.clone();
    let byline = match &son.uploader {
        Some(u) => format!("by {}", u.display_name),
        None => "anonymous".to_string(),
    };

    view! {
        // Square, and image-only.
        //
        // Square because that is what a son now *is*: `storage::square` crops
        // and scales every upload to a 1024x1024 canvas, so the 4:5 frame this
        // replaces was re-cropping an already-square picture and cutting the top
        // and bottom off it for nothing. Older sons that predate the pipeline are
        // still their original shape and get centre-cropped by `object-cover`,
        // which is the same thing the grid was already doing to them.
        //
        // Image-only because the caption block underneath -- title row, byline
        // row, each with its own padding -- was a strip of chrome stapled to the
        // bottom of every tile, and twelve of them in a grid read as a table of
        // labels rather than a wall of sons. The text now sits over the image,
        // where it costs no layout at all.
        // `content-visibility:auto` is what makes a very long grid affordable.
        // It lets the browser skip layout, style and paint for every tile that
        // is off screen, and pick the work back up as one scrolls in -- so the
        // cost of a 10,000-son grid tracks what is on screen rather than what
        // has been loaded. It needs no intrinsic-size hint here because
        // `aspect-square` already fixes the box's height from its grid-assigned
        // width, so a skipped tile still reserves exactly the right space and
        // the scrollbar does not jump.
        <div class="card group relative aspect-square [content-visibility:auto] has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-accent">
            // The anchor covers the whole tile. `aria-label` carries the title,
            // because the link's only content is an image and the caption that
            // names it lives outside the anchor.
            <A
                href=href
                attr:class="absolute inset-0 block focus-visible:outline-none"
                attr:aria-label=link_label
            >
                <img
                    src=son.thumb_url.clone()
                    alt=alt_text
                    loading=if priority { "eager" } else { "lazy" }
                    fetchpriority=if priority { "high" } else { "auto" }
                    decoding="async"
                    class="absolute inset-0 block h-full w-full object-cover transition-transform duration-300 ease-out group-hover:scale-[1.03]"
                />
            </A>

            // The caption, revealed on hover.
            //
            // `[@media(hover:none)]:opacity-100` is the part that matters: a
            // touch device never fires hover, so keyed on hover alone the title
            // and the tear button would be permanently invisible on every phone.
            // There, the caption simply stays up. Same reasoning as the upload
            // prompt -- ask the device what it can do, rather than guessing from
            // its width.
            //
            // `group-focus-within` keeps it reachable by keyboard, since tabbing
            // to the link or the tear button never triggers hover either.
            //
            // pointer-events-none so the gradient does not intercept clicks
            // meant for the link underneath; the tear button switches them back
            // on for itself alone.
            // The tear sits in the corner, not in the caption row.
            //
            // Sharing that row cost the title 55px of a 145px caption on a
            // 375px phone, which put "sonion powder" back to "sonion pow…" --
            // the same truncation this card was reshaped to fix. Up here it
            // costs the title nothing, and a like control in the top corner of
            // a tile is where galleries put it anyway.
            //
            // Plain `pointer-events-auto`, with no conditional: on a pointer
            // device the button cannot be clicked without hovering the card
            // first, which is exactly what reveals it, and on touch it is
            // always visible. So there is no state in which it is invisible and
            // still clickable.
            <span class="pointer-events-auto absolute right-2 top-2 opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100 [@media(hover:none)]:opacity-100">
                <LikeButton
                    id=son.id.clone()
                    initial_count=son.likes
                    initial_liked=son.liked_by_me
                    small=true
                />
            </span>

            <div class="pointer-events-none absolute inset-x-0 bottom-0 rounded-b-lg bg-gradient-to-t from-black/90 via-black/60 to-transparent px-2.5 pb-2.5 pt-10 opacity-0 transition-opacity duration-200 group-hover:opacity-100 group-focus-within:opacity-100 [@media(hover:none)]:opacity-100">
                <span class="block truncate text-[0.8125rem] font-semibold leading-snug text-ink">
                    {title}
                </span>
                <span class="block truncate text-[0.6875rem] leading-normal text-ink-2">
                    {byline}
                </span>
            </div>
        </div>
    }
}

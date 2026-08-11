use leptos::prelude::*;
use leptos_router::components::A;

use crate::components::like::LikeButton;
use crate::models::Son;

#[component]
pub fn SonCard(son: Son) -> impl IntoView {
    let href = format!("/son/{}", son.id);

    // Reserve the right box before the image loads so the grid does not reflow
    // as thumbnails stream in.
    let ratio = format!("{} / {}", son.width.max(1), son.height.max(1));

    view! {
        <A href=href attr:class="card">
            <div class="card-frame" style:aspect-ratio=ratio>
                <img
                    src=son.thumb_url.clone()
                    alt=son.title.clone()
                    loading="lazy"
                    decoding="async"
                />
            </div>
            <div class="card-meta">
                <span class="card-title">{son.title.clone()}</span>
                <LikeButton
                    id=son.id.clone()
                    initial_count=son.likes
                    initial_liked=son.liked_by_me
                    small=true
                />
            </div>
        </A>
    }
}

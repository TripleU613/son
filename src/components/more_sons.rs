//! A Pinterest-style masonry of further sons, used to fill the space below a
//! single-son view (the detail page, and the son-of-the-day tab).
//!
//! Uses CSS multi-column rather than grid: real masonry wants uneven column
//! runs, and `columns` gives that natively with `break-inside: avoid` on the
//! cards. It also lets each card keep its true aspect ratio via the
//! `--son-ratio` custom property the card already publishes, instead of the
//! uniform 4:5 crop the main grid uses.

use leptos::prelude::*;

use crate::api::list_sons;
use crate::components::card::SonCard;

#[component]
pub fn MoreSons(
    /// Omit this son from the results -- it is the one already being shown.
    #[prop(optional, into)]
    exclude: Option<String>,
) -> impl IntoView {
    // Newest, one page. No cursor state: this is a "keep browsing" surface, not
    // a second paginated gallery, so it deliberately stops after one page.
    let more = Resource::new(
        || (),
        |_| async move { list_sons(None, Some("newest".to_string())).await },
    );

    view! {
        <Suspense fallback=|| ()>
            {move || {
                let exclude = exclude.clone();
                more.get()
                    .and_then(Result::ok)
                    .map(|page| {
                        let sons: Vec<_> = page
                            .sons
                            .into_iter()
                            .filter(|s| Some(&s.id) != exclude.as_ref())
                            .collect();
                        (!sons.is_empty())
                            .then(|| {
                                view! {
                                    <section class="mt-6 border-t border-line pt-6">
                                        <h2 class="m-0 mb-4 text-base font-semibold text-ink-2">"More sons"</h2>
                                        <div class="columns-2 gap-3 sm:columns-3 lg:columns-4 xl:columns-5 [&_.card]:mb-3 [&_.card]:break-inside-avoid [&_.card-frame]:aspect-[var(--son-ratio,4/5)]">
                                            <For
                                                each=move || sons.clone()
                                                key=|s| s.id.clone()
                                                let:son
                                            >
                                                <SonCard son=son/>
                                            </For>
                                        </div>
                                    </section>
                                }
                            })
                    })
            }}
        </Suspense>
    }
}

//! A grid of further sons, used to fill the space below a single-son view (the
//! detail page, and the son-of-the-day tab).
//!
//! Was a CSS multi-column masonry, so that each card could keep its true aspect
//! ratio in ragged columns instead of the main grid's uniform crop. That reason
//! is gone: `storage::square` crops every upload to a square, so the ragged
//! columns had nothing left to be ragged about and this is now the same grid the
//! gallery uses.

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
                                    <section class="mt-4 border-t border-line pt-5 lg:mt-6 lg:pt-6">
                                        // A section label, not a heading that
                                        // competes with the son's own <h1>: at
                                        // text-base semibold it read as a
                                        // second page title. Smaller, tracked
                                        // out and dimmed, it announces the
                                        // section without claiming the page.
                                        <h2 class="m-0 mb-4 text-[0.6875rem] font-semibold uppercase tracking-[0.08em] text-ink-3">
                                            "More sons"
                                        </h2>
                                        <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
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

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, HashedStylesheet, Meta, MetaTags, Title};
use leptos_router::components::{Route, Router, Routes, A};
use leptos_router::hooks::use_location;
use leptos_router::{path, SsrMode};

use crate::api::current_user;
use crate::components::admin::Admin;
use crate::components::detail::SonDetail;
use crate::components::gallery::Gallery;
use crate::components::icon::{
    Ico, LuCircleAlert, LuCirclePlus, LuImage, LuLogOut, LuTrophy, LuUserRound,
};
use crate::components::leaderboard::Leaderboard;
use crate::components::legal::{Privacy, Terms};
use crate::components::progress::{Loading, TopProgress};
use crate::components::search::SearchPage;
use crate::components::search_box::SearchBox;
use crate::components::sort_chips::{GalleryView, SortCtx};
use crate::components::upload::Upload;

/// Cloudflare Web Analytics beacon.
///
/// The one piece of third-party script on the site. Cloudflare already records
/// requests, bandwidth, cache hit ratio, threats and top paths at the edge with
/// nothing added -- that is server-side and needs no tag. This adds what the edge
/// cannot see: page views tied to a session, referrers, browser/OS/country
/// breakdowns, and Core Web Vitals (LCP/INP/CLS), which is what shows whether
/// the gallery is fast on real phones.
///
/// Cookieless and env-gated: absent token, absent script. Kept as a token rather
/// than committed, since it identifies the account's analytics site.
fn cf_beacon_token() -> Option<String> {
    // `shell` is server-rendered but compiles for wasm too, so this needs a body
    // in both builds. There is no environment to read in the browser, and the
    // tag is already in the HTML by then.
    #[cfg(feature = "ssr")]
    {
        std::env::var("CF_ANALYTICS_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
    }
    #[cfg(not(feature = "ssr"))]
    None
}

/// The HTML document. Server-side only entry point; `HydrationScripts` injects
/// the wasm loader so the client picks up where SSR left off.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                // The mark is a crying face, drawn as vector and rasterised
                // from that one source so all three sizes agree.
                //
                // Order matters and is not stylistic. Chrome and Firefox pick
                // the LAST icon they understand, so declaring the SVG after the
                // PNG (with an explicit type, or they will not consider it) is
                // what gets them the vector -- which is the whole point at a
                // 16px favicon, where a downscaled raster smears. Safari and
                // iOS ignore the SVG and keep the PNG path.
                //
                // The version query is a deliberate cache bust: public/ is
                // served through Cloudflare and both PNG paths already have
                // history at the edge, so replacing the bytes at an unchanged
                // URL would leave the previous mark in tabs for hours.
                <link rel="icon" type="image/png" sizes="32x32" href="/favicon-32.png?v=2"/>
                <link rel="icon" type="image/svg+xml" href="/favicon.svg"/>
                <link rel="apple-touch-icon" href="/apple-touch-icon.png?v=2"/>
                <meta name="theme-color" content="#08090b"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options=options.clone()/>
                // Emitted here rather than as a <Stylesheet> inside App: the
                // hashed filename lives in a file next to the binary, so this
                // needs LeptosOptions, which only exists server-side. See
                // `hash-files` in Cargo.toml for why the hash matters.
                <HashedStylesheet options=options.clone() id="leptos"/>
                <MetaTags/>
                // Last in <head> and deferred, so analytics never delays first
                // paint. `data-cf-beacon` is Cloudflare's own attribute contract.
                {cf_beacon_token()
                    .map(|token| {
                        view! {
                            <script
                                defer
                                src="https://static.cloudflareinsights.com/beacon.min.js"
                                data-cf-beacon=format!("{{\"token\": \"{token}\"}}")
                            />
                        }
                    })}
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    // Sort lives here, not in Gallery: the desktop header renders the same four
    // filters the mobile in-page row does, and both must drive one value.
    let (view, set_view) = signal(GalleryView::default());
    provide_context(SortCtx { view, set_view });

    // The in-flight counter behind the top bar. Creating a signal and providing
    // context during render is fine; it is writing an existing one that kills
    // hydration, which is why nothing here calls `start`.
    let loading = Loading::provide();

    view! {
        <Title text="son collection"/>
        <Meta property="og:site_name" content="son collection"/>

        // Handing the router a `set_is_routing` is not just a notification: it
        // switches client-side navigation to a transition, so the previous view
        // stays mounted until the new route's resources resolve, and this stays
        // true for exactly that window. That is a real load state rather than a
        // guessed timer -- and it is also why the route-level <Suspense>
        // fallbacks no longer flash on in-app navigation. The bar replaces
        // them there; they still render on a full page load.
        <Router set_is_routing=SignalSetter::map(move |routing: bool| {
            if routing { loading.start() } else { loading.finish() }
        })>
            // One shell, rearranged entirely by CSS. The primary nav exists
            // exactly once in the DOM and becomes either the desktop sidebar's
            // link list or the mobile bottom bar -- rendering two copies would
            // duplicate landmarks, and branching on window.innerWidth in Rust
            // would invite the hydration mismatch that already took down the
            // wasm module once this session.
            <div class="flex min-h-[100dvh] flex-col">
                // Fixed, so per spec it is not a flex item and cannot touch
                // this column's layout. First in source order anyway, because
                // it is first on screen.
                <TopProgress/>
                <Header/>
                <BottomNav/>
                <main class="mx-auto w-full max-w-content flex-1 px-4 pt-[calc(56px+0.75rem)] pb-[calc(58px+env(safe-area-inset-bottom)+1.5rem)] lg:px-8 lg:pt-[calc(60px+1.5rem)] lg:pb-8">
                    <Routes fallback=NotFound>
                    // Async mode on the two data-driven routes: the default
                    // out-of-order streaming flushes <head> before resources
                    // resolve, so og:/twitter: tags and card markup ended up in
                    // a JS-swapped <template> where unfurlers never see them.
                    <Route path=path!("/") view=Gallery ssr=SsrMode::Async/>
                    <Route path=path!("/son/:id") view=SonDetail ssr=SsrMode::Async/>
                    // No server data; streaming is fine here.
                    <Route path=path!("/upload") view=Upload/>
                    <Route path=path!("/leaderboard") view=Leaderboard ssr=SsrMode::Async/>
                    <Route path=path!("/search") view=SearchPage ssr=SsrMode::Async/>
                    // Not SsrMode::Async: it carries no SEO-relevant content
                    // and is gated server-side in every fn it calls anyway, so
                    // there is nothing here for out-of-order streaming to leak.
                    <Route path=path!("/admin") view=Admin/>
                        <Route path=path!("/privacy") view=Privacy/>
                        <Route path=path!("/tos") view=Terms/>
                    </Routes>
                </main>
                <SiteFooter/>
            </div>
        </Router>
    }
}

/// Legal fine print, on the pages where it is not in the way.
///
/// It used to sit under every page. The gallery and a son page are where people
/// actually spend time, and two words of boilerplate terminating an image grid
/// is the kind of permanent furniture the redesign is removing -- so those two
/// routes drop it.
///
/// It is not deleted, and cannot be: these are the site's only links to
/// /privacy and /tos, and Google's OAuth verification requires a reachable
/// privacy policy for the sign-in this app ships. It still server-renders as a
/// real crawlable anchor on /upload, /leaderboard, /search and the legal pages
/// themselves, and both routes are in the sitemap. `AccountAction`'s menu
/// carries the same two links for signed-in visitors on any page; the footer
/// stays *in addition* because the signed-out control is a bare sign-in link
/// with no menu behind it -- exactly the audience that needs the policy before
/// they sign in.
///
/// A component rather than an inline branch in `App`, because `use_location`
/// panics outside `<Router>` and `App`'s body runs before the `<Router>` in its
/// own view exists. That failure is at runtime, and on the server it takes the
/// whole request down. `AccountAction` has the same shape for the same reason.
#[component]
fn SiteFooter() -> impl IntoView {
    let path = use_location().pathname;
    // `<Show>`, not `.then()`: the pathname is genuinely reactive and this has
    // to flip on client-side navigation. The server knows the request path, so
    // both renders agree and there is nothing here to mismatch.
    let show = move || {
        let p = path.get();
        !(p == "/" || p.starts_with("/son/"))
    };

    view! {
        <Show when=show>
            // Padded clear of the mobile bottom nav rather than competing with
            // it, and on the same max-w-content column and lg gutter as <main>
            // so it lines up with the content above rather than with the
            // viewport.
            <footer class="px-4 pb-[calc(58px+env(safe-area-inset-bottom)+0.75rem)] pt-2 lg:px-8 lg:pb-6">
                <div class="mx-auto flex max-w-content justify-center gap-4 text-[0.6875rem] text-ink-3/70">
                    <A href="/privacy" attr:class="transition-colors hover:text-ink-2">
                        "Privacy"
                    </A>
                    <A href="/tos" attr:class="transition-colors hover:text-ink-2">
                        "Terms"
                    </A>
                </div>
            </footer>
        </Show>
    }
}

/// The header. On mobile: brand, a search field and the account action. On
/// desktop it additionally carries the section links and -- on the gallery only
/// -- the sort chips, replacing the sidebar entirely.
#[component]
fn Header() -> impl IntoView {
    view! {
        <header class="fixed inset-x-0 top-0 z-30 h-topbar border-b border-line bg-bg lg:h-topbar-lg">
            // Inner wrapper so the bar's contents line up with the content
            // column below it: the bar stays full-bleed (its border needs to
            // reach the viewport edges) while the brand and the icons sit on the
            // same left/right edges as the grid.
            <div class="mx-auto flex h-full max-w-content items-center gap-3 px-4 lg:gap-4 lg:px-8">
            // The real mark, at 2x for retina. width/height are set so the
            // header does not reflow while it loads, and it is eager rather than
            // lazy because it is the first thing above the fold.
            // `lg:flex-1` here and on the icon group opposite makes the two
            // side regions equal, which is what actually centres the search
            // field: left to size themselves, the 93px brand and the 3-icon
            // group are not the same width, and the field sat 29px left of the
            // page centre. Measured, not eyeballed. Below lg the brand stays
            // `flex-none` so the field can take the rest of a narrow bar.
            <A href="/" attr:class="flex flex-none items-center lg:flex-1" attr:aria-label="son collection, home">
                <img
                    src="/logo.png"
                    alt="son collection"
                    width="183"
                    height="96"
                    decoding="async"
                    class="h-7 w-auto lg:h-8"
                />
            </A>

            // The search field, between the brand and the account controls at
            // every width. It used to collapse behind a magnifier on desktop
            // and expand on click, which meant the bar held two things that
            // both meant "search" -- the icon and, once open, the field it
            // revealed. The field is its own affordance, so the icon is gone
            // and the state that drove it with it.
            //
            // `flex-1` with a desktop cap and `mx-auto`: the field takes the
            // room between the two fixed side groups, and once the cap binds,
            // the auto margins absorb what is left over and centre it rather
            // than letting one 448px pill drift against the brand.
            <SearchBox/>

            <div class="flex flex-none items-center gap-1 lg:flex-1 lg:justify-end lg:gap-2">
                // Desktop-only section links, as icons alongside the account
                // action. No Gallery entry: the brand is the gallery, which is
                // the default view. Icon-only, so each carries an aria-label
                // and a title -- the label is the accessible name, the title is
                // the hover tooltip.
                <nav class="hidden flex-none items-center gap-1 lg:flex" aria-label="main">
                    <A
                        href="/upload"
                        attr:class="icon-btn"
                        attr:aria-label="Contribute"
                        attr:title="Contribute"
                    >
                        <Ico icon=LuCirclePlus size=18/>
                    </A>
                    <A
                        href="/leaderboard"
                        attr:class="icon-btn"
                        attr:aria-label="Leaderboard"
                        attr:title="Leaderboard"
                    >
                        <Ico icon=LuTrophy size=18/>
                    </A>
                </nav>

                <AccountAction/>
                </div>
            </div>
        </header>
    }
}

/// Primary navigation on mobile only: a fixed bottom bar. Desktop uses the
/// header above instead of a sidebar.
#[component]
fn BottomNav() -> impl IntoView {
    view! {
        <nav class="fixed inset-x-0 bottom-0 z-30 flex border-t border-line bg-bg pb-[env(safe-area-inset-bottom)] lg:hidden" aria-label="main">
            // exact, not the default prefix match: every route starts with "/",
            // so without this the Gallery link would be aria-current on every
            // page in the app.
            <A href="/" attr:class="flex min-h-bottomnav flex-1 flex-col items-center justify-center gap-[3px] px-2 py-1 text-ink-3 transition-colors hover:text-ink-2 aria-[current=page]:text-accent" exact=true>
                <Ico icon=LuImage/>
                <span class="text-[0.7rem] aria-[current=page]:font-semibold">"Gallery"</span>
            </A>
            <A href="/upload" attr:class="flex min-h-bottomnav flex-1 flex-col items-center justify-center gap-[3px] px-2 py-1 text-ink-3 transition-colors hover:text-ink-2 aria-[current=page]:text-accent">
                <Ico icon=LuCirclePlus/>
                <span class="text-[0.7rem] aria-[current=page]:font-semibold">"Contribute"</span>
            </A>
            <A href="/leaderboard" attr:class="flex min-h-bottomnav flex-1 flex-col items-center justify-center gap-[3px] px-2 py-1 text-ink-3 transition-colors hover:text-ink-2 aria-[current=page]:text-accent">
                <Ico icon=LuTrophy/>
                <span class="text-[0.7rem] aria-[current=page]:font-semibold">"Leaderboard"</span>
            </A>
        </nav>
    }
}

/// Sign-in link, or the signed-in user's avatar plus sign-out. Always the
/// compact icon form now that the header is the only place it appears -- the
/// wider sidebar variant went away with the sidebar.
#[component]
fn AccountAction() -> impl IntoView {
    let location = use_location();
    let user = Resource::new(|| (), |_| current_user());
    // Whether the account menu is open. Signed-out visitors never see it, so
    // this is only ever toggled by the avatar button below.
    let (menu, set_menu) = signal(false);

    view! {
        <Suspense fallback=|| ()>
            {move || {
                user.get()
                    .map(|res| match res {
                        Ok(Some(u)) => {
                            let initial = u
                                .display_name
                                .chars()
                                .next()
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_else(|| "?".into());
                            view! {
                                // `relative` so the panel can hang off the
                                // avatar rather than off the header, which would
                                // put it at the far edge of the viewport.
                                <div class="relative flex flex-none items-center">
                                    <button
                                        type="button"
                                        class="icon-btn overflow-hidden"
                                        aria-label="Account"
                                        title="Account"
                                        aria-haspopup="menu"
                                        aria-expanded=move || menu.get().to_string()
                                        on:click=move |_| set_menu.update(|m| *m = !*m)
                                    >
                                        {u
                                            .avatar_url
                                            .clone()
                                            .map(|src| {
                                                view! {
                                                    <img class="h-6 w-6 rounded-full object-cover" src=src alt=""/>
                                                }
                                                    .into_any()
                                            })
                                            .unwrap_or_else(|| {
                                                // No Google picture: their
                                                // initial, so the control still
                                                // reads as "you" rather than as a
                                                // generic person glyph.
                                                view! {
                                                    <span class="flex h-6 w-6 items-center justify-center rounded-full bg-surface-raised text-[0.7rem] font-semibold text-ink-2">
                                                        {initial.clone()}
                                                    </span>
                                                }
                                                    .into_any()
                                            })}
                                    </button>

                                    <Show when=move || menu.get()>
                                        // A click anywhere else closes it. A
                                        // full-viewport transparent layer behind
                                        // the panel does that without a document
                                        // listener to add, remove, and leak.
                                        <div
                                            class="fixed inset-0 z-40"
                                            on:click=move |_| set_menu.set(false)
                                        />
                                        <div
                                            class="absolute right-0 top-full z-50 mt-1 min-w-[11rem] overflow-hidden rounded-lg border border-line bg-surface-raised py-1 shadow-lg"
                                            role="menu"
                                        >
                                            <p class="truncate px-3 py-1.5 text-[0.8rem] text-ink-3">
                                                {u.display_name.clone()}
                                            </p>
                                            <div class="my-1 border-t border-line"/>
                                            {u.is_admin
                                                .then(|| {
                                                    view! {
                                                        <A
                                                            href="/admin"
                                                            attr:class="block px-3 py-2 text-[0.85rem] text-accent hover:bg-surface-hover"
                                                            attr:role="menuitem"
                                                        >
                                                            "Admin"
                                                        </A>
                                                    }
                                                })}
                                            // The fine print, reachable from
                                            // any page including the gallery,
                                            // which no longer carries a footer.
                                            <div class="my-1 border-t border-line"/>
                                            <A
                                                href="/privacy"
                                                attr:class="block px-3 py-2 text-[0.85rem] text-ink-2 transition-colors hover:bg-surface-hover hover:text-ink"
                                                attr:role="menuitem"
                                            >
                                                "Privacy"
                                            </A>
                                            <A
                                                href="/tos"
                                                attr:class="block px-3 py-2 text-[0.85rem] text-ink-2 transition-colors hover:bg-surface-hover hover:text-ink"
                                                attr:role="menuitem"
                                            >
                                                "Terms"
                                            </A>
                                            <div class="my-1 border-t border-line"/>
                                            <form method="post" action="/auth/logout">
                                                <button
                                                    type="submit"
                                                    role="menuitem"
                                                    class="flex w-full items-center gap-2 px-3 py-2 text-left text-[0.85rem] text-ink-2 transition-colors hover:bg-surface-hover hover:text-ink"
                                                >
                                                    <Ico icon=LuLogOut size=15/>
                                                    "Sign out"
                                                </button>
                                            </form>
                                        </div>
                                    </Show>
                                </div>
                            }
                                .into_any()
                        }
                        _ => {
                            // Ok(None) (never signed in, or Google sign-in is
                            // not configured yet) and Err (couldn't reach D1)
                            // get the same link -- nothing useful to tell a
                            // visitor apart between those here.
                            let return_to = location.pathname.get();
                            let href = format!(
                                "/auth/google/login?return_to={}",
                                urlencode(&return_to),
                            );
                            view! {
                                <a
                                    class="icon-btn"
                                    href=href
                                    aria-label="Sign in"
                                    title="Sign in"
                                >
                                    <Ico icon=LuUserRound size=18/>
                                </a>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}

/// Percent-encode a path for use as a query value. `return_to` is always a
/// same-origin path (checked again server-side in oauth_route::login), so
/// only the characters that would break a query string need escaping.
fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// The site logo as this page's link-preview image, for pages that have no image
/// of their own.
///
/// Deliberately opt-in per page rather than a default in `App`, because
/// leptos_meta resolves a duplicated `og:image` first-set-wins: a default here
/// silently beat `SonDetail`'s own tag and every son previewed as the site logo.
/// Raw markup in `shell` is worse still -- it cannot be overridden at all, and
/// produced two `og:image` tags where an unfurler picks whichever it likes.
///
/// Absolute, because unfurlers reject a relative `og:image` outright.
#[component]
pub fn SitePreview() -> impl IntoView {
    view! {
        <Meta property="og:image" content=crate::seo::absolute("/logo-large.png")/>
        <Meta property="og:type" content="website"/>
    }
}

/// Set the response status during SSR.
///
/// Without this a missing page renders the 404 body under HTTP 200 -- a soft
/// 404, which Google treats as a thin duplicate of every other soft 404 on the
/// site rather than as "gone". No-op on the client, where the response has
/// already been sent.
pub fn set_status(code: u16) {
    #[cfg(feature = "ssr")]
    if let Some(resp) = use_context::<leptos_axum::ResponseOptions>() {
        resp.set_status(
            axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::OK),
        );
    }
    #[cfg(not(feature = "ssr"))]
    let _ = code;
}

#[component]
fn NotFound() -> impl IntoView {
    set_status(404);

    view! {
        <Title text="no son here — son collection"/>
        // Same shape as the site's other zero states -- icon tile, one line --
        // which is what makes this read as a finished page rather than a stub,
        // now that it carries no link of its own. It is not a dead end: the
        // header logo is a home link at every width and the mobile bottom bar
        // still has Gallery.
        //
        // The tile is sized by its padding and the 28px glyph, not by an
        // explicit box, so there is one number to change rather than three that
        // can disagree.
        //
        // Written out here rather than reusing EmptyState because that
        // component's line is a <p>: a 404 needs a real <h1>, and dropping the
        // page's only heading is an SEO regression on top of an accessibility
        // one. The crying face this page used to end on is not gone either --
        // it is the favicon in the tab of this very page.
        <section class="flex min-h-[46vh] flex-col items-center justify-center gap-3 text-center">
            <span class="inline-flex items-center justify-center rounded-lg border border-line bg-surface p-3 text-ink-3">
                <Ico icon=LuCircleAlert size=28/>
            </span>
            <h1 class="m-0 text-base font-semibold text-ink">"No son at this address"</h1>
            <p class="m-0 text-[0.8125rem] text-ink-3">"404"</p>
        </section>
    }
}

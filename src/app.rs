use leptos::prelude::*;
use leptos_meta::{provide_meta_context, HashedStylesheet, MetaTags, Title};
use leptos_router::components::{Route, Router, Routes, A};
use leptos_router::hooks::use_location;
use leptos_router::{path, SsrMode};

use crate::api::current_user;
use crate::components::admin::Admin;
use crate::components::detail::SonDetail;
use crate::components::gallery::Gallery;
use crate::components::icon::{
    Ico, LuCirclePlus, LuImage, LuLogOut, LuSearch, LuTrophy, LuUserRound,
};
use crate::components::leaderboard::Leaderboard;
use crate::components::legal::{Privacy, Terms};
use crate::components::search::SearchPage;
use crate::components::search_box::SearchBox;
use crate::components::sort_chips::{GalleryView, SortCtx};
use crate::components::tag_page::TagPage;
use crate::components::upload::Upload;

/// An inline SVG favicon (the crying-face emoji on the accent yellow) --
/// no image asset to generate or keep in sync with the brand color, and it
/// renders crisp at every size browsers ask for.
const FAVICON: &str = "data:image/svg+xml,\
<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'>\
<text y='.9em' font-size='90'>%F0%9F%98%AD</text>\
</svg>";

/// The HTML document. Server-side only entry point; `HydrationScripts` injects
/// the wasm loader so the client picks up where SSR left off.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <link rel="icon" href=FAVICON/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options=options.clone()/>
                // Emitted here rather than as a <Stylesheet> inside App: the
                // hashed filename lives in a file next to the binary, so this
                // needs LeptosOptions, which only exists server-side. See
                // `hash-files` in Cargo.toml for why the hash matters.
                <HashedStylesheet options=options.clone() id="leptos"/>
                <MetaTags/>
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

    view! {
        <Title text="son collection"/>

        <Router>
            // One shell, rearranged entirely by CSS. The primary nav exists
            // exactly once in the DOM and becomes either the desktop sidebar's
            // link list or the mobile bottom bar -- rendering two copies would
            // duplicate landmarks, and branching on window.innerWidth in Rust
            // would invite the hydration mismatch that already took down the
            // wasm module once this session.
            <div class="flex min-h-[100dvh] flex-col">
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
                    <Route path=path!("/tag/:slug") view=TagPage ssr=SsrMode::Async/>
                    <Route path=path!("/search") view=SearchPage ssr=SsrMode::Async/>
                    // Not SsrMode::Async: it carries no SEO-relevant content
                    // and is gated server-side in every fn it calls anyway, so
                    // there is nothing here for out-of-order streaming to leak.
                    <Route path=path!("/admin") view=Admin/>
                        <Route path=path!("/privacy") view=Privacy/>
                        <Route path=path!("/tos") view=Terms/>
                    </Routes>
                </main>
                // Legal links live in a thin footer now that desktop has no
                // sidebar to hang them off. Padded clear of the mobile bottom
                // nav rather than competing with it.
                <footer class="flex justify-center gap-4 px-4 pb-[calc(58px+env(safe-area-inset-bottom)+1rem)] pt-4 text-xs text-ink-3 lg:px-6 lg:pb-8 lg:pt-6">
                    <A href="/privacy">"Privacy"</A>
                    <A href="/tos">"Terms"</A>
                </footer>
            </div>
        </Router>
    }
}

/// The header. On mobile: brand, a search field and the account action. On
/// desktop it additionally carries the section links and -- on the gallery only
/// -- the sort chips, replacing the sidebar entirely.
#[component]
fn Header() -> impl IntoView {
    // Search collapses to an icon on desktop and expands in place. Starts
    // closed, so the server and the first hydration pass agree on the tree.
    let (search_open, set_search_open) = signal(false);

    view! {
        <header class="fixed inset-x-0 top-0 z-30 h-topbar border-b border-line bg-bg lg:h-topbar-lg">
            // Inner wrapper so the bar's contents line up with the content
            // column below it: the bar stays full-bleed (its border needs to
            // reach the viewport edges) while the brand and the icons sit on the
            // same left/right edges as the grid.
            <div class="mx-auto flex h-full max-w-content items-center gap-3 px-4 lg:gap-4 lg:px-8">
            <A href="/" attr:class="flex flex-none items-center gap-2 text-xl font-bold leading-none tracking-tight" attr:aria-label="son collection, home">
                <span class="text-accent">"son"</span>
                <span class="flex flex-none items-center gap-px text-[0.55em] leading-none" aria-hidden="true">
                    <span>"\u{1F62D}"</span>
                    <span>"\u{1F62D}"</span>
                    <span>"\u{1F62D}"</span>
                </span>
            </A>

            <div class="ml-auto flex min-w-0 items-center gap-2">
                // Desktop-only section links, as icons alongside search and the
                // account action. No Gallery entry: the brand is the gallery,
                // which is the default view. Icon-only, so each carries an
                // aria-label and a title -- the label is the accessible name,
                // the title is the hover tooltip.
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

                // Icon on desktop, expanding in place; the mobile bar shows the
                // field directly, since there is room for it there.
                <button
                    class="icon-btn hidden lg:inline-flex"
                    aria-label="Search"
                    aria-expanded=move || search_open.get().to_string()
                    title="Search"
                    on:click=move |_| set_search_open.update(|o| *o = !*o)
                >
                    <Ico icon=LuSearch size=18/>
                </button>
                // Collapsed to nothing on desktop until the icon above is
                // clicked, always visible on mobile where the bar has room.
                // `lg:hidden` rather than a width/opacity transition: an
                // invisible-but-present field is still focusable, so tabbing
                // through the header would land in a search box nobody can see.
                <SearchBox extra_class=Signal::derive(move || {
                    if search_open.get() {
                        "lg:flex lg:w-64".to_string()
                    } else {
                        "lg:hidden".to_string()
                    }
                })/>
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

    view! {
        <Suspense fallback=|| ()>
            {move || {
                user.get()
                    .map(|res| match res {
                        Ok(Some(u)) => {
                            view! {
                                <div class="flex min-w-0 items-center gap-2 text-[0.85rem]">
                                    {u.is_admin
                                        .then(|| {
                                            view! {
                                                <A href="/admin" attr:class="text-[0.8rem] text-accent">"Admin"</A>
                                            }
                                        })}
                                    {u
                                        .avatar_url
                                        .clone()
                                        .map(|src| view! { <img class="h-6 w-6 flex-none rounded-full object-cover" src=src alt=""/> })}
                                    <form method="post" action="/auth/logout" class="inline-flex">
                                        <button
                                            type="submit"
                                            class="icon-btn"
                                            aria-label="Sign out"
                                            title="Sign out"
                                        >
                                            <Ico icon=LuLogOut size=16/>
                                        </button>
                                    </form>
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

#[component]
fn NotFound() -> impl IntoView {
    view! {
        <Title text="no son here — son collection"/>
        <section class="flex min-h-[46vh] flex-col items-center justify-center gap-3 text-center">
            <h1 class="m-0 text-[2rem] font-bold">"404"</h1>
            <p>"No son at this address. Son 😭"</p>
            <A href="/">"back to the collection"</A>
        </section>
    }
}

//! Lucide line icons, wrapped so call sites don't repeat sizing boilerplate.
//!
//! `leptos_icons` renders `icondata` SVG data as a real `<svg>` element in
//! Rust -- no JavaScript icon package, and no Unicode/emoji standing in for an
//! icon. Only the Lucide set is pulled in (`icondata_lu`), and unused icon
//! consts are dead-code-eliminated, so the wasm only carries what's referenced.
//!
//! `currentColor` is inherited by default, so an icon takes the colour of
//! whatever control contains it -- which is what makes a single `.icon-btn`
//! style work for every icon button.

use leptos::prelude::*;
use leptos_icons::Icon;

/// Re-exported so call sites say `icon::LuHeart` rather than reaching for
/// `icondata_lu` directly and coupling every component to the icon set.
pub use icondata_lu::{
    LuArrowLeft, LuCheck, LuCircleAlert, LuCirclePlus, LuCloudUpload, LuDownload, LuEllipsis,
    LuFlag, LuGrid2x2, LuHeart, LuImage, LuLayoutGrid, LuList, LuLogOut, LuMenu, LuRefreshCw,
    LuSearch, LuSun, LuTrophy, LuUserRound, LuX,
};

/// Default icon size in px. The plan's 16-22px band; 18 reads correctly next
/// to 13-14px navigation text and 14-16px control text.
pub const SIZE: u32 = 18;

/// An icon at a given pixel size, inheriting `currentColor`.
///
/// `aria-hidden` is unconditional: an icon here is always either decorative
/// beside a text label, or inside a control that carries its own `aria-label`.
/// Announcing the glyph too would just duplicate the accessible name.
#[component]
pub fn Ico(
    icon: icondata_core::Icon,
    #[prop(optional)] size: Option<u32>,
    #[prop(optional, into)] class: Option<String>,
) -> impl IntoView {
    let px = format!("{}px", size.unwrap_or(SIZE));
    view! {
        <span class=move || format!("ico {}", class.clone().unwrap_or_default()) aria-hidden="true">
            <Icon icon=icon width=px.clone() height=px/>
        </span>
    }
}

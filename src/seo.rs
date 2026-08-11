//! Small URL/text helpers shared by every page component that emits
//! `<Meta>`/`<Link>`/JSON-LD needing an absolute URL or a script-safe string.
//! Not gated behind `ssr`: components render on both the server and (post-
//! hydration) in wasm, so this has to compile under both feature sets.

/// Turns a path or already-absolute URL into an absolute one under
/// `SITE_ORIGIN`. Link unfurlers (Discord, Twitter, Slack) and search engines
/// reject relative `og:image`/canonical URLs, so this matters for every page
/// that emits one.
///
/// Already-absolute input passes through unchanged. `orig_url`/`thumb_url`
/// are already absolute in production (R2 serves from
/// `media.soncollection.com`, a different origin from the app) and only
/// relative in local-disk dev; blindly prepending `SITE_ORIGIN` on top of an
/// already-absolute URL produced a real bug once --
/// `http://site.comhttps://media.site.com/...`.
pub fn absolute(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    #[cfg(feature = "ssr")]
    {
        if let Ok(origin) = std::env::var("SITE_ORIGIN") {
            let origin = origin.trim_end_matches('/');
            if !origin.is_empty() {
                return format!("{origin}{url}");
            }
        }
    }
    url.to_string()
}

/// Escapes a string for embedding inside a JSON string literal that itself
/// sits inside a `<script>` tag (JSON-LD structured data). Hand-rolled rather
/// than pulled from `serde_json`, which isn't available in the wasm/hydrate
/// build this module also compiles under.
///
/// Beyond JSON's own escaping (quotes, backslashes, control characters), this
/// also escapes `<`, `>`, and `&`: a literal `</script>` in a title or
/// uploader name would close the tag early regardless of JSON syntax, since
/// HTML's script-parsing doesn't know or care about JSON.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_url_passes_through_unchanged() {
        assert_eq!(
            absolute("https://media.soncollection.com/orig/x.png"),
            "https://media.soncollection.com/orig/x.png"
        );
    }

    #[test]
    fn json_escape_neutralizes_script_close() {
        let escaped = json_escape("</script><script>alert(1)</script>");
        assert!(!escaped.contains("</script>"));
        assert!(escaped.contains("\\u003c/script\\u003e"));
    }

    #[test]
    fn json_escape_handles_quotes_and_backslashes() {
        assert_eq!(
            json_escape("a \"quote\" and \\backslash"),
            "a \\\"quote\\\" and \\\\backslash"
        );
    }
}

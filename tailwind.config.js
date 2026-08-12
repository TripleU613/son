/** @type {import('tailwindcss').Config} */
module.exports = {
  // Classes live inside Leptos `view!` macros in .rs files, not in .html/.jsx,
  // so the scanner has to read Rust source or every utility gets purged.
  content: ["./src/**/*.rs"],
  theme: {
    extend: {
      // The palette, and the single source of truth for it: utilities like
      // `bg-surface` and `text-ink-2` exist because of this block, so nothing
      // downstream ever hardcodes a hex.
      //
      // The neutral ramp is hue 45 (warm) at matched luminance, re-hued from
      // the hue-220 blue-charcoal it started as. That re-hue is the single
      // largest "more yellow" move available and it costs no contrast at all,
      // because it rotates hue without brightening: every border, panel, field
      // and page background on the site is one of these nine tokens, so the
      // whole UI reads warm without one large yellow fill anywhere.
      //
      // Ratios measured against the bg below, not estimated:
      //   ink 18.21, ink-2 9.50, ink-3 5.72, accent 13.06, accent-hover 14.18,
      //   accent-ink on accent 13.13, accent-muted 8.01.
      // `ink.3` was raised as part of the re-hue specifically because the value
      // it replaces measured 4.44:1 on the old bg -- below the 4.5:1 AA floor
      // for normal text, on the colour used for every byline and timestamp on
      // the site. It is now 5.72:1 on bg, 5.45:1 on surface, 5.11:1 on
      // surface-raised, so it passes on all three surfaces it actually lands on.
      colors: {
        bg: "#0c0b08",
        surface: {
          DEFAULT: "#13120d",
          raised: "#1b1912",
          hover: "#232017",
        },
        line: {
          DEFAULT: "#383429",
          strong: "#4e4839",
        },
        ink: {
          DEFAULT: "#f7f6f3",
          2: "#b8b4a8",
          3: "#908a7a",
        },
        // The accent is a scale, not one hex repeated, so "more yellow" has
        // vocabulary: a wash, a hairline and a muted text tone are three
        // different jobs and reusing DEFAULT for all of them is what forces the
        // choice between "invisible" and "shouting".
        accent: {
          // The brand mark's colour. Do not retune this one.
          DEFAULT: "#ffcc33",
          hover: "#ffd75e",
          active: "#eab900",
          // Pressed-chip fill. Raised from 0.1 -- measured on the composite,
          // accent text on it is still 9.96:1 and ink is 13.88:1, so the fill
          // gets visibly warmer at no contrast cost whatsoever.
          soft: "rgba(255, 204, 51, 0.14)",
          // Large, low-stakes washes: the rank-1 leaderboard row, the
          // empty-state icon badge. Deliberately too faint to carry meaning on
          // its own -- everything sitting on it is also marked some other way.
          veil: "rgba(255, 204, 51, 0.05)",
          // The quiet accent hairline. This is the token that turns every card
          // and chip hover yellow, which is a lot of yellow for one value.
          line: "rgba(255, 204, 51, 0.28)",
          // Text that should read yellow without shouting: 8.01:1 on bg and
          // 7.63:1 on surface, so it is a legitimate body-text colour and not
          // just decoration.
          muted: "#c9a02e",
          border: "rgba(255, 204, 51, 0.75)",
          ink: "#0a0a0b",
        },
        danger: "#ef6a6a",
        ok: "#4ee39a",
      },
      borderRadius: {
        sm: "8px",
        DEFAULT: "10px",
        lg: "13px",
      },
      spacing: {
        // Chrome heights, referenced by the content padding that clears them.
        topbar: "56px",
        "topbar-lg": "60px",
        bottomnav: "58px",
        // `h-13`/`w-13` were in empty.rs and emitted *zero* rules: Tailwind's
        // default spacing scale jumps 12 -> 14, so both icon badges were
        // unsized dead classes that only looked right because the padding and
        // the glyph size happened to add up. Verified by grepping the compiled
        // stylesheet, which had no `.h-13` rule in it at all.
        13: "3.25rem",
      },
      fontFamily: {
        sans: [
          "Inter",
          "Geist",
          "system-ui",
          "-apple-system",
          "BlinkMacSystemFont",
          "Segoe UI",
          "Roboto",
          "sans-serif",
        ],
      },
      maxWidth: {
        content: "1320px",
        wide: "1800px",
      },
      // Motion lives here as config keyframes rather than hand-written CSS so
      // it comes out as real utilities (`animate-rise-in`) the scanner can see
      // and purge like anything else. Hand-rolled @keyframes in the stylesheet
      // would be exempt from that and would drift.
      keyframes: {
        shimmer: {
          "0%": { backgroundPosition: "-150% 0" },
          "100%": { backgroundPosition: "250% 0" },
        },
        "rise-in": {
          "0%": { opacity: "0", transform: "translateY(10px)" },
          "100%": { opacity: "1", transform: "none" },
        },
        "fade-in": {
          "0%": { opacity: "0" },
          "100%": { opacity: "1" },
        },
      },
      animation: {
        shimmer: "shimmer 1.9s cubic-bezier(.4, 0, .6, 1) infinite",
        // The `both` fill-mode on the two entrance animations is load-bearing,
        // not decoration. The global prefers-reduced-motion block clamps
        // animation-duration to 0.01ms; without `both`, an element whose
        // animation has effectively already finished falls back to its
        // *unanimated* style only if a fill is in force. Without one it can be
        // left showing the 0% frame -- opacity 0 -- which is content that
        // exists in the DOM and cannot be seen, for exactly the users who
        // asked for less motion.
        "rise-in": "rise-in .34s cubic-bezier(.16, 1, .3, 1) both",
        "fade-in": "fade-in .22s ease-out both",
      },
      backgroundImage: {
        // The skeleton's travelling highlight. Warm rather than the usual
        // white, so a loading screen is a yellow moment instead of a grid of
        // grey boxes -- the cheapest place on the site to put accent colour,
        // because nothing is legible there yet to lose contrast against.
        sheen:
          "linear-gradient(100deg, transparent 20%, rgba(255, 204, 51, 0.07) 45%, transparent 70%)",
      },
    },
  },
  plugins: [],
};

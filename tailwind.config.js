/** @type {import('tailwindcss').Config} */
module.exports = {
  // Classes live inside Leptos `view!` macros in .rs files, not in .html/.jsx,
  // so the scanner has to read Rust source or every utility gets purged.
  content: ["./src/**/*.rs"],
  theme: {
    extend: {
      // The same neutral-charcoal + one-warm-yellow palette as before, moved
      // from CSS custom properties into the theme so utilities like `bg-surface`
      // and `text-ink-2` exist and the palette stays the single source of truth.
      colors: {
        bg: "#08090b",
        surface: {
          DEFAULT: "#0d0f12",
          raised: "#121419",
          hover: "#171a20",
        },
        line: {
          DEFAULT: "#292d35",
          strong: "#3a3f49",
        },
        ink: {
          DEFAULT: "#f4f4f5",
          2: "#a6a8b0",
          3: "#737780",
        },
        accent: {
          DEFAULT: "#ffcc33",
          hover: "#ffd85c",
          active: "#eab900",
          soft: "rgba(255, 204, 51, 0.1)",
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
    },
  },
  plugins: [],
};

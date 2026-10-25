export default {
  content: ["./index.html", "./src/**/*.{vue,ts}"],
  theme: {
    extend: {
      colors: {
        "surface-3": "var(--surface-3)",
        "surface-4": "var(--surface-4)",
        "surface-5": "var(--surface-5)",
        primary: "var(--color-base)",
        contrast: "var(--color-contrast)",
        "button-bg": "var(--surface-4)",
        "brand-shadow": "var(--brand-shadow)",
      },
    },
  },
  plugins: [],
};

// Vitest setup (WP-18): every test sees the registered i18n catalogs, as the app does
// after main.tsx imports app/i18n. Component tests opt into jsdom per file.
import "../src/app/i18n";

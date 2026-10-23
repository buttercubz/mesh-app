# TypeScript conventions

Applies to `packages/*`, `apps/mobile` and `tools/lab-analyze`. Design references:
DESIGN §2.4, §3.7, §11, §13.1. These are the convLink rules carried over, plus the
intercom-specific ones.

## Tooling

- **pnpm only** (`pnpm@11.5.3`, Node ≥ 22.18). Never npm, yarn, bun or `npx`; the
  Capacitor CLI runs as `pnpm exec cap …`.
- Workspace packages are consumed from `dist/`: build dependencies before testing a
  consumer, `pnpm --filter "<pkg>..." build` (e.g. `"@meshapp/mobile..."`).
- Every package extends the root `tsconfig.base.json` and `eslint.config.js`.
- Versions: React 19.2.x, Vite 8.x, TypeScript 5.9.x, Tailwind 4.x, Zustand 5.x,
  i18next 26.x + react-i18next, Vitest 4.x, Playwright, Capacitor 8.5.x.

## Compiler and lint rules (convLink)

- `strict: true`, `verbatimModuleSyntax: true`, `erasableSyntaxOnly: true`,
  `noUnusedLocals`, `noUnusedParameters`, `noFallthroughCasesInSwitch`,
  `noUncheckedSideEffectImports`.
- Consequences: `import type` for type-only imports; no `enum`, no `namespace`, no
  parameter properties (use union types and `as const` objects).
- **Every `catch` block has a comment** explaining why the error is handled or
  ignored (`no-empty` with `allowEmptyCatch: false`). No silent failures.
- No `any` in new code (use `unknown` + guards); `@ts-ignore` only with a reason.

## React

- React 19 with the **React Compiler** on (`babel-plugin-react-compiler`); **no
  `StrictMode`** (it double-invokes effects that drive the native engine). Follow the
  Rules of React; do not add manual `useMemo`/`useCallback` unless the compiler
  cannot handle the case.
- Function components only. Primitives come from `@meshapp/ui-kit`; app
  compositions live in `apps/mobile/src/components/`.
- Driving rules: minimum touch target 44 px (PTT 160 px), dark theme default, **no
  text input on the ride screen**, every state also announced by earcons.

## State

- **One Zustand store per domain** (`useEngineStore`, `useGroupsStore`,
  `useSettingsStore`, `useLabStore`); non-React code reads them with `getState()`.
- Persisted stores declare `name`, `version`, `migrate` and `merge`.
- **No secrets in web storage**: no keys, epoch secrets or invite text in
  localStorage, IndexedDB or any persisted store. `useGroupsStore` holds metadata only
  (id, name, role, lastUsed). Invite URLs from deep links go straight to
  `parseInvite` and are never stored.

## Engine access

- The UI talks to the engine only through `@meshapp/engine-api`
  (`createEngineClient(MeshIntercom)`); generated types in `src/generated/` are written
  by `cargo xtask gen-ts` and never edited by hand. Discriminant guards live in
  `engine-api`, not in components.
- **Gesture commands:** `goLive`, `resumeMic`, `labStart` with `audio ≠ "off"` and
  `audioCheck{op:"start"}` go only through `gestureCommand` (or the `goLive()` /
  `resumeMic()` sugar), called synchronously from the tap handler. `send` rejects
  them with `useGestureCommand`, natively and in `web.ts`. A lint rule and tests
  enforce it.
- One event subscription (`useEngineEvents`) fans events into stores. Snapshots are
  the source of truth; request a full snapshot on foreground return (WebView timers
  are not trusted while backgrounded).
- Lobby and PTT ride screens call `setKeepScreenOn(true)` on mount and `false` on
  unmount.

## i18n

- All user-facing text goes through i18next catalogs built with convLink's
  `defineCatalog(en, es)` (English source, Spanish parity checked at compile time and
  by a test). Locale `es` (neutral) and `en`.
- Namespaces and owners: `common`, `errors`, `notices`, `prompts` (WP-18); `lab`
  (WP-21); `onboarding`, `group`, `ride`, `settings` (WP-22).
- The engine sends i18n keys plus params; every engine key has a `notices`/`prompts`
  entry (parity test). Demo-ride bot lines in `prompts` may be clearly marked
  placeholders.

## Product rules

- **No text chat**, ever. No Cardo/Sena trademarks in copy. The honest range promise
  (DESIGN §0.2) is used verbatim in onboarding.

## Tests

- Vitest (node + jsdom) for guards, stores (persist migrate/merge), i18n parity,
  components from fixture snapshots, PTT pointercancel/visibility handling.
- Playwright e2e against `vite dev` + the WASM simulator (`apps/mobile/e2e/`).
- Static contract tests (`apps/mobile/tests/*.test.mjs`) for Info.plist,
  entitlements, PrivacyInfo, AndroidManifest and `capacitor.config`.

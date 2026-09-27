# piarapk

Telegram multi-account promotion tool. Rewrite of tg-piar (TypeScript) on Flutter/Dart + Rust.

**Stack:** Flutter (desktop: macOS, Windows) + Rust core ([grammers](https://codeberg.org/Lonami/grammers), MTProto) via flutter_rust_bridge + SQLite (drift).

## Functionality (target)

1. **Accounts** — add via phone (code + 2FA), import tdata / StringSession; two separate pools: promo accounts and parser accounts.
2. **Shop** ([dark.shopping](https://dark.shopping/), official API) — two storefronts: buy accounts and buy chats; balance, orders, auto-import of delivered goods.
3. **Promo (piar)** — invite users into a chat in batches (flood-aware), post a message, chat is read-only for members.
4. **Parser** — collect chat participants / message authors (ported from the old working tg-piar scraper), separate window, separate account pool.

## Architecture

```
lib/          Flutter UI (screens, state, DB via drift, dark.shopping API client)
rust/         Rust core (grammers): account pool, auth flows, session import,
              invite engine, scraper, read-only channel setup
  rust/src/api/   flutter_rust_bridge API surface
  rust/src/core/  telegram logic (modules)
codemagic.yaml  CI: build macOS .app in Codemagic cloud
```

## Development

```bash
flutter pub get          # deps
flutter analyze          # static analysis (runs in CI)
flutter test             # dart tests
# Rust:
cd rust && cargo check   # type-check the core (run with +stable-x86_64-pc-windows-gnu on Windows w/o VS)
# Bridge codegen after editing rust/src/api/:
flutter_rust_bridge_codegen generate
```

## Security notes

- Sessions are credentials. Never commit `data/`, `*.session*`, `.env`.
- dark.shopping API key lives in local config, never in the repo.

## Status

Phase 1 — project scaffold. See docs/PLAN.md for the roadmap.

# Contributing to Rustbound Idle

Thank you for being here. You do not need to write Rust to contribute. Ideas, testing,
documentation, design, accessibility feedback, and welcoming other contributors all count.

Rustbound Idle is the companion project to the
[Rustbound series](https://journeytocode.io/series/rustbound-build-your-first-rust-app/),
which builds this codebase from scratch, one article at a time. You do not have to read it
before contributing, but it is the fastest way to understand *why* the code looks the way it
does, and the larger issues assume you have.

## Where to start

- Have an idea or question? Open a [Discussion](https://github.com/journey-to-code-org/rustbound-idle/discussions).
  Half-formed is fine. That is what Discussions are for.
- Found a bug? Use the bug-report issue form.
- Want something small? Look for the [`good first issue`](https://github.com/journey-to-code-org/rustbound-idle/labels/good%20first%20issue) label.
- Stuck? Ask publicly in the issue or Discussion. Your question may help the next person.

Please do not open a large pull request without an issue or Discussion first. A short conversation can
prevent two people from solving different versions of the same problem.

## Run it locally

You need Rust, the `wasm32-unknown-unknown` target, `wasm-pack`, Node, and Python 3.

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
cd web
npm install
cd ..
bash ./scripts/dev.sh
```

The game will be available at <http://localhost:8080/>.

## Three boundaries that protect the project

1. The Rust engine never learns about the browser. No DOM, `localStorage`, clocks, or service workers.
2. The browser never calculates game rules. Damage, rewards, prices, XP, and enemy strength come from Rust.
3. The service worker only makes application files available offline. It owns no game logic or saves.

If a useful change seems to cross one of these boundaries, open a Discussion. That is a design
conversation, not a rejection.

## Before opening a pull request

On Windows, keep Cargo at two build jobs to avoid exhausting linker memory.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test -j 2 --all-targets
```

The Rust test suite also checks that web/sw.js caches the files copied into
dist by scripts/build.sh. Run cargo test -j 2 --all-targets to check for drift.
The checker accepts the build script's fixed setup and plain copies from web/ into
dist/, dist/icons/, and dist/pkg/. If the build commands change, update the checker
to understand the new form; unsupported commands fail the test.
It also expects a literal APP_SHELL array and a simple service-worker preamble.

If you changed browser code, build and serve the release, then run both browser checks:

```bash
bash ./scripts/build.sh
python3 -m http.server 8080 --directory dist
```

In another terminal:

```bash
cd web
npm install
npx playwright install chromium
node check.mjs http://localhost:8080/
npm run check:offline
```

## Pull requests

Small is welcome. One test, one CSS fix, or one clarified sentence is a real contribution.
Explain what changed, why it helps, and how you checked it. Review may ask for changes; that is a normal
part of making the project better together.

## Using AI tools

You are welcome to use AI assistants, code generators, or agents while working on Rustbound Idle.
Two things make that work well for everyone:

**Say so.** Mention in the pull request that a tool helped, and roughly how. This is not a
confession. It helps reviewers know where to look, and it helps the next contributor calibrate
what the tool is good at here.

**You are the author.** Commits must be authored by you: your name, your email, your GitHub
account. If you want to credit the tool, add a `Co-authored-by:` trailer to the commit message.
A commit authored by a bot or tool account, with a person only opening the pull request, will be
sent back with a request to re-author it. Most tools have a setting for this; if yours does not,
`git commit --amend --reset-author` fixes a commit before you push.

This is not a rule about whether you used a tool. It is about who gets credit and who stands behind
the change. This is a learning project, and the person learning is the one whose name belongs on
the work.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

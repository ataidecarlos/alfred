# Third-Party Notices

Alfred is distributed under the MIT License (see `LICENSE`). It depends on the
following third-party software. Where a dependency is invoked as a separate
process rather than linked or vendored, that is noted — the attribution is
still required by the license terms.

## Pi Agent Harness

- **Component:** `@earendil-works/pi-coding-agent` (and its sibling packages
  `@earendil-works/pi-agent-core`, `@earendil-works/pi-ai`)
- **Project:** https://github.com/earendil-works/pi
- **Use:** Invoked as a subprocess (`pi --mode rpc`) to run the agent loop,
  LLM providers, and tool calls. Not linked, not vendored.
- **License:** MIT

```
MIT License

Copyright (c) 2025 Mario Zechner

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Deferred / not currently included

These are recorded so the attribution requirements are understood before
adoption. Neither is a dependency of the current build.

- **Laya** (`convaiinnovations/laya`) — Apache-2.0. Decision model, deferred
  (see `ROADMAP.md`). Apache-2.0 requires preserving the NOTICE file and
  stating changes if the weights or code are redistributed.
- **pi-chat** (`earendil-works/pi-chat`) — MIT. Vendors portions of the Vercel
  Chat SDK (MIT). Deferred; re-evaluate after release.

## Rust crate dependencies

Rust crates used by Alfred carry their own licenses, enumerated by
`cargo metadata` / `cargo deny`. Generate the authoritative list from the
lockfile rather than maintaining it by hand.

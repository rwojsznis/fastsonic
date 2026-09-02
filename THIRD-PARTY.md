# Third-party code vendored into this repository

Fastsonic is MIT licensed (see `LICENSE`). Some source files are copied from
other projects rather than pulled in as dependencies; their notices are kept
here, as their licences require.

Ordinary Cargo dependencies are not listed — they carry their own licences in
the crates.io registry and are not copied into this tree.

---

## `src/api/subsonic/types.rs` — opensubsonic 0.4.0

The Subsonic / OpenSubsonic response types are adapted from the `data` module
of the [`opensubsonic`](https://github.com/M0Rf30/opensubsonic-rs) crate,
version 0.4.0, dual-licensed MIT OR Apache-2.0. Fastsonic takes it under MIT.

They are vendored rather than depended on because the crate is young and sits
at the centre of this app's data layer, while the transport it also provides
is a handful of query-parameter `GET`s that this app builds itself
(`migration/00-decisions.md`, D3). The types are trimmed to what this client
reads and every field is defaulted, so the shapes have diverged from the
originals.

```
MIT License

Copyright (c) 2026 Gianluca Boiano

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

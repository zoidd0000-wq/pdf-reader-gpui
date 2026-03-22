# pdf-reader-gpui

This is a GUI program that renders PDF files.

The GUI is implemented using [`gpui`](https://www.gpui.rs/): "A fast, productive UI framework for Rust from the creators of Zed".

## Usage

- Build a release version locally using `cargo build --release` then run `target/release/pdf-reader-gpui.exe`.
  - When developing use `cargo run` instead
- Or download a precompiled executable from the [latest GitHub release](https://github.com/Lej77/pdf-reader-gpui/releases).
- Build as website using [trunk](https://trunkrs.dev/) (`cd trunk` then `trunk serve` or `trunk build --release`).
  - You can try the current web demo at: <https://lej77.github.io/pdf-reader-gpui/>

### `cargo install`

You can use `cargo install` to easily build from source without manually cloning the repo:

```bash
cargo install --git https://github.com/Lej77/pdf-reader-gpui.git
```

You can use [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall) to easily download the precompiled executables from a GitHub release:

```bash
cargo binstall --git https://github.com/Lej77/pdf-reader-gpui.git pdf-reader-gpui
```

After installing you can update the program using [nabijaczleweli/cargo-update: A cargo subcommand for checking and applying updates to installed executables](https://github.com/nabijaczleweli/cargo-update):

```bash
cargo install-update --git pdf-reader-gpui

# OR update all installed programs:
cargo install-update --git --all
```

You can uninstall uisng:

```bash
 cargo uninstall pdf-reader-gpui
```

## References

- GUI
  - [GPUI](https://www.gpui.rs/)
    - [GitHub](https://github.com/zed-industries/zed/tree/main/crates/gpui)
    - [docs.rs](https://docs.rs/gpui/latest/gpui/)
  - [GPUI Component](https://longbridge.github.io/gpui-component/)
    - [GitHub](https://github.com/longbridge/gpui-component)
    - [docs.rs](https://docs.rs/gpui-component/latest/gpui_component/)
    - [longbridge/gpui-component | DeepWiki](https://deepwiki.com/longbridge/gpui-component)
- PDF
  - [`mupdf` -  Safe Rust wrapper to MuPDF ](https://crates.io/crates/mupdf/0.5.0)
    - Used by `miro`, `tdf` and `MView6`.
    - License: `AGPL-3.0`
  - [`pdfium-render` -  A high-level idiomatic Rust wrapper around Pdfium, the C++ PDF library used by the Google Chromium project. ](https://crates.io/crates/pdfium-render)
    - License: `MIT` or `Apache-2.0`
  - [`hayro` -  A rasterizer for PDF files. ](https://crates.io/crates/hayro)
    - Has a [demo website](https://laurenzv.github.io/hayro/).
    - License: `Apache-2.0`
  - [`poppler` -  Wrapper for the GPL-licensed Poppler PDF rendering library. ](https://crates.io/crates/poppler)
    - License: `GPL-2.0`
  - [`printpdf` -  Rust library for reading and writing PDF files ](https://crates.io/crates/printpdf)
    - Has a [printpdf-wasm Demo](https://fschutt.github.io/printpdf/)
    - License: `MIT`
  - [`pdfium` - Modern Rust interface to PDFium, the PDF library from Google](https://crates.io/crates/pdfium)
    - Used by `MView6`.
    - License: `GPL-3.0`
  - [PDF.js - A general-purpose, web standards-based platform for parsing and rendering PDFs.](https://mozilla.github.io/pdf.js/)
    - Can maybe be used inside a webview.

## Alternatives

There are some other Rust projects that can be used to view PDF files:
- [vincent-uden/miro: A native pdf viewer for Windows and Linux (Wayland/X11) with configurable keybindings.](https://github.com/vincent-uden/miro)
- [itsjunetime/tdf: A tui-based PDF viewer](https://github.com/itsjunetime/tdf)
- [newinnovations/MView6: High-performance PDF and photo viewer built with Rust and GTK4](https://github.com/newinnovations/MView6?tab=readme-ov-file)

## License

This project is released under [Apache License (Version 2.0)](./LICENSE-APACHE).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

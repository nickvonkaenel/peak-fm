const SYNTECT_LICENSE: &str = r#"MIT License

Copyright (c) 2017 Tristan Hume, Keith Hall, Google Inc and other contributors

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
SOFTWARE."#;

const TWO_FACE_LICENSE: &str = r#"MIT License

Copyright (c) 2023-2023 The `two-face` developers (https://github.com/CosmicHorrorDev/two-face).

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
SOFTWARE."#;

const NO_CLOWN_FIESTA_NOTICE: &str = r#"Bundled asset: `src/core/noclownfiesta.tmTheme`

SHA-256: `497436e882381943c576a003ffd62cf0221fc9d1f52cd7372be1ffd47a047f7c`

This is the legacy TextMate adaptation previously shipped by `fm`, based on No
Clown Fiesta by Gustaf Rydholm (aktersnurra).

Upstream source: <https://github.com/aktersnurra/no-clown-fiesta.nvim>

License status was checked at upstream revision
[`04b808e9769ded988089772ffcaf08d92d75d927`](https://github.com/aktersnurra/no-clown-fiesta.nvim/commit/04b808e9769ded988089772ffcaf08d92d75d927):
no license file or permission grant was found. The upstream revision used to
create this adaptation and the adaptation's author were not recorded.

This is a provenance notice, not a license grant. Peak File Manager's MIT
license does not apply to this asset, and Peak File Manager grants no
redistribution rights to the upstream material. The theme is bundled with
attribution at the Peak File Manager maintainer's direction."#;

fn main() {
    let acknowledgements = two_face::acknowledgement::listing().to_md();
    let document = format!(
        "# Syntax and Theme Licenses and Provenance Notices\n\n\
This file covers the software and embedded assets used by Peak File Manager's \
syntax-highlighting and theme features. It is not a complete inventory of \
every Rust dependency.\n\n\
The dependency-derived notices below were generated for the exact versions \
locked by this release. A provenance notice documents the separately bundled \
No Clown Fiesta adaptation, for which no upstream license was found.\n\n\
## No Clown Fiesta TextMate adaptation\n\n\
{NO_CLOWN_FIESTA_NOTICE}\n\n\
## Syntect 5.3.0\n\n\
Source: <https://github.com/trishume/syntect>\n\n\
````text\n{SYNTECT_LICENSE}\n````\n\n\
## two-face 0.4.5\n\n\
Source: <https://github.com/CosmicHorrorDev/two-face>\n\n\
````text\n{TWO_FACE_LICENSE}\n````\n\n\
## Embedded syntax and theme assets supplied by two-face 0.4.5\n\n\
{acknowledgements}"
    );
    let normalized = document
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = normalized.trim_end().to_owned() + "\n";
    std::fs::write("SYNTAX_THEME_LICENSES.md", normalized).expect("write SYNTAX_THEME_LICENSES.md");
}

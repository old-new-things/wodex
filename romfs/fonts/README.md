# Bundled font

Noto Sans CJK SC Regular, version 2.004.

Copyright © 2014–2021 Adobe (http://www.adobe.com/).

Unmodified upstream file:
https://github.com/notofonts/noto-cjk/blob/Sans2.004/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf

SHA-256: `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`.

Distributed under the SIL Open Font License 1.1 in `OFL.txt`, copied from the same release:
https://github.com/notofonts/noto-cjk/blob/Sans2.004/LICENSE

This is the full language-specific Regular OTF, with 44,810 Unicode mappings. It covers Latin and broad Simplified Chinese, Traditional Chinese, Japanese and Korean text, using Simplified Chinese regional glyph forms. It is not a subset of the app's interface text; model replies may contain arbitrary characters. It does not cover every Unicode character or all emoji.

Switch packages include this directory in RomFS. Desktop builds copy it to `resources/fonts` beside the executable. SDL_ttf/FreeType reads the file through its stream API, and the renderer caches only visible text textures.

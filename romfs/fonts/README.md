# Bundled fonts

## Text: Noto Sans CJK

Noto Sans CJK SC Regular, version 2.004.

Copyright © 2014–2021 Adobe (http://www.adobe.com/).

Unmodified upstream file:
https://github.com/notofonts/noto-cjk/blob/Sans2.004/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf

SHA-256: `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`.

Distributed under the SIL Open Font License 1.1 in `OFL.txt`, copied from the same release:
https://github.com/notofonts/noto-cjk/blob/Sans2.004/LICENSE

This is the full language-specific Regular OTF, with 44,810 Unicode mappings. It covers Latin and broad Simplified Chinese, Traditional Chinese, Japanese and Korean text, using Simplified Chinese regional glyph forms. It is not a subset of the app's interface text; model replies may contain arbitrary characters. It does not cover every Unicode character or all emoji.

Switch packages include this directory in RomFS. Desktop builds copy it to `resources/fonts` beside the executable. SDL_ttf/FreeType reads the file through its stream API, and the renderer caches only visible text textures.

## Code: Source Code Pro

Source Code Pro Regular, upright version 2.042, unmodified OpenType font from Adobe's official repository. The 131,128-byte font provides compact, equal-width ASCII code glyphs. Chinese and other non-ASCII code text continue using Noto Sans CJK, with two cells reserved for wide characters.

Copyright © 2023 Adobe (http://www.adobe.com/), with Reserved Font Name "Source".

Pinned upstream commit: `803b7e23ec97ae58b6232ea76519a76d428ba268`.

Font source:
https://github.com/adobe-fonts/source-code-pro/blob/803b7e23ec97ae58b6232ea76519a76d428ba268/OTF/SourceCodePro-Regular.otf

SHA-256: `9f9664e2edf6f045c11e774f9bd0be6993971f2544a39061a5ce478b96b051f8`.

Distributed under SIL Open Font License 1.1. The unmodified upstream license is in `SourceCodePro-OFL.txt`, also included in the binary's third-party license archive:
https://github.com/adobe-fonts/source-code-pro/blob/803b7e23ec97ae58b6232ea76519a76d428ba268/LICENSE.md

License SHA-256: `7c940e28a5388e9bba866cf0e408edda45fe0899ba98665b8f6ab31dc5e4b8ff`.

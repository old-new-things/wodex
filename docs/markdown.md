# Markdown in conversations

Wodex renders Markdown in assistant answers and in the optional thinking view.
User prompts and the composer show their original text. Markdown source remains
unchanged in saved history and in requests sent to the selected provider.

The renderer runs in the Rust `no_std` application. SDL and SDL_ttf provide native
font metrics and drawing; Markdown does not introduce a browser or web view.
Body text uses the bundled Noto Sans CJK font. Inline and fenced code use the
bundled Source Code Pro font for ASCII, with Noto Sans CJK for other Unicode
characters. Wide characters occupy two code cells.

## Supported formatting

| Syntax | Display |
| --- | --- |
| Headings and paragraphs | Sized headings, wrapped paragraphs, preserved line breaks |
| `**bold**`, `*italic*`, `~~strike~~` | Styled text, including combinations |
| Inline code | Monospace text with a contrasting background |
| Ordered, unordered, and nested lists | Markers and bounded indentation |
| `- [ ]` and `- [x]` | Read-only task markers |
| `>` quotes and horizontal rules | Quote borders and divider lines |
| Fenced and indented code | Preserved indentation, fixed character cells, wrapped long lines |
| Pipe tables | Header rows, cell wrapping, and left/center/right alignment |
| Inline and reference links | Readable label and destination |
| Inline and reference images | PNG/JPEG previews with alternative text |

Code fences show their language label when supplied. Code has no syntax
highlighting. Wide tables become labeled rows so every column remains readable
on the handheld screen. Repeated labels use at most 96 bytes of their header;
longer headers are also shown in full once. Lists and quotes limit visual
indentation to preserve room for text.

Links and tasks are not interactive. Raw HTML is displayed literally with code
styling. Wodex does not execute HTML or open links.

During streaming, an unfinished code fence stays a code block through the
current end of the response. Unmatched inline delimiters remain readable as
literal text. Formatting can settle as more of the response arrives.

## Images

Images in assistant replies and thinking text use ordinary Markdown:

```md
![A diagram / 示意图](https://example.com/diagram.png)
```

A complete image reference loads when its card is visible. Wodex downloads one
image at a time in the background, decodes it in Rust, and fits the preview inside
a fixed card without stretching it. Transparent PNG pixels blend with the card
background. Alternative text appears below the image. Loading and error states
keep the same card size so text does not jump when a download finishes.

URLs must use HTTPS and point directly to PNG or JPEG data. Downloads do not
include the chat API key, cookies, conversation text, or a referrer, and redirects
are rejected. Local files, embedded `data:` URLs, and URLs with credentials are
not loaded. Plain links, images inside code, unfinished image syntax, and user
prompts do not trigger image downloads.

Each download is limited to 4 MiB. Source dimensions are limited to 4,096 pixels
per side and 2,097,152 pixels in total. Previews are reduced to at most 1,024
pixels per side before creating an SDL texture. PNG supports grayscale, palette,
RGB and RGBA data up to 8 bits per component; 16-bit PNG, GIF, WebP and SVG are
not supported. Animated PNG shows its default static image. Image metadata and
color profiles are discarded; JPEG EXIF orientation is not applied. Image upload,
full-screen viewing, animation and image generation are outside this feature.

Image textures use an in-memory 16 MiB cache with at most 32 URL entries.
Repeated references share a download. Failures remain readable and do not retry
every frame. Leaving a conversation clears its image cache and cancels its
pending download; returning can fetch the image again. Images are not saved to
the SD card. Original Markdown remains in conversation history.

Demo and `--screenshot` runs disable image downloads, keeping generated previews
offline. Ordinary interactive use loads visible images.

## Rendering limits

Markdown uses bounded parsing and layout caches. The parser accepts at most
512 KiB of source, 64 levels of nesting, 2,048 blocks, 8,192 spans, 65,536 parser
events, 32 table columns, 4,096 table cells, and 128 image references per message.
Expanded display text is limited
to four times the input size plus 4 KiB. Rich layout also limits source lines and
laid-out lines to 8,192, with at most 32,768 styled text runs. If a limit is
exceeded, Wodex displays the complete original message as plain text instead of
discarding content.

Rich layouts use a 16 MiB cache with at most 256 entries. Only lines intersecting
the visible chat area create text textures, including when scrolling through
long answers. These rendering limits do not change the existing conversation,
message, or saved-history limits.

## Visual checks

The saved conversations in `tests/fixtures/markdown-*.json` exercise typography
and CJK, quotes and tasks, fenced code, regular and wide tables, interrupted
responses, and offline image placeholders. They contain no API keys. The partial
response intentionally has an unclosed fence and unfinished inline syntax.

After building the desktop application, render all fixtures with:

```sh
python3 tests/preview_markdown.py build/desktop/wodex
```

The script copies each fixture into a temporary data directory, uses SDL's dummy
video driver, and writes BMP screenshots under `build/desktop/markdown-previews`.
An optional second argument selects another output directory. Screenshots use
the production renderer and do not send network requests or read user history.
The chat opens at the bottom, matching ordinary conversation behavior.

The `image_integration` CTest suite also runs the production application against
an isolated loopback server. Its test-only HTTP wrapper routes synthetic HTTPS
fixture URLs to that server while leaving production URL validation intact.
The suite checks actual PNG/JPEG pixels, transparency and aspect ratio, cache
reuse, failures, cancellation and credential isolation, and writes previews to
`build/desktop/image-previews`. It requires no external downloads. To run it
directly:

```sh
python3 tests/mock_images.py build/desktop/wodex_image_tests \
  build/desktop/resources build/desktop/image-previews
```

Synthetic image fixtures contain no metadata or personal content. Their
regeneration script is `tests/fixtures/generate_images.py`; Pillow is needed
only to regenerate those committed fixtures, not to run the tests.

On macOS, convert a screenshot for preview with:

```sh
sips -s format png build/desktop/markdown-previews/markdown-typography.bmp \
  --out build/desktop/markdown-previews/markdown-typography.png
```

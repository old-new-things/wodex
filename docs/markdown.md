# Markdown in conversations

Wodex renders Markdown in assistant answers and in the optional thinking view.
User prompts and the composer show their original text. Markdown source remains
unchanged in saved history and in requests sent to DeepSeek.

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
| Images | Readable alternative text and destination |

Code fences show their language label when supplied. Code has no syntax
highlighting. Wide tables become labeled rows so every column remains readable
on the handheld screen. Repeated labels use at most 96 bytes of their header;
longer headers are also shown in full once. Lists and quotes limit visual
indentation to preserve room for text.

Links and tasks are not interactive. Image URLs are shown as text; the renderer
does not download images, open links, execute HTML, or send network requests.
Raw HTML is displayed literally with code styling. Rendering happens locally.

During streaming, an unfinished code fence stays a code block through the
current end of the response. Unmatched inline delimiters remain readable as
literal text. Formatting can settle as more of the response arrives.

## Rendering limits

Markdown uses bounded parsing and layout caches. The parser accepts at most
512 KiB of source, 64 levels of nesting, 2,048 blocks, 8,192 spans, 65,536 parser
events, 32 table columns, and 4,096 table cells. Expanded display text is limited
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
and CJK, quotes and tasks, fenced code, regular and wide tables, and interrupted
responses. They contain no API keys. The partial response intentionally has an
unclosed fence and unfinished inline syntax.

After building the desktop application, render all fixtures with:

```sh
python3 tests/preview_markdown.py build/desktop/wodex
```

The script copies each fixture into a temporary data directory, uses SDL's dummy
video driver, and writes BMP screenshots under `build/desktop/markdown-previews`.
An optional second argument selects another output directory. Screenshots use
the production renderer and do not send network requests or read user history.
The chat opens at the bottom, matching ordinary conversation behavior.

On macOS, convert a screenshot for preview with:

```sh
sips -s format png build/desktop/markdown-previews/markdown-typography.bmp \
  --out build/desktop/markdown-previews/markdown-typography.png
```

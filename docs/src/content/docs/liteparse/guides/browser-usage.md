---
title: Browser Usage (WASM)
description: Run LiteParse entirely in the browser with the WASM package.
sidebar:
  order: 9
---

LiteParse ships a WebAssembly package that runs entirely in the browser — no server, no cloud calls. It supports PDF parsing and custom OCR engines implemented in JavaScript.

## Install

```bash
npm install @llamaindex/liteparse-wasm
```

## Quick start

```typescript
import init, { LiteParse } from "@llamaindex/liteparse-wasm";

// Load the WASM module
await init();

const parser = new LiteParse({
  ocrEnabled: false,
  outputFormat: "json",
});

// data is a Uint8Array (e.g. from <input type="file"> or fetch)
const bytes = new Uint8Array(await file.arrayBuffer());
const result = await parser.parse(bytes);

console.log(result.text);
console.log(result.pages[0]);
```

## What works in the browser

- **PDF parsing** from `Uint8Array` input (use `file.arrayBuffer()` to get bytes from a file picker for example)
- **Custom OCR** via the `ocrEngine` callback interface (see below)
- **Text, JSON, and markdown output formats**
- **Document complexity** via `parser.isComplex(bytes)` — see the [complexity guide](/liteparse/guides/complexity/)
- **The extraction options** — annotations, form fields, structure trees, vector graphics, and the rest. See [Extraction options](/liteparse/guides/extraction/)

## What doesn't work

- **File path input** — pass `Uint8Array` instead
- **DOCX/XLSX/PPTX conversion** — requires LibreOffice, which isn't available in the browser
- **Built-in Tesseract or HTTP OCR** — use the custom `ocrEngine` interface instead
- **Screenshots** — not available in the WASM build
- **`numWorkers`** — parsing is single-threaded in WASM; the option is not exposed
- **`imageOutputDir`** — there is no filesystem to write to. Use `extractImages` and read the bytes from the result instead

## OCR in the browser

The native Tesseract and HTTP OCR backends are not available in WASM. To use OCR, pass a custom `ocrEngine` object with a `recognize` method:

```typescript
const parser = new LiteParse({
  ocrEnabled: true,
  ocrLanguage: "eng",
  ocrEngine: {
    /**
     * @param imageData PNG-encoded image bytes
     * @param width  rendered page width in pixels
     * @param height rendered page height in pixels
     * @param language e.g. "eng"
     * @returns array of { text, bbox: [x1, y1, x2, y2], confidence }
     */
    async recognize(imageData, width, height, language) {
      // e.g. call a Web Worker wrapping tesseract.js, or a remote OCR service
      return [
        { text: "Hello", bbox: [10, 20, 80, 40], confidence: 0.98 },
      ];
    },
  },
});
```

This lets you plug in any OCR implementation — a Web Worker running tesseract.js, a cloud OCR API, or anything else that returns text with bounding boxes.

## Config options

All optional, camelCase:

| Option | Type | Default | Description |
|---|---|---|---|
| `ocrLanguage` | `string` | `"eng"` | Language code passed to the OCR engine |
| `ocrEnabled` | `boolean` | `false` | Run OCR on text-sparse pages. Off by default in WASM — there is no built-in engine, so this does nothing without `ocrEngine` |
| `ocrEngine` | `object` | — | Custom JS-side OCR engine (see above) |
| `ocrFailureFatal` | `boolean` | `true` | When `false`, OCR failures return partial results instead of throwing |
| `ocrHedgeDelaysMs` | `number[]` | `[]` | Request-hedging schedule for a remote `ocrEngine` |
| `maxPages` | `number` | `1000` | Stop after this many pages |
| `targetPages` | `string` | — | e.g. `"1-5,10,15-20"` |
| `dpi` | `number` | `150` | Render DPI for OCR |
| `outputFormat` | `"json" \| "text" \| "markdown"` | `"json"` | Shape of `result.text`. Also accepts `"md"`. Throws on any other value |
| `preserveVerySmallText` | `boolean` | `false` | Keep tiny text that's normally filtered |
| `skipDiagonalText` | `boolean` | `false` | Drop text more than 2° off the nearest right angle |
| `cropBox` | `{ top, right, bottom, left }` | — | Fraction to crop from each side of every page |
| `password` | `string` | — | Password for protected PDFs |
| `quiet` | `boolean` | `false` | Suppress progress logging |
| `imageMode` | `"off" \| "placeholder" \| "embed"` | `"placeholder"` | How image references appear in markdown. Also accepts `"none"` for `off` |
| `extractLinks` | `boolean` | `true` | Render `[text](url)` in markdown |
| `keepHeadersFooters` | `boolean` | `false` | Keep running header/footer chrome in markdown |
| `emitWordBoxes` | `boolean` | `false` | Per-word sub-boxes on each text item |

The [extraction options](/liteparse/guides/extraction/) — `extractImages`, `extractVectorGraphics`, `extractAnnotations`, `extractFormFields`, `extractStructureTree`, `extractContentBounds`, `extractXfaPackets`, `extractTextMetadata`, `includeComplexity`, and `renderFormFields` — are all available here too, with the same camelCase names and the same `false` defaults.

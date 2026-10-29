// Types for the mica-md wasm package. Mirrors crates/md-core/SCHEMA.md; if a
// kind / mark type / data key is added there and not here, the .d.ts is stale.

/** An inline mark over `Block.text`, offsets in UTF-16 code units, `[start, end)`. */
export interface Mark {
  start: number;
  end: number;
  /** `bold` | `italic` | `strike` | `code` | `link` | `image` | `math` | `html` | `footnote` */
  type: string;
  /** Link target (`link` / `image` marks). May be absent. */
  href?: string;
  /** Link title (`[t](url "title")`). */
  title?: string;
}

/** Clean text plus the marks over it. */
export interface InlineText {
  text: string;
  marks: Mark[];
}

/** One block. `type` is the block kind; `data` carries structure keys. */
export interface Block {
  /**
   * `page` | `paragraph` | `heading` | `bulleted_list` | `numbered_list` |
   * `todo` | `quote` | `code_block` | `math_block` | `image` | `table` |
   * `divider` | `footnote_def`
   */
  type: string;
  text: string;
  data: Record<string, unknown>;
}

/** HTML dialect: `export` is sanitized share HTML; `clipboard` keeps source math. */
export type HtmlProfile = 'export' | 'clipboard';

/** Stored table-cell shapes: inline source, text+marks, or empty. */
export type StoredCell = string | InlineText | null;

/** Load the Node build (automatic on first use). */
export function init(): unknown;

/** Load the Web build (fetches the .wasm). Call once in browsers/bundlers. */
export function initWeb(wasmUrl?: string): Promise<unknown>;

export function parseBody(markdown: string, options?: { fragment?: boolean }): Block[];
export function parseInline(source: string): InlineText;
export function parseInlineBatch(sources: string[]): InlineText[];
export function serializeBody(blocks: Block[], options?: { assets?: Record<string, string> }): string;
export function serializeInline(inline: InlineText): string;
export function renderHtml(
  blocks: Block[],
  options?: { imageSrcs?: Record<string, string>; profile?: HtmlProfile },
): string;
export function decodeCells(cells: StoredCell[]): InlineText[];
export function encodeCells(cells: InlineText[]): StoredCell[];
export function mathRunSpans(source: string): Array<[number, number]>;
export function parseInlineMath(source: string): InlineText;
export function engineInfo(): {
  crate_version: string;
  schema_version: number;
  source_fingerprint: string;
};
import {
  ChunkedSerie,
  Field,
  Serie,
  SerieReader,
  SpillOptions,
  type JoinHow,
  type JoinKeys,
  type JoinOptionsInput,
} from '..'

const trades: Serie = Serie.fromScalars(
  Field.from('trade: struct<id: int64 not null, venue: utf8 not null> not null'),
  [{ id: 1, venue: 'XNAS' }],
)

// The kind is one of DuckDB's words; any spelling the core reads is text.
const inner: JoinHow = 'inner'
const spelled: JoinHow = 'LEFT OUTER JOIN'
// The keys: the text of a key list, key texts and pairs, or a mapping.
const keys: JoinKeys[] = [
  'venue',
  'venue = market',
  ['id', ['venue', 'market']],
  new Map([['venue', 'market']]),
  { venue: 'market' },
]
// The options: each slot absent or null its default.
const options: JoinOptionsInput[] = [
  {},
  { coalesce: true, suffix: '_right', build: null, prune: true, pushdownKeys: 10_000 },
  { build: 'right', spill: new SpillOptions({ byteSize: 0 }) },
  { spill: null, pushdownKeys: null },
]
const held: Serie = trades.joinWith(trades, keys[0], inner, options[1])
const chunked: ChunkedSerie = ChunkedSerie.fromSerie(trades).joinWith(
  ChunkedSerie.fromSerie(trades),
  'id',
  spelled,
)
const streamed: SerieReader = SerieReader.fromSerie(trades).joinWith(trades, 'id', undefined, {
  build: 'left',
})

// @ts-expect-error the build side is `left` or `right`
const middle: JoinOptionsInput = { build: 'middle' }
// @ts-expect-error a suffix is text
const numbered: JoinOptionsInput = { suffix: 1 }
// @ts-expect-error the spill bound is a SpillOptions
const raw: JoinOptionsInput = { spill: { byteSize: 0 } }
// @ts-expect-error a key is text, a pair, or a mapping of terms
trades.joinWith(trades, 7)

void [held, chunked, streamed, middle, numbered, raw]

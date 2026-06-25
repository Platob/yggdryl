// Tests that every value type survives JSON.stringify / fromJSON round-trips.
// Build first with `npm run build`, then run `node --test`.

const { test } = require('node:test')
const assert = require('node:assert')
const { Version, Uri, Url, MimeType, MediaType, Compression } = require('..')

function roundTrip(cls, value) {
  const json = JSON.stringify(value)
  const restored = cls.fromJSON(JSON.parse(json))
  assert.strictEqual(restored.toString(), value.toString())
  return json
}

test('value types round-trip through toJSON / fromJSON', () => {
  assert.strictEqual(roundTrip(Version, new Version(1, 4, 2)), '"1.4.2"')
  roundTrip(Uri, new Uri('https://example.com/docs?page=2#intro'))
  roundTrip(Url, new Url('https://user:pw@example.com:8443/api?v=1#t'))
  assert.strictEqual(roundTrip(MimeType, new MimeType('image/png')), '"image/png"')
  // An unknown but well-formed MIME (Other) round-trips verbatim.
  roundTrip(MimeType, new MimeType('application/x-made-up'))
  assert.strictEqual(
    roundTrip(MediaType, MediaType.fromPath('data.csv.gz')),
    '["text/csv","application/gzip"]'
  )
  assert.strictEqual(roundTrip(Compression, new Compression('gzip')), '"gzip"')
})

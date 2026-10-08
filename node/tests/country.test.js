'use strict'

// `node/src/country.rs`: one ISO 3166-1 alpha-2 country code, read as the
// `country` datatype reads one, redirected to the core.

const assert = require('node:assert/strict')
const test = require('node:test')

const { Country } = require('yggdryl')

test('a country answers the legal tender ISO 4217 list one gives it', () => {
  const us = new Country('US')
  assert.equal(us.toString(), 'US')
  assert.equal(us.toJSON(), 'US')
  assert.ok(us.isListed)
  assert.equal(us.currency, 'USD')
  assert.equal(new Country('GB').currency, 'GBP')
  assert.equal(new Country('CH').currency, 'CHF')
  // The user-assigned code is a value of rank zero, with no currency.
  const masked = new Country('XX')
  assert.ok(!masked.isListed)
  assert.equal(masked.currency, null)
  assert.ok(us.equals(new Country('US')))
  assert.ok(!us.equals(masked))
})

test('a country the datatype refuses throws', () => {
  assert.throws(() => new Country('USA'))
  assert.throws(() => new Country('é'))
})

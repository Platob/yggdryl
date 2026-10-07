import { Country } from '..'

const us: Country = new Country('US')
const listed: boolean = us.isListed
const currency: string | null = us.currency
const same: boolean = us.equals(new Country('US'))
const text: string = us.toString()
const json: string = us.toJSON()

// @ts-expect-error the currency is read-only
us.currency = 'EUR'
// @ts-expect-error a code is text
new Country(1)

void [listed, currency, same, text, json]

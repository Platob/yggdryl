import { Mic } from '..'

const segment: Mic = new Mic('XNGS')
const operating: string | null = segment.operating
const isSegment: boolean = segment.isSegment
const country: string | null = segment.country
const none: boolean = segment.isNone
const same: boolean = segment.equals(new Mic('XNAS'))
const text: string = segment.toString()
const json: string = segment.toJSON()

// @ts-expect-error the operating MIC is read-only
segment.operating = 'XNAS'
// @ts-expect-error a code is text
new Mic(1)

void [operating, isSegment, country, none, same, text, json]

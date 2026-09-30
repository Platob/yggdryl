import {
  Field,
  TimeInForce,
  fix,
  fields,
  timeInForceFixCode,
  timeInForceFromFix,
  type TimeInForceField,
  type TimeInForceName,
} from '..'

const code: number = TimeInForce.GTC
const name: TimeInForceName = 'IOC'
const field: TimeInForceField = fields.timeinforce('tif', { nullable: false })
const read: TimeInForceName = timeInForceFromFix('1')
const wire: string | null = timeInForceFixCode('DAY')
const venue = new Field('venuetif', 'utf8')
venue.fix.timeinforces = [{ wire: 'G', timeinforce: 'GTC' }]
const pairs: Array<{ wire: string; timeinforce: string }> = venue.fix.timeinforces
const mapped: string | null = new fix.FixRegistry().timeinforceOf(59, '1')

// @ts-expect-error a member is read-only
TimeInForce.GTC = 7
// @ts-expect-error no member goes by that name
const missing: number = TimeInForce.NOT_A_TIME

void [code, name, field, read, wire, pairs, mapped, missing]

import { State, fields, type StateField, type StateName } from '..'

const code: number = State.NEW
const name: StateName = 'PARTIALLY_FILLED'
const expired: 9500 = State.EXPIRED
const approved: 8013 = State.APPROVED
const field: StateField = fields.state('state', { nullable: false })

// @ts-expect-error a member is read-only
State.NEW = 7
// @ts-expect-error no member goes by that name
const missing: number = State.NOT_A_STATE

void [code, name, expired, approved, field, missing]

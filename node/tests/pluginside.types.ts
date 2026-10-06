import {
  PluginSide,
  fields,
  fix,
  pluginSideFromPluginType,
  type PluginSideField,
  type PluginSideName,
} from '..'

const code: number = PluginSide.BUYS
const name: PluginSideName = 'SELL'
const field: PluginSideField = fields.pluginside('role', { nullable: false })
const read: PluginSideName = pluginSideFromPluginType('x.SellSideFIXCPluginCBlock')
const registry = new fix.FixRegistry()
const added: boolean = registry.addSource('venue', { pluginside: PluginSide.SELL })

// @ts-expect-error a member is read-only
PluginSide.BUYS = 7
// @ts-expect-error no member goes by that name: the enum is not `Side`
const missing: number = PluginSide.SSHT
// @ts-expect-error a Side member is no plugin side
const sideOnly: PluginSideName = 'SSHT'

void [code, name, field, read, added, missing, sideOnly]

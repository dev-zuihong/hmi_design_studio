import { describe, expect, it } from 'vitest'
import { addQuickAction } from './quick-actions'
import { demoProject } from './model'
import { parseSchemaInput, schemaDefault, schemaDefaults, schemaType } from './schema'

describe('schema configuration values', () => {
  it('preserves boolean, number, object, array and enum JSON types', () => {
    expect(parseSchemaInput('false', { type: 'boolean' })).toBe(false)
    expect(parseSchemaInput('2', { type: 'integer' })).toBe(2)
    expect(parseSchemaInput('{"address": 3}', { type: 'object' })).toEqual({ address: 3 })
    expect(parseSchemaInput('[1, 2]', { type: 'array' })).toEqual([1, 2])
    expect(parseSchemaInput('"fast"', { type: 'string', enum: ['fast', 'slow'] })).toBe('fast')
    expect(parseSchemaInput('null', { type: ['boolean', 'null'] })).toBeNull()
    expect(() => parseSchemaInput('"false"', { type: 'boolean' })).toThrow()
    expect(() => parseSchemaInput('[]', { type: 'object' })).toThrow()
    expect(() => parseSchemaInput('{}', { type: 'array' })).toThrow()
    expect(() => parseSchemaInput('2.5', { type: 'integer' })).toThrow()
    expect(parseSchemaInput('[true, null]', true)).toEqual([true, null])
    expect(() => parseSchemaInput('1', false)).toThrow('不接受任何值')
    expect(schemaType(false)).toBe('json')
    expect(schemaType({ type: ['null'] })).toBe('null')
  })
  it('copies declared defaults without turning optional fields into required ones', () => {
    const schema = { properties: { mode: { enum: ['fast', 'slow'], default: 'fast' }, fixed: { const: 7 }, extra: { type: 'object' } } }
    expect(schemaDefaults(schema)).toEqual({ mode: 'fast', fixed: 7 })
    expect(schemaDefault({ type: 'array', default: [1] })).toEqual([1])
  })
})

describe('quick actions', () => {
  it('creates one trigger and action and replaces the event binding without duplicating execution', () => {
    const project = demoProject(), button = project.pages[0]!.widgets.find(w => w.kind === 'button')!
    const id = addQuickAction(project, button.id, { kind: 'connection.close', connectionId: 'sensor' })
    const flow = project.flows.find(flow => flow.id === id)!
    expect(flow.nodes.map(node => node.kind)).toEqual(['trigger.ui', 'connection.close'])
    expect(flow.edges).toHaveLength(1)
    expect(button.events.click).toEqual({ kind: 'flow', flowId: id, entryNodeId: flow.nodes[0]!.id })
    expect(Object.keys(button.events)).toEqual(['click'])
  })
  it('keeps command input references and rejects missing resources before mutation', () => {
    const project = demoProject(), button = project.pages[0]!.widgets.find(w => w.kind === 'button')!
    const params = { target: { source: 'input' as const, key: 'target_input' } }
    const id = addQuickAction(project, button.id, { kind: 'protocol.command', connectionId: 'sensor', command: 'setTarget', params })
    expect(project.flows.find(flow => flow.id === id)!.nodes[1]!.config.params).toEqual(params)
    params.target.key = 'changed'
    expect(project.flows.find(flow => flow.id === id)!.nodes[1]!.config.params.target.key).toBe('target_input')
    const count = project.flows.length
    expect(() => addQuickAction(project, button.id, { kind: 'connection.open', connectionId: 'missing' })).toThrow()
    expect(project.flows).toHaveLength(count)
  })
})

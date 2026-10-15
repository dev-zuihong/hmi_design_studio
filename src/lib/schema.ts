export type Schema = Record<string, any>

export function schemaType(schema: Schema | boolean): string {
  if (typeof schema === 'boolean') return 'json'
  return (Array.isArray(schema.type) ? schema.type.find((type: string) => type !== 'null') ?? schema.type[0] : schema.type) ?? 'string'
}

export function schemaDefault(schema: Schema | boolean): unknown {
  if (typeof schema === 'boolean') return null
  if (schema.default !== undefined) return JSON.parse(JSON.stringify(schema.default))
  if (schema.const !== undefined) return JSON.parse(JSON.stringify(schema.const))
  if (schema.enum?.length) return JSON.parse(JSON.stringify(schema.enum[0]))
  switch (schemaType(schema)) {
    case 'number': case 'integer': return 0
    case 'boolean': return false
    case 'object': return {}
    case 'array': return []
    case 'null': return null
    default: return ''
  }
}

export function parseSchemaInput(text: string, schema: Schema | boolean): unknown {
  if (schema === false) throw new Error('该参数的协议约束不接受任何值')
  if (schema === true) return JSON.parse(text)
  const type = schemaType(schema)
  if (type === 'string' && !schema.enum && schema.const === undefined) return text
  const value = JSON.parse(text)
  if (schema.enum && !schema.enum.some((item: unknown) => JSON.stringify(item) === JSON.stringify(value))) throw new Error('请选择声明的选项')
  if (schema.const !== undefined && JSON.stringify(value) !== JSON.stringify(schema.const)) throw new Error('该参数必须使用声明的固定值')
  const types = Array.isArray(schema.type) ? schema.type : [schema.type]
  if (value === null && (types.includes('null') || schema.enum?.includes(null))) return value
  if ((type === 'number' || type === 'integer') && (typeof value !== 'number' || !Number.isFinite(value) || type === 'integer' && !Number.isInteger(value))) throw new Error(type === 'integer' ? '请输入整数' : '请输入有限数字')
  if (type === 'boolean' && typeof value !== 'boolean') throw new Error('请选择 true 或 false')
  if (type === 'object' && (!value || typeof value !== 'object' || Array.isArray(value))) throw new Error('请输入 JSON 对象，例如 {"address": 1}')
  if (type === 'array' && !Array.isArray(value)) throw new Error('请输入 JSON 数组，例如 [1, 2]')
  if (type === 'null' && value !== null) throw new Error('该参数必须为 null')
  return value
}

export function schemaDefaults(schema: Schema | undefined): Record<string, unknown> {
  return Object.fromEntries(Object.entries(schema?.properties ?? {}).filter(([, field]) => typeof field === 'object' && field !== null && ((field as Schema).default !== undefined || (field as Schema).const !== undefined)).map(([key, field]) => [key, schemaDefault(field as Schema)]))
}

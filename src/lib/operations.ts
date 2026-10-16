import { clone } from './model'

export interface WorkspaceOperation { path: string[]; beforeExists: boolean; before: unknown; afterExists: boolean; after: unknown; index?: number }
const equal = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b)
const keyed = (value: unknown): value is { id: string }[] => Array.isArray(value) && value.every(item => item && typeof item === 'object' && typeof item.id === 'string')

/** Paths address entities by stable ID. Array indexes never identify editable objects. */
export function diffDocument(before: any, after: any, path: string[] = []): WorkspaceOperation[] {
  if (!path.length) { before = clone(before); after = clone(after) }
  if (equal(before, after)) return []
  if (keyed(before) && keyed(after)) {
    const operations: WorkspaceOperation[] = []
    for (const item of before) if (!after.some(next => next.id === item.id)) operations.push({ path: [...path, item.id], beforeExists: true, before: clone(item), afterExists: false, after: null })
    after.forEach((item, index) => {
      const previous = before.find(candidate => candidate.id === item.id)
      if (previous) operations.push(...diffDocument(previous, item, [...path, item.id]))
      else operations.push({ path: [...path, item.id], beforeExists: false, before: null, afterExists: true, after: clone(item), index })
    })
    const stagedOrder = before.filter(item => after.some(next => next.id === item.id)).map(item => item.id)
    after.forEach((item, index) => { if (!before.some(previous => previous.id === item.id)) stagedOrder.splice(Math.min(index, stagedOrder.length), 0, item.id) })
    const finalOrder = after.map(item => item.id)
    if (!equal(stagedOrder, finalOrder)) operations.push({ path: [...path, '$order'], beforeExists: true, before: stagedOrder, afterExists: true, after: finalOrder })
    return operations
  }
  if (before && after && typeof before === 'object' && typeof after === 'object' && !Array.isArray(before) && !Array.isArray(after)) {
    return [...new Set([...Object.keys(before), ...Object.keys(after)])].flatMap(key => {
      if (!(key in before) || !(key in after)) return [{ path: [...path, key], beforeExists: key in before, before: key in before ? clone(before[key]) : null, afterExists: key in after, after: key in after ? clone(after[key]) : null }]
      return diffDocument(before[key], after[key], [...path, key])
    })
  }
  return [{ path, beforeExists: before !== undefined, before: before === undefined ? null : clone(before), afterExists: after !== undefined, after: after === undefined ? null : clone(after) }]
}

function location(root: any, path: string[]) {
  let parent = root
  for (const segment of path.slice(0, -1)) {
    parent = Array.isArray(parent) ? parent.find(item => item.id === segment) : parent?.[segment]
    if (parent === undefined || parent === null) throw new Error(`对象已变更：${path.join('.')}`)
  }
  const key = path.at(-1)!
  const index = Array.isArray(parent) ? parent.findIndex(item => item.id === key) : -1
  const exists = key === '$order' && Array.isArray(parent) || (Array.isArray(parent) ? index >= 0 : Object.prototype.hasOwnProperty.call(parent, key))
  const value = key === '$order' && Array.isArray(parent) ? parent.map(item => item.id) : Array.isArray(parent) ? parent[index] : parent[key]
  return { parent, key, index, exists, value }
}
export function applyOperations<T>(document: T, operations: WorkspaceOperation[], verify = true): T {
  let result = clone(document)
  for (const operation of operations) {
    if (!operation.path.length) { if (verify && !equal(result, operation.before)) throw new Error('工程已被另一窗口修改'); result = clone(operation.after) as T; continue }
    const { parent, key, index, exists, value } = location(result, operation.path)
    if (verify && (exists !== operation.beforeExists || exists && !equal(value, operation.before))) throw new Error(`操作冲突：${operation.path.join('.')}`)
    if (key === '$order' && Array.isArray(parent)) {
      const order = operation.after as string[]
      const sorted = order.map(id => parent.find(item => item.id === id)).filter(Boolean)
      sorted.push(...parent.filter(item => !order.includes(item.id))); parent.splice(0, parent.length, ...sorted)
    } else if (Array.isArray(parent)) {
      if (!operation.afterExists) { if (index >= 0) parent.splice(index, 1) }
      else if (index >= 0) parent[index] = clone(operation.after)
      else parent.splice(Math.min(operation.index ?? parent.length, parent.length), 0, clone(operation.after))
    } else if (operation.afterExists) parent[key] = clone(operation.after)
    else delete parent[key]
  }
  return result
}
export function invertOperations(operations: WorkspaceOperation[]): WorkspaceOperation[] {
  return [...operations].reverse().map(operation => ({ ...operation, beforeExists: operation.afterExists, before: operation.after, afterExists: operation.beforeExists, after: operation.before }))
}

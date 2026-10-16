import { expect, it } from 'vitest'
import { matchesEditorSearch } from './editor-search'

it('finds a field by its original label, translated label or stable key', () => {
  const translations: Record<string, string> = { '最小宽度': 'Minimum width', '尺寸约束': 'Size constraints' }
  const translate = (text: string) => translations[text] ?? text
  const terms = ['尺寸约束', '最小宽度', 'minWidth']
  for (const query of ['最小宽度', ' Minimum WIDTH ', 'minWidth', 'Size constraints', '']) {
    expect(matchesEditorSearch(query, terms, translate)).toBe(true)
  }
  expect(matchesEditorSearch('height', terms, translate)).toBe(false)
})

it('matches user-defined property names and node kinds as literal text', () => {
  expect(matchesEditorSearch('temperature[0]', ['temperature[0]'])).toBe(true)
  expect(matchesEditorSearch('protocol.command', ['协议命令', 'protocol.command'])).toBe(true)
  expect(matchesEditorSearch('[', ['unrelated'])).toBe(false)
})

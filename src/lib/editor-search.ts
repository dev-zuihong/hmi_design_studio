/** Match authored labels, localized labels and stable keys without changing project data. */
export function matchesEditorSearch(query: string, terms: readonly string[], translate: (text: string) => string = text => text): boolean {
  const normalized = query.trim().toLocaleLowerCase()
  return !normalized || terms.some(term => `${term} ${translate(term)}`.toLocaleLowerCase().includes(normalized))
}

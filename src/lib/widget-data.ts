export interface ByteRange { startByte: number; byteCount: number }
export function validByteRange(value: unknown, length: number, allowEmpty = false): value is ByteRange {
  if (!Number.isInteger(length) || length < 1 || length > 32 || !value || typeof value !== 'object') return false
  const { startByte, byteCount } = value as ByteRange
  return Number.isInteger(startByte) && Number.isInteger(byteCount) && startByte >= 0 && byteCount >= (allowEmpty ? 0 : 1) && startByte + byteCount <= length
}
/** Serializes an unsigned selection mask without JavaScript's signed 32-bit shifts. */
export function byteRangeMask(value: ByteRange, length: number): string | undefined {
  if (!validByteRange(value, length, true)) return undefined
  return (((1n << BigInt(value.byteCount)) - 1n) << BigInt(value.startByte)).toString(16).toUpperCase().padStart(Math.ceil(length / 4), '0')
}
export interface MemorySegment { id: string; label?: string; start: number; size: number; color?: string; enabled?: boolean }
export interface MappedSegment extends MemorySegment { valid: boolean; overlap: boolean; outOfRange: boolean; offset: number; fraction: number }
export function memorySegments(start: number, size: number, source: MemorySegment[]): MappedSegment[] {
  const validRange = Number.isSafeInteger(start) && start >= 0 && Number.isSafeInteger(size) && size > 0 && Number.isSafeInteger(start + size)
  const segments = source.filter(item => item && item.enabled !== false).map(item => {
    const valid = Number.isSafeInteger(item.start) && item.start >= 0 && Number.isSafeInteger(item.size) && item.size > 0 && Number.isSafeInteger(item.start + item.size)
    const left = validRange && valid ? Math.max(start, Math.min(start + size, item.start)) : start
    const right = validRange && valid ? Math.max(start, Math.min(start + size, item.start + item.size)) : start
    return { ...item, valid, overlap: false, outOfRange: !validRange || !valid || item.start < start || item.start + item.size > start + size, offset: validRange ? (left - start) / size : 0, fraction: validRange ? Math.max(0, right - left) / size : 0 }
  })
  for (let i = 0; i < segments.length; i++) for (let j = i + 1; j < segments.length; j++) {
    const a = segments[i]!, b = segments[j]!
    if (a.valid && b.valid && a.start < b.start + b.size && b.start < a.start + a.size) a.overlap = b.overlap = true
  }
  return segments
}
export function tableRowKey(row: Record<string, unknown>, field: string, index: number): string | number {
  const value = row[field || 'id'] ?? row.id ?? row.key
  return typeof value === 'string' || typeof value === 'number' ? value : index
}
export function rowCondition(row: Record<string, unknown>, field: unknown, defaultValue = true) { return typeof field === 'string' && field ? Boolean(row[field]) : defaultValue }
export function validateText(value: string, options: { format?: string; pattern?: string; maxLength?: number; required?: boolean }): string | undefined {
  if (options.required && !value.trim()) return '请输入值'
  if (options.maxLength && value.length > options.maxLength) return '超过最大长度'
  if (!value) return undefined
  if (options.format === 'hex' && !/^(?:[\da-fA-F]{2})+$/.test(value.replace(/\s/g, ''))) return '请输入完整的十六进制字节'
  if (options.format === 'address' && (!/^(?:0[xX][\da-fA-F]{1,8}|\d{1,10})$/.test(value.trim()) || Number(value.trim()) > 0xffffffff)) return '请输入 0–4294967295 的十进制地址或 0x 开头的十六进制地址'
  if (options.pattern) { try { if (!new RegExp(`^(?:${options.pattern})$`).test(value)) return '输入格式不匹配' } catch { return '校验规则无效' } }
  return undefined
}

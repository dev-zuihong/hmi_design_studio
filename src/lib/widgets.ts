import type { CSSProperties } from 'vue'
import type { LayoutItem, LayoutSpec, Page, Widget } from './model'
import { widgetKinds } from './model'

export const inputWidgetKinds = ['input','textarea','switch','select','checkbox','radio','spinbox','slider','date','time','datetime','table','tabs','stack','file-picker','byte-range']
/** Context substitutions are path segments, so device keys containing dots stay intact. */
export function resolveRuntimePath(value: unknown, path = '', context: Record<string, unknown> = {}): unknown {
  if (!path) return value
  let result: any = value
  for (const segment of path.split('.')) {
    const key = segment.startsWith('$') ? context[segment.slice(1)] : segment
    if ((typeof key !== 'string' && typeof key !== 'number') || ['__proto__','prototype','constructor'].includes(String(key)) || result === null || typeof result !== 'object' || !Object.prototype.hasOwnProperty.call(result, key)) return undefined
    result = result[key]
  }
  return result
}

export function getRootWidgets(page: Page): Widget[] {
  const ids = new Set(page.widgets.map(widget => widget.id))
  return page.widgets.filter(widget => !widget.parentId || !ids.has(widget.parentId))
}
export function getWidgetChildren(page: Page, parentId: string, slotId?: string): Widget[] {
  return page.widgets.filter(widget => widget.parentId === parentId && (slotId === undefined || (widget.slotId ?? 'content') === slotId))
}
export function containerChildren(page: Page, parentId: string): Map<string, Widget[]> {
  const result = new Map<string, Widget[]>()
  for (const widget of getWidgetChildren(page, parentId)) {
    const slot = widget.slotId ?? 'content'
    const list = result.get(slot) ?? []
    list.push(widget)
    result.set(slot, list)
  }
  return result
}
export function isContainer(kindOrWidget: string | Widget): boolean {
  const kind = typeof kindOrWidget === 'string' ? kindOrWidget : kindOrWidget.kind
  return widgetKinds.find(item => item.kind === kind)?.container === true
}
export function layoutStyle(layout?: LayoutSpec): CSSProperties {
  const type = layout?.type ?? 'absolute'
  const style: CSSProperties = { position: 'relative', boxSizing: 'border-box', gap: `${Math.max(0, layout?.gap ?? 8)}px`, padding: `${Math.max(0, layout?.padding ?? 8)}px`, minWidth: 0, minHeight: 0 }
  if (type === 'absolute') return { ...style, display: 'block', padding: 0, overflow: 'visible' }
  if (type === 'horizontal') return { ...style, display: 'flex', flexDirection: 'row', alignItems: 'stretch' }
  if (type === 'vertical') return { ...style, display: 'flex', flexDirection: 'column', alignItems: 'stretch' }
  if (type === 'form') return { ...style, display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 2fr)', gridAutoRows: 'minmax(0, auto)', alignItems: 'center' }
  return { ...style, display: 'grid', gridTemplateColumns: `repeat(${Math.max(1, layout?.columns ?? 2)}, minmax(0, 1fr))`, gridAutoRows: 'minmax(0, auto)' }
}
export function layoutItemStyle(item?: LayoutItem, parentLayout?: LayoutSpec): CSSProperties {
  const style: CSSProperties = { boxSizing: 'border-box', minWidth: item?.minWidth != null ? `${item.minWidth}px` : '0', minHeight: item?.minHeight != null ? `${item.minHeight}px` : '0' }
  if (item?.maxWidth != null) style.maxWidth = `${item.maxWidth}px`
  if (item?.maxHeight != null) style.maxHeight = `${item.maxHeight}px`
  if (parentLayout?.type === 'absolute' || !parentLayout) {
    style.position = 'absolute'; style.left = '0px'; style.top = '0px'
    return style
  }
  if (item?.grow != null) style.flexGrow = Math.max(0, item.grow)
  if (item?.align) style.alignSelf = item.align
  if (parentLayout.type === 'grid') {
    if (item?.row != null) style.gridRow = `${item.row + 1} / span ${Math.max(1, item.rowSpan ?? 1)}`
    if (item?.column != null) style.gridColumn = `${item.column + 1} / span ${Math.max(1, item.columnSpan ?? 1)}`
  }
  return style
}
export function widgetBoxStyle(widget: Widget, parentLayout?: LayoutSpec): CSSProperties {
  if (!parentLayout || parentLayout.type === 'absolute') return { position: 'absolute', left: `${widget.x}px`, top: `${widget.y}px`, width: `${widget.width}px`, height: `${widget.height}px`, zIndex: widget.zIndex, boxSizing: 'border-box' }
  return { ...layoutItemStyle(widget.layoutItem, parentLayout), width: widget.layoutItem?.grow ? undefined : `${widget.width}px`, height: `${widget.height}px`, zIndex: widget.zIndex }
}
export function hasWidgetChildren(page: Page, widget: Widget): boolean { return isContainer(widget) && page.widgets.some(item => item.parentId === widget.id) }
export function findParentLayout(page: Page, widget: Widget): LayoutSpec | undefined { return page.widgets.find(item => item.id === widget.parentId)?.layout ?? page.layout }
export function widgetDefault(kind: string): Widget | undefined { return widgetKinds.find(item => item.kind === kind) as unknown as Widget | undefined }

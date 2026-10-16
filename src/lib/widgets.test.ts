import { describe, expect, it } from 'vitest'
import { createWidget, emptyProject, widgetKinds } from './model'
import { containerChildren, getRootWidgets, getWidgetChildren, isContainer, layoutItemStyle, layoutStyle, widgetBoxStyle } from './widgets'

describe('shared widget catalog and recursive layouts', () => {
  it('keeps the requested common Qt-style kinds in a single catalog', () => {
    expect(widgetKinds.map(item => item.kind)).toEqual([
      'text','button','tool-button','input','textarea','checkbox','radio','select','switch','spinbox','slider','date','time','datetime','image','number','indicator','progress','chart','log','list','tree','table','group','frame','scroll','tabs','stack','splitter','dialog','menu','toolbar','statusbar','separator','spacer',
    ])
    expect(new Set(widgetKinds.map(item => item.kind)).size).toBe(widgetKinds.length)
  })
  it('creates stable container slots and a responsive v2 page by default', () => {
    const project=emptyProject(), tabs=createWidget('tabs')
    expect(project.schemaVersion).toBe(2)
    expect(project.pages[0]?.responsive).toBe(true)
    expect(tabs.slots?.map(slot=>slot.id)).toEqual(['tab_1','tab_2'])
    expect(tabs.layout?.type).toBe('absolute')
  })
  it('groups nested children by stable parent and slot ids', () => {
    const project=emptyProject(), page=project.pages[0]!, container=createWidget('tabs'), first=createWidget('button'), second=createWidget('text')
    first.parentId=container.id; first.slotId='tab_1'; second.parentId=container.id; second.slotId='tab_2'
    page.widgets=[container,first,second]
    expect(isContainer(container)).toBe(true)
    expect(getRootWidgets(page)).toEqual([container])
    expect(getWidgetChildren(page,container.id,'tab_1')).toEqual([first])
    expect([...containerChildren(page,container.id).keys()]).toEqual(['tab_1','tab_2'])
  })
  it('provides absolute, grid, and flex sizing styles shared by editor and runtime', () => {
    const widget=createWidget('button',32,48); const box=widgetBoxStyle(widget,{type:'absolute'})
    expect(box.left).toBe('32px'); expect(box.top).toBe('48px')
    expect(layoutStyle({type:'grid',columns:3,gap:12}).gridTemplateColumns).toBe('repeat(3, minmax(0, 1fr))')
    expect(layoutItemStyle({grow:2,row:1,column:2,rowSpan:2,columnSpan:3},{type:'grid'})).toMatchObject({flexGrow:2,gridRow:'2 / span 2',gridColumn:'3 / span 3'})
  })
})

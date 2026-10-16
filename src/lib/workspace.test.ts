import { describe,it,expect } from 'vitest'
import { MemoryWorkspace } from './workspace'
import { applyOperations,diffDocument } from './operations'
import { clone,createWidget,emptyProject,createNode } from './model'
describe('shared workspace with independent histories',()=>{
  it('merges separate fields by stable IDs and selectively undoes only the originating window',()=>{
    const project=emptyProject(),widget=createWidget('button',80,90);project.pages[0]!.widgets.push(widget);const store=new MemoryWorkspace(project)
    const a=clone(store.project);a.pages[0]!.widgets[0]!.x=300;store.apply('main',diffDocument(store.project,a))
    const b=clone(store.project);b.name='Logic changed name';store.apply('logic',diffDocument(store.project,b))
    store.undo('main');expect(store.project.pages[0]!.widgets[0]!.x).toBe(80);expect(store.project.name).toBe('Logic changed name')
    store.redo('main');expect(store.project.pages[0]!.widgets[0]!.x).toBe(300)
  })
  it('rejects an overlapping undo atomically and retains both histories',()=>{
    const store=new MemoryWorkspace(emptyProject());const first=clone(store.project);first.name='First';store.apply('main',diffDocument(store.project,first));const second=clone(store.project);second.name='Second';store.apply('logic',diffDocument(store.project,second));expect(()=>store.undo('main')).toThrow('冲突');expect(store.project.name).toBe('Second');expect(store.history('main').past).toHaveLength(1)
  })
  it('keeps redo after an unrelated edit in another window and becomes clean when undo restores saved content',()=>{
    const store=new MemoryWorkspace(emptyProject()),a=clone(store.project);a.name='Changed';store.apply('main',diffDocument(store.project,a));store.undo('main');expect(store.state().dirty).toBe(false);const b=clone(store.project);b.width=800;store.apply('logic',diffDocument(store.project,b));expect(store.history('main').future).toHaveLength(1);store.redo('main');expect(store.project.name).toBe('Changed');expect(store.project.width).toBe(800)
  })
  it('does not let undo remove an entity newly referenced by another window',()=>{
    const store=new MemoryWorkspace(emptyProject()),withWidget=clone(store.project),widget=createWidget('input',100,100);withWidget.pages[0]!.widgets.push(widget);store.apply('main',diffDocument(store.project,withWidget));const withFlow=clone(store.project),node=createNode('assign');node.config.value={source:'input',key:widget.id};withFlow.flows.push({id:'flow',name:'flow',enabled:true,timeoutMs:30000,nodes:[node],edges:[]});store.apply('logic',diffDocument(store.project,withFlow));expect(()=>store.undo('main')).toThrow('引用');expect(store.project.pages[0]!.widgets).toHaveLength(1)
  })
  it('saving a previous revision does not mark newer changes as saved',()=>{
    const store=new MemoryWorkspace(emptyProject()),saved=clone(store.project);const next=clone(saved);next.name='While saving';store.apply('logic',diffDocument(saved,next));store.markSaved(0,saved);expect(store.state().dirty).toBe(true)
  })
  it('represents insert, reorder and delete with stable entity paths',()=>{
    const before=emptyProject(),a=createWidget('text'),b=createWidget('button'),c=createWidget('input');before.pages[0]!.widgets=[a,b];const after=clone(before);after.pages[0]!.widgets=[b,c,a];const operations=diffDocument(before,after);expect(operations.every(operation=>!operation.path.some(segment=>/^\d+$/.test(segment)))).toBe(true);expect(applyOperations(before,operations)).toEqual(after)
  })
})

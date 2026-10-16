import { describe,it,expect,vi } from 'vitest'
import { emptyProject } from './model'
const mocks=vi.hoisted(()=>({call:vi.fn()}))
vi.mock('./backend',()=>({desktop:true,call:mocks.call}))
vi.mock('@tauri-apps/api/core',()=>({convertFileSrc:(path:string)=>`asset://${path}`}))
import { refreshAssets,assetUrls } from './resources'
describe('preview asset refresh',()=>{
 it('keeps the new revision assets when an older resolve finishes later',async()=>{
  let finishOld!:(paths:Record<string,string>)=>void
  mocks.call.mockImplementationOnce(()=>new Promise(resolve=>finishOld=resolve)).mockResolvedValueOnce({image:'/new/image.png'})
  const project=emptyProject(),old=refreshAssets(project);await refreshAssets(project)
  expect(assetUrls.value).toEqual({image:'asset:///new/image.png'})
  finishOld({image:'/old/image.png'});await old;expect(assetUrls.value).toEqual({image:'asset:///new/image.png'})
 })
})

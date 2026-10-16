import { describe,it,expect } from 'vitest'
import { BrowserProjectPackage } from './project-package'
import { emptyProject } from './model'
import { ZipReader,ZipWriter,Uint8ArrayReader,Uint8ArrayWriter } from '@zip.js/zip.js'
async function modifyArchive(blob:Blob,change:(manifest:any)=>void,directory=false){
 const reader=new ZipReader(new Uint8ArrayReader(new Uint8Array(await blob.arrayBuffer()))),writer=new ZipWriter(new Uint8ArrayWriter())
 for(const entry of await reader.getEntries()){if(entry.directory)continue;let bytes=await entry.getData!(new Uint8ArrayWriter());if(entry.filename==='manifest.json'){const manifest=JSON.parse(new TextDecoder().decode(bytes));change(manifest);bytes=new TextEncoder().encode(JSON.stringify(manifest))}await writer.add(entry.filename,new Uint8ArrayReader(bytes))}
 await reader.close();if(directory)await writer.add('unlisted/',undefined,{directory:true});return new File([new Uint8Array(await writer.close()).buffer],'changed.hmiproj')
}
describe('portable editable project archives',()=>{
  it('roundtrips source, translations, assets and opaque foreign platform dependencies',async()=>{
    const project=emptyProject(),archive=new BrowserProjectPackage();archive.projectId=project.id;project.assets=[{id:'picture',path:'assets/picture.png'}];project.i18n!.messages['en-US']={title:'Hello'};archive.files.set('assets/picture.png',new Uint8Array([1,2,3]));archive.files.set('plugins/x86_64-pc-windows-msvc/example/plugin.exe',new Uint8Array([77,90]));archive.metadata['plugins/x86_64-pc-windows-msvc/example/plugin.exe']={sha256:'',size:2,mode:'executable',unixMode:0o755};const blob=await archive.save(project),reader=new BrowserProjectPackage(),restored=await reader.open(new File([blob],'example.hmiproj'));expect(restored).toEqual(project);expect(reader.files.get('assets/picture.png')).toEqual(new Uint8Array([1,2,3]));expect(reader.metadata['plugins/x86_64-pc-windows-msvc/example/plugin.exe']?.unixMode).toBe(0o755);expect(reader.files.has('i18n/en-US.json')).toBe(true)
  })
  it('fails saving a resource reference that cannot be included',async()=>{const project=emptyProject();project.assets=[{id:'missing',path:'assets/missing.png'}];await expect(new BrowserProjectPackage().save(project)).rejects.toThrow('资源缺失')})
  it('rejects resource paths that would be unsafe or unusable on the supported platforms',async()=>{
    for(const path of ['../asset.png','C:/asset.png','assets\\asset.png','assets/NUL.txt','assets/trailing.','assets/question?.png']){const project=emptyProject(),archive=new BrowserProjectPackage();archive.projectId=project.id;archive.files.set(path,new Uint8Array([1]));await expect(archive.save(project)).rejects.toThrow('路径无效')}
  })
  it('rejects invalid permissions, mismatched schema and unlisted directory entries',async()=>{
    const blob=await new BrowserProjectPackage().save(emptyProject())
    for(const change of [(manifest:any)=>manifest.files['project.json'].mode='symlink',(manifest:any)=>manifest.files['project.json'].unixMode=0o4755])await expect(new BrowserProjectPackage().open(await modifyArchive(blob,change))).rejects.toThrow('权限声明无效')
    await expect(new BrowserProjectPackage().open(await modifyArchive(blob,manifest=>manifest.projectSchemaVersion=1))).rejects.toThrow('格式版本与清单不一致')
    await expect(new BrowserProjectPackage().open(await modifyArchive(blob,()=>{},true))).rejects.toThrow('不安全')
  })
  it('retains the current resource state when a legacy JSON import fails',async()=>{
    const archive=new BrowserProjectPackage(),project=emptyProject();archive.projectId=project.id;archive.files.set('assets/image.png',new Uint8Array([1,2]))
    await expect(archive.open(new File(['broken json'],'broken.json'))).rejects.toThrow();expect(archive.projectId).toBe(project.id);expect(archive.files.get('assets/image.png')).toEqual(new Uint8Array([1,2]))
  })
  it('refreshes external language tables and records dependencies that cannot be included',async()=>{
    const project=emptyProject(),archive=new BrowserProjectPackage();archive.projectId=project.id;archive.files.set('i18n/de-DE.json',new TextEncoder().encode('{}'));project.i18n!.messages['en-US']={title:'Updated'};project.connections.push({id:'c',name:'Connection',port:'sim://temperature',baudRate:9600,dataBits:8,stopBits:1,parity:'none',protocolId:'missing-protocol',protocolConfig:{}})
    const blob=await archive.save(project),restored=new BrowserProjectPackage();await restored.open(new File([blob],'messages.hmiproj'));expect(restored.files.has('i18n/de-DE.json')).toBe(false);expect(JSON.parse(new TextDecoder().decode(restored.files.get('i18n/en-US.json')))).toEqual({title:'Updated'});expect(restored.missingDependencies).toEqual(['plugin:missing-protocol'])
  })
})

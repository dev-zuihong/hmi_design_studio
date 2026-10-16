import { Uint8ArrayReader, Uint8ArrayWriter, ZipReader, ZipWriter } from '@zip.js/zip.js'
import { parseProject } from './editor'
import type { Project } from './model'

export interface PackageFile { sha256: string; size: number; mode: 'file'|'executable'; unixMode?: number }
export interface PackageManifest { format:'hmi-project';formatVersion:1;projectSchemaVersion:number;files:Record<string,PackageFile>;missingDependencies:string[] }
const encoder=new TextEncoder(),decoder=new TextDecoder()
const safePath=(path:string)=>!!path&&!path.startsWith('/')&&!path.includes('\\')&&!path.includes(':')&&path.split('/').every(part=>part&&part!=='.'&&part!=='..'&&!/[\u0000-\u001f\u007f-\u009f<>"|?*]/.test(part)&&!/[ .]$/.test(part)&&!/^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$/i.test(part.split('.')[0]!))
async function digest(bytes:Uint8Array){return [...new Uint8Array(await crypto.subtle.digest('SHA-256',new Uint8Array(bytes).buffer))].map(byte=>byte.toString(16).padStart(2,'0')).join('')}
export class BrowserProjectPackage {
  files=new Map<string,Uint8Array>();metadata:Record<string,PackageFile>={};missingDependencies:string[]=[]
  projectId=''
  private async database(){if(typeof indexedDB==='undefined')return;return new Promise<IDBDatabase>((resolve,reject)=>{const request=indexedDB.open('hmi-project-resources-v2',1);request.onupgradeneeded=()=>request.result.createObjectStore('packages');request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error)})}
  async persistResources(){const database=await this.database();if(!database||!this.projectId)return;await new Promise<void>((resolve,reject)=>{const transaction=database.transaction('packages','readwrite');transaction.objectStore('packages').put({files:[...this.files],metadata:this.metadata,missing:this.missingDependencies},this.projectId);transaction.oncomplete=()=>resolve();transaction.onerror=()=>reject(transaction.error)});database.close()}
  async ensureResources(project:Project){if(this.projectId===project.id)return;const database=await this.database();let saved:any;if(database){saved=await new Promise((resolve,reject)=>{const transaction=database.transaction('packages','readonly'),request=transaction.objectStore('packages').get(project.id);request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error)});database.close()}this.projectId=project.id;this.files=new Map(saved?.files??[]);this.metadata=saved?.metadata??{};this.missingDependencies=saved?.missing??[]}
  async open(file:File):Promise<Project>{
    if(file.name.toLowerCase().endsWith('.json')){const project=parseProject(await file.text());this.files.clear();this.metadata={};this.missingDependencies=[];this.projectId=project.id;await this.persistResources();return project}
    const reader=new ZipReader(new Uint8ArrayReader(new Uint8Array(await file.arrayBuffer())))
    const loaded=new Map<string,Uint8Array>();let total=0
    try{
      const entries=await reader.getEntries();if(entries.length>100000)throw Error('工程包文件数量超限')
      for(const entry of entries){if(entry.directory||entry.symlink||!safePath(entry.filename)||loaded.has(entry.filename))throw Error('工程包包含不安全或重复的路径');if(entry.uncompressedSize>1024**3||(entry.filename==='manifest.json'&&entry.uncompressedSize>16*1024**2))throw Error('工程包文件大小超限');total+=entry.uncompressedSize;if(total>4*1024**3)throw Error('工程包解压大小超限');loaded.set(entry.filename,await entry.getData!(new Uint8ArrayWriter()))}
    }finally{await reader.close()}
    const manifest=JSON.parse(decoder.decode(loaded.get('manifest.json'))) as PackageManifest
    if(!manifest||manifest.format!=='hmi-project'||manifest.formatVersion!==1||!manifest.files||typeof manifest.files!=='object'||Array.isArray(manifest.files))throw Error('不支持的工程包格式')
    if(manifest.missingDependencies!==undefined&&(!Array.isArray(manifest.missingDependencies)||manifest.missingDependencies.some(value=>typeof value!=='string')))throw Error('工程包依赖声明无效')
    if(loaded.size!==Object.keys(manifest.files).length+1)throw Error('工程包文件集合与清单不一致')
    for(const [path,descriptor]of Object.entries(manifest.files)){if(!descriptor||!['file','executable'].includes(descriptor.mode)||descriptor.unixMode!==undefined&&(!Number.isInteger(descriptor.unixMode)||descriptor.unixMode<0||descriptor.unixMode>0o777))throw Error(`工程包权限声明无效：${path}`);const bytes=loaded.get(path);if(!safePath(path)||path==='manifest.json'||!bytes||bytes.length!==descriptor.size||await digest(bytes)!==descriptor.sha256)throw Error(`工程包资源校验失败：${path}`)}
    const source=decoder.decode(loaded.get('project.json'))
    if(JSON.parse(source).schemaVersion!==manifest.projectSchemaVersion)throw Error('工程格式版本与清单不一致')
    const project=parseProject(source)
    loaded.delete('manifest.json');this.files=loaded;this.metadata=manifest.files;this.missingDependencies=manifest.missingDependencies??[];this.projectId=project.id;await this.persistResources();return project
  }
  async save(project:Project):Promise<Blob>{
    await this.ensureResources(project)
    this.files.set('project.json',encoder.encode(JSON.stringify(project,null,2)))
    for(const path of this.files.keys())if(/^i18n\/[^/]+\.json$/.test(path))this.files.delete(path)
    for(const [locale,messages]of Object.entries(project.i18n?.messages??{}))this.files.set(`i18n/${locale}.json`,encoder.encode(JSON.stringify(messages,null,2)))
    for(const asset of project.assets??[])if(!safePath(asset.path)||!this.files.has(asset.path))throw Error(`工程资源缺失或路径无效：${asset.path}`)
    const missing=new Set(this.missingDependencies.filter(value=>!value.startsWith('plugin:')&&!value.startsWith('python:')&&!value.startsWith('backend:')))
    for(const backend of project.backends??[])if(![...this.files.keys()].some(path=>path===`backends/${backend.providerId}/manifest.json`||path.startsWith('backends/')&&path.endsWith(`/${backend.providerId}/manifest.json`)))missing.add(`backend:${backend.providerId}`)
    for(const connection of project.connections)if(![...this.files.keys()].some(path=>path===`plugins/${connection.protocolId}/manifest.json`||path.startsWith('plugins/')&&path.endsWith(`/${connection.protocolId}/manifest.json`)))missing.add(`plugin:${connection.protocolId}`)
    if(project.flows.some(flow=>flow.nodes.some(node=>node.kind==='python'))){if(!project.python)missing.add('python:runtime-not-configured');else if(![...this.files.keys()].some(path=>path.startsWith('python/')&&(path===`python/${project.python!.runtimeId}/runtime-manifest.json`||path.endsWith(`/${project.python!.runtimeId}/runtime-manifest.json`)||path.endsWith(`/projects/${project.id}/runtime-manifest.json`))))missing.add(`python:${project.python.runtimeId}`)}
    this.missingDependencies=[...missing].sort()
    const files:Record<string,PackageFile>={}
    for(const [path,bytes]of this.files){if(!safePath(path)||path==='manifest.json')throw Error(`工程资源路径无效：${path}`);files[path]={sha256:await digest(bytes),size:bytes.length,mode:this.metadata[path]?.mode??'file',unixMode:this.metadata[path]?.unixMode??0o644}}
    const manifest:PackageManifest={format:'hmi-project',formatVersion:1,projectSchemaVersion:project.schemaVersion,files,missingDependencies:this.missingDependencies}
    const writer=new ZipWriter(new Uint8ArrayWriter())
    for(const [path,descriptor]of Object.entries(files))await writer.add(path,new Uint8ArrayReader(this.files.get(path)!),{level:6,externalFileAttributes:((descriptor.unixMode??0o644)|0o100000)<<16})
    await writer.add('manifest.json',new Uint8ArrayReader(encoder.encode(JSON.stringify(manifest,null,2))))
    const bytes=await writer.close();await this.persistResources();return new Blob([new Uint8Array(bytes).buffer],{type:'application/vnd.hmi.project+zip'})
  }
  async addAsset(file:File,project:Project){await this.ensureResources(project);const extension=file.name.split('.').at(-1)?.toLowerCase()??'bin',id=`asset_${crypto.randomUUID().replaceAll('-','')}`,path=`assets/${id}.${extension}`;this.files.set(path,new Uint8Array(await file.arrayBuffer()));await this.persistResources();return{id,path}}
  resourceUrls(project:Project){return Object.fromEntries((project.assets??[]).map(asset=>[asset.id,this.files.has(asset.path)?URL.createObjectURL(new Blob([new Uint8Array(this.files.get(asset.path)!).buffer])):'']))}
}
export const browserProjectPackage=new BrowserProjectPackage()
export async function downloadProjectPackage(project:Project){const blob=await browserProjectPackage.save(project),url=URL.createObjectURL(blob),link=document.createElement('a');link.href=url;link.download=`${project.name}.hmiproj`;link.click();setTimeout(()=>URL.revokeObjectURL(url),1000)}

import { shallowRef } from 'vue'
import { clone, type Widget, type FlowNode, type FlowEdge } from './model'
export type DesignerClipboard = {type:'widgets';projectId:string;widgets:Widget[]} | {type:'nodes';projectId:string;nodes:FlowNode[];edges:FlowEdge[]}
export const designerClipboard=shallowRef<DesignerClipboard>()
const channel=typeof BroadcastChannel==='undefined'?undefined:new BroadcastChannel('hmi-studio-clipboard')
if(channel)channel.onmessage=event=>{if(event.data?.type==='clipboard')designerClipboard.value=event.data.value;else if(event.data?.type==='request'&&designerClipboard.value)channel.postMessage({type:'clipboard',value:designerClipboard.value})}
channel?.postMessage({type:'request'})
export function writeDesignerClipboard(value:DesignerClipboard){designerClipboard.value=clone(value);channel?.postMessage({type:'clipboard',value:clone(value)})}

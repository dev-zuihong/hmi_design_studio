import { describe, expect, it } from 'vitest'
import { byteRangeMask, memorySegments, rowCondition, tableRowKey, validByteRange, validateText } from './widget-data'
import { resolveRuntimePath } from './widgets'

describe('generic device UI data contracts',()=>{
  it('represents full and high-bit 32-byte masks without signed bitwise overflow',()=>{
    expect(byteRangeMask({startByte:0,byteCount:32},32)).toBe('FFFFFFFF')
    expect(byteRangeMask({startByte:31,byteCount:1},32)).toBe('80000000')
    expect(byteRangeMask({startByte:0,byteCount:0},32)).toBe('00000000')
    expect(validByteRange({startByte:0,byteCount:0},4)).toBe(false)
    expect(validByteRange({startByte:3,byteCount:2},4)).toBe(false)
    expect(validByteRange({startByte:1.5,byteCount:1},4)).toBe(false)
    expect(validByteRange({startByte:0,byteCount:33},33)).toBe(false)
  })
  it('uses half-open memory ranges, identifies real overlaps and preserves the uint32 end boundary',()=>{
    const result=memorySegments(0xffff0000,0x10000,[
      {id:'a',start:0xffff0000,size:0x8000},
      {id:'b',start:0xffff8000,size:0x8000},
      {id:'disabled',start:0xffff0000,size:0x10000,enabled:false},
    ])
    expect(result).toHaveLength(2)
    expect(result.every(item=>item.valid&&!item.overlap&&!item.outOfRange)).toBe(true)
    expect(result[1]).toMatchObject({offset:.5,fraction:.5})
    const invalid=memorySegments(0,100,[{id:'a',start:0,size:60},{id:'b',start:50,size:70},{id:'empty',start:10,size:0},{id:'overflow',start:Number.MAX_SAFE_INTEGER,size:1}])
    expect(invalid[0]?.overlap).toBe(true)
    expect(invalid[1]).toMatchObject({overlap:true,outOfRange:true,fraction:.5})
    expect(invalid[2]?.valid).toBe(false)
    expect(invalid[3]?.valid).toBe(false)
  })
  it('keeps configured row identity and availability independent of sort order',()=>{
    const row={id:'legacy',deviceKey:'usb.device.1',canStart:false,showSettings:true}
    expect(tableRowKey(row,'deviceKey',7)).toBe('usb.device.1')
    expect(rowCondition(row,'canStart')).toBe(false)
    expect(rowCondition(row,'showSettings')).toBe(true)
    expect(rowCondition(row,'missing')).toBe(false)
    expect(rowCondition(row,undefined)).toBe(true)
  })
  it('resolves device-specific state without splitting a substituted key or reading inherited properties',()=>{
    const backend={devices:{'usb.device.1':{status:'ready'}}}
    expect(resolveRuntimePath(backend,'devices.$deviceKey.status',{deviceKey:'usb.device.1'})).toBe('ready')
    expect(resolveRuntimePath(backend,'devices.$deviceKey.status',{})).toBeUndefined()
    expect(resolveRuntimePath(backend,'constructor')).toBeUndefined()
    expect(resolveRuntimePath(Object.create({inherited:true}),'inherited')).toBeUndefined()
    expect(resolveRuntimePath(backend,'')).toBe(backend)
  })
  it('rejects incomplete bytes, invalid addresses and malformed validation patterns',()=>{
    expect(validateText('AA 00 ff',{format:'hex'})).toBeUndefined()
    expect(validateText('ABC',{format:'hex'})).toBeTruthy()
    expect(validateText('0xFFFFFFFF',{format:'address'})).toBeUndefined()
    expect(validateText('4294967295',{format:'address'})).toBeUndefined()
    expect(validateText('100000000',{format:'address'})).toBeUndefined()
    for(const invalid of ['0207F000','FFFFFFFF','4294967296','0x100000000','-1','1.5','1e3']) expect(validateText(invalid,{format:'address'})).toBeTruthy()
    expect(validateText('AABB',{maxLength:3})).toBeTruthy()
    expect(validateText('a',{pattern:'['})).toBeTruthy()
    expect(validateText('',{required:true})).toBeTruthy()
  })
})

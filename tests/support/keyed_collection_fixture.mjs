import { readFile } from 'node:fs/promises';
import { createMonoPeFixture } from './mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './il2cpp_pe_fixture.mjs';
const profiles = JSON.parse(await readFile(new URL('../fixtures/mono-pe-profiles.json', import.meta.url)));
// Independent runtime ABI facts; profile JSON selects only the module identity.
const layouts = {
    V1Cattrs: {32:[0x34,0x38,0x24,0x78,0x68,null,null],64:[0x50,0x58,0x30,0xb0,0x9c,null,null]},
    V2: {32:[0x2c,0x30,0x20,0x60,0xa4,0x1e,0x94],64:[0x48,0x50,0x30,0x98,0x100,0x2a,0xf0]},
    V3: {32:[0x2c,0x30,0x20,0x60,0x9c,0xf,0x8c],64:[0x48,0x50,0x30,0x98,0x100,0x1b,0xf0]},
};
export function createKeyedCollectionFixture({family='V2', width=64, dictionary=true, parallel=false, reversed=false, renamed=false, inline=false}={}) {
    const mono = family !== 'il2cpp', wide = width === 64, bytes = width / 8, header = 2 * bytes;
    const fixture = mono ? createMonoPeFixture(profiles.builds.find(p=>p.width===width && p.version===family))
        : createIl2cppPeFixture({width, version:[2022,3,0,37029]});
    const memory = fixture.memory;
    const write = (at, data) => data.forEach((byte,i)=>memory.set(at+BigInt(i),byte));
    const number = (at, size, value) => {
        const data = new Uint8Array(size), view = new DataView(data.buffer);
        if(size===8) view.setBigUint64(0,BigInt.asUintN(64,BigInt(value)),true);
        else if(size===4) view.setUint32(0,Number(value)>>>0,true);
        else if(size===2) view.setUint16(0,Number(value),true);
        else view.setUint8(0,Number(value));
        write(at,data);
    };
    const ptr = (at,value)=>number(at,bytes,value);
    const text = (at,value)=> {const data=new Uint8Array(256);data.set(new TextEncoder().encode(value));write(at,data);};
    const [nameOffset,namespaceOffset,parentOffset,fieldsOffset,countOffset,kindOffset,genericOffset] = mono ? layouts[family][width]
        : wide ? [0x10,0x18,0x58,0x80,0x124] : [8,0xc,0x2c,0x40,0xac];
    const root=0x30000n, owner=0x31000n, definition=0x32000n, entry=0x33000n, entryDefinition=0x34000n;
    const object=0x70000n,vtable=0x71000n;
    const vectorType=0x50000n,elementType=0x50100n,intType=0x50200n,hashType=0x50300n,valueType=0x50400n,keyType=0x50500n;
    for(const klass of [root,owner,definition,entry,entryDefinition])for(let i=0n;i<0x200n;i++)memory.set(klass+i,0);
    const named=(klass,at,name,namespace)=>{
        ptr(klass+BigInt(nameOffset),at);text(at,name);
        ptr(klass+BigInt(namespaceOffset),at+256n);text(at+256n,namespace);
    };
    named(root,0x40000n,'DerivedCollection','Game');
    named(owner,0x40200n,dictionary?'Dictionary`2':'HashSet`1','System.Collections.Generic');
    ptr(root+BigInt(parentOffset),owner);
    number(vectorType+BigInt(bytes+2),1,0x1d);ptr(vectorType,mono?entry:elementType);
    number(elementType+BigInt(bytes+2),1,0x15);ptr(elementType,0x36000n);ptr(0x36000n+BigInt(3*bytes),entry);
    number(intType+BigInt(bytes+2),1,0x08);number(hashType+BigInt(bytes+2),1,0x09);
    number(valueType+BigInt(bytes+2),1,inline?0x11:0x0e);number(keyType+BigInt(bytes+2),1,inline?0x05:0x0e);
    const keyBytes=inline?1:bytes, valueBytes=inline?16:bytes;
    const names=parallel ? dictionary?['table','linkSlots','keySlots','valueSlots','touchedSlots','count']:['table','links','slots','touched','count']
        : dictionary?['_buckets','_entries','_count','_freeCount']:['_buckets','_slots','_count','_lastIndex'];
    if(renamed&&!parallel)names.forEach((name,i)=>names[i]=dictionary?name.slice(1):`m${name}`);
    const outer=names.map((name,i)=>[name,header+i*bytes,i>=names.length-2?intType:vectorType]);
    const hash=reversed?4:0, next=reversed?0:4, key=8, value=8+(dictionary?keyBytes:0);
    const members=parallel?[['HashCode',header+hash,hashType],['Next',header+next,intType]]
        : [['hashCode',header+hash,hashType],['next',header+next,intType],...(dictionary?[['key',header+key,keyType]]:[]),['value',header+value,valueType]];
    const stride=parallel?8:value+valueBytes;
    let nameIndex=0;
    const fields=(klass,counted,at,entries)=>{
        ptr(klass+BigInt(fieldsOffset),at);number(counted+BigInt(countOffset),mono?4:2,entries.length);
        if(mono&&kindOffset!==null){
            number(klass+BigInt(kindOffset),1,3);
            const descriptor=klass===owner?0x35000n:0x39000n;
            ptr(klass+BigInt(genericOffset),descriptor);ptr(descriptor,counted);number(klass+BigInt(countOffset),4,0x7fffffff);
        }
        entries.forEach(([name,offset,type],i)=>{
            const field=at+BigInt(i*(wide?32:mono?16:20)),nameAddress=0x44000n+BigInt(nameIndex++*256);
            ptr(field+BigInt(mono?bytes:0),nameAddress);text(nameAddress,name);ptr(field+BigInt(mono?0:bytes),type);
            number(field+BigInt(wide?0x18:0xc),4,offset);
        });
    };
    fields(owner,mono&&kindOffset!==null?definition:owner,0x37000n,outer);
    fields(entry,mono&&kindOffset!==null?entryDefinition:entry,0x38000n,members);
    number(entry+BigInt(mono?(wide?0x1c:0x10):(wide?0xf8:0x80)),4,stride+header);
    ptr(object,mono?vtable:root);ptr(vtable,root);
    number(0x60000n,8,object);number(0x60008n,1,dictionary?0:1);number(0x60009n,1,0);
    number(0x6000an,1,family==='V1Cattrs'?1:family==='V3'?3:2);
    return {...fixture,number,ptr,object,vtable,root,owner,width,bytes,outer,stride,hash,next,key,value,keyBytes,valueBytes};
}
